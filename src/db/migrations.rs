use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::catalog::split_legacy_location;
use super::playlists::ensure_playlist_tracks_allow_duplicates;
use super::settings::ensure_key_bindings_allow_duplicates;

pub(super) const SCHEMA_VERSION: i64 = 6;

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    migrate_to_version(conn, SCHEMA_VERSION)
}

fn migrate_to_version(conn: &Connection, target_version: i64) -> Result<()> {
    let foreign_keys_enabled: bool =
        conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if foreign_keys_enabled {
        conn.pragma_update(None, "foreign_keys", "OFF")?;
    }

    let migration_result = migrate_in_transaction(conn, target_version);
    let restore_result = if foreign_keys_enabled {
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(Into::into)
    } else {
        Ok(())
    };

    migration_result?;
    restore_result
}

fn migrate_in_transaction(conn: &Connection, target_version: i64) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let version = user_version(&tx)?;
    if version < 0 {
        bail!("database schema version {version} is invalid");
    }
    if version > SCHEMA_VERSION {
        bail!(
            "database schema version {version} is newer than this GMUS build supports ({SCHEMA_VERSION})"
        );
    }
    if target_version > SCHEMA_VERSION || target_version < version {
        bail!("cannot migrate database from schema version {version} to {target_version}");
    }

    for next_version in (version + 1)..=target_version {
        match next_version {
            1 => migrate_v1(&tx)?,
            2 => migrate_v2(&tx)?,
            3 => migrate_v3(&tx)?,
            4 => migrate_v4(&tx)?,
            5 => migrate_v5(&tx)?,
            6 => migrate_v6(&tx)?,
            _ => unreachable!("schema migration is defined"),
        }
        tx.pragma_update(None, "user_version", next_version)?;
    }
    ensure_foreign_keys_valid(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn user_version(conn: &Connection) -> Result<i64> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(Into::into)
}

fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS media_items (
            id              INTEGER PRIMARY KEY,
            fingerprint     TEXT NOT NULL UNIQUE,
            title           TEXT,
            artist          TEXT,
            album           TEXT,
            album_artist    TEXT,
            album_year      INTEGER,
            release_date    TEXT,
            composer        TEXT,
            genre           TEXT,
            cover_path      TEXT,
            track_number    INTEGER,
            track_total     INTEGER,
            disc_number     INTEGER,
            disc_total      INTEGER,
            duration_ms     INTEGER,
            compilation     INTEGER NOT NULL DEFAULT 0,
            first_seen_at   INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS locations (
            id              INTEGER PRIMARY KEY,
            media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
            path            TEXT NOT NULL UNIQUE,
            file_size       INTEGER,
            modified_at     INTEGER,
            seen_at         INTEGER NOT NULL,
            missing         INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS play_events (
            id              INTEGER PRIMARY KEY,
            media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
            location_id     INTEGER REFERENCES locations(id) ON DELETE SET NULL,
            played_at       INTEGER NOT NULL,
            duration_ms     INTEGER NOT NULL DEFAULT 0,
            completed       INTEGER NOT NULL DEFAULT 0,
            source          TEXT NOT NULL DEFAULT 'local'
        );

        CREATE TABLE IF NOT EXISTS media_stats (
            media_item_id   INTEGER PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
            play_count      INTEGER NOT NULL DEFAULT 0,
            last_played_at  INTEGER,
            total_play_ms   INTEGER NOT NULL DEFAULT 0,
            skip_count      INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS library_roots (
            id              INTEGER PRIMARY KEY,
            path            TEXT NOT NULL UNIQUE,
            active          INTEGER NOT NULL DEFAULT 1,
            added_at        INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL,
            last_scanned_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS playlists (
            id              INTEGER PRIMARY KEY,
            name            TEXT NOT NULL UNIQUE COLLATE NOCASE,
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS playlist_tracks (
            id              INTEGER PRIMARY KEY,
            playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
            position        INTEGER NOT NULL,
            added_at        INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS app_browser_selection (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            tree_kind       TEXT NOT NULL,
            artist          TEXT,
            album           TEXT,
            playlist_id     INTEGER,
            media_item_id   INTEGER,
            updated_at      INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS app_key_bindings (
            action          TEXT NOT NULL,
            key             TEXT NOT NULL,
            updated_at      INTEGER NOT NULL,
            PRIMARY KEY (action, key)
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key             TEXT PRIMARY KEY,
            value           TEXT NOT NULL,
            updated_at      INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS app_filter_state (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            filter          TEXT NOT NULL,
            updated_at      INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_locations_media_item
            ON locations(media_item_id);
        CREATE INDEX IF NOT EXISTS idx_play_events_media_item
            ON play_events(media_item_id, played_at);
        CREATE INDEX IF NOT EXISTS idx_media_items_artist_album
            ON media_items(album_artist, artist, album);
        CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist_position
            ON playlist_tracks(playlist_id, position);
        "#,
    )?;
    repair_v1_schema(conn)
}

fn migrate_v2(conn: &Connection) -> Result<()> {
    migrate_v1(conn)?;
    conn.execute_batch(
        r#"
        CREATE TABLE media_items_v2 (
            id              INTEGER PRIMARY KEY,
            duplicate_key   TEXT NOT NULL,
            title           TEXT,
            artist          TEXT,
            album           TEXT,
            album_artist    TEXT,
            album_year      INTEGER,
            release_date    TEXT,
            composer        TEXT,
            genre           TEXT,
            cover_path      TEXT,
            track_number    INTEGER,
            track_total     INTEGER,
            disc_number     INTEGER,
            disc_total      INTEGER,
            duration_ms     INTEGER,
            compilation     INTEGER NOT NULL DEFAULT 0,
            first_seen_at   INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL
        );

        INSERT INTO media_items_v2 (
            id, duplicate_key, title, artist, album, album_artist, album_year,
            release_date, composer, genre, cover_path, track_number, track_total,
            disc_number, disc_total, duration_ms, compilation, first_seen_at, updated_at
        )
        SELECT
            id, fingerprint, title, artist, album, album_artist, album_year,
            release_date, composer, genre, cover_path, track_number, track_total,
            disc_number, disc_total, duration_ms, compilation, first_seen_at, updated_at
        FROM media_items;

        DROP TABLE media_items;
        ALTER TABLE media_items_v2 RENAME TO media_items;

        CREATE INDEX idx_media_items_duplicate_key
            ON media_items(duplicate_key);
        CREATE INDEX idx_media_items_artist_album
            ON media_items(album_artist, artist, album);
        "#,
    )?;
    split_legacy_present_locations(conn)?;
    conn.execute_batch(
        r#"
        CREATE UNIQUE INDEX idx_locations_one_present_per_media_item
            ON locations(media_item_id)
            WHERE missing = 0;
        "#,
    )?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<()> {
    ensure_column(conn, "locations", "fs_device", "INTEGER")?;
    ensure_column(conn, "locations", "fs_inode", "INTEGER")?;
    conn.execute_batch(
        r#"
        CREATE INDEX idx_locations_filesystem_identity
            ON locations(fs_device, fs_inode)
            WHERE fs_device IS NOT NULL AND fs_inode IS NOT NULL;
        "#,
    )?;
    Ok(())
}

fn migrate_v4(conn: &Connection) -> Result<()> {
    ensure_column(conn, "locations", "modified_at_ns", "INTEGER")?;
    ensure_column(conn, "locations", "scan_version", "INTEGER")?;
    Ok(())
}

fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        UPDATE locations
        SET path = 'unix:' || lower(hex(CAST(path AS BLOB)));

        UPDATE library_roots
        SET path = 'unix:' || lower(hex(CAST(path AS BLOB)));

        UPDATE media_items
        SET cover_path = 'unix:' || lower(hex(CAST(cover_path AS BLOB)))
        WHERE cover_path IS NOT NULL;
        "#,
    )?;
    Ok(())
}

fn migrate_v6(conn: &Connection) -> Result<()> {
    ensure_column(conn, "locations", "folder_art_signature", "TEXT")?;
    conn.execute("UPDATE locations SET scan_version = NULL", [])?;
    Ok(())
}

fn repair_v1_schema(conn: &Connection) -> Result<()> {
    ensure_column(conn, "media_items", "cover_path", "TEXT")?;
    ensure_column(conn, "media_items", "album_year", "INTEGER")?;
    ensure_column(conn, "media_items", "release_date", "TEXT")?;
    ensure_column(conn, "media_items", "composer", "TEXT")?;
    ensure_column(conn, "media_items", "genre", "TEXT")?;
    ensure_column(conn, "media_items", "track_total", "INTEGER")?;
    ensure_column(conn, "media_items", "disc_total", "INTEGER")?;
    ensure_column(
        conn,
        "media_items",
        "compilation",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_playlist_tracks_allow_duplicates(conn)?;
    ensure_key_bindings_allow_duplicates(conn)?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist_position
            ON playlist_tracks(playlist_id, position);
        "#,
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }

    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )?;
    Ok(())
}

fn split_legacy_present_locations(conn: &Connection) -> Result<()> {
    let media_item_ids = {
        let mut stmt = conn.prepare(
            r#"
            SELECT media_item_id
            FROM locations
            WHERE missing = 0
            GROUP BY media_item_id
            HAVING COUNT(*) > 1
            ORDER BY media_item_id
            "#,
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<i64>>>()?
    };

    for media_item_id in media_item_ids {
        let location_ids = {
            let mut stmt = conn.prepare(
                r#"
                SELECT id
                FROM locations
                WHERE media_item_id = ?1 AND missing = 0
                ORDER BY id
                "#,
            )?;
            let rows = stmt.query_map(params![media_item_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<i64>>>()?
        };
        for location_id in location_ids.into_iter().skip(1) {
            split_legacy_location(conn, location_id, media_item_id)?;
        }
    }
    Ok(())
}

fn ensure_foreign_keys_valid(conn: &Connection) -> Result<()> {
    let violation = conn
        .query_row("PRAGMA foreign_key_check", [], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .optional()?;
    if let Some((table, row_id, parent)) = violation {
        bail!("migration introduced foreign key violation in {table} row {row_id} referencing {parent}");
    }
    Ok(())
}
