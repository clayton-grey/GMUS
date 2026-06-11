use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::media::TrackMetadata;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy)]
pub struct StoredTrack {
    pub media_item_id: i64,
    pub location_id: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct DbStats {
    pub media_items: i64,
    pub locations: i64,
    pub play_events: i64,
    pub completed_plays: i64,
}

#[derive(Debug, Clone)]
pub struct LibraryRoot {
    pub path: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistTrack {
    pub id: i64,
    pub media_item_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBrowserSelection {
    pub tree_kind: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub playlist_id: Option<i64>,
    pub media_item_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedKeyBinding {
    pub action: String,
    pub key: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SavedPaneLayout {
    pub library_percent_offset: i16,
    pub info_height_offset: i16,
}

#[derive(Debug, Clone)]
pub struct LibraryTrack {
    pub media_item_id: i64,
    pub location_id: i64,
    pub path: String,
    pub library_root: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_year: Option<i64>,
    pub release_date: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub cover_path: Option<String>,
    pub track_number: Option<i64>,
    pub track_total: Option<i64>,
    pub disc_number: Option<i64>,
    pub disc_total: Option<i64>,
    pub duration_ms: Option<i64>,
    pub compilation: bool,
    pub play_count: i64,
}

impl LibraryTrack {
    pub fn display_title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.path)
    }

    pub fn display_artist(&self) -> &str {
        self.artist.as_deref().unwrap_or("")
    }

    pub fn display_album(&self) -> &str {
        self.album.as_deref().unwrap_or("")
    }

    pub fn tree_artist(&self) -> &str {
        self.album_artist
            .as_deref()
            .or(self.artist.as_deref())
            .unwrap_or("<Unknown Artist>")
    }

    pub fn tree_album(&self) -> &str {
        self.album.as_deref().unwrap_or("<Unknown Album>")
    }
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub(crate) fn open_in_memory_for_tests() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version = user_version(conn)?;
    if version > SCHEMA_VERSION {
        anyhow::bail!(
            "database schema version {version} is newer than this GMUS build supports ({SCHEMA_VERSION})"
        );
    }

    let tx = conn.unchecked_transaction()?;
    migrate_v1(&tx)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn user_version(conn: &Connection) -> Result<i64> {
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

pub fn upsert_library_root(conn: &Connection, path: &Path) -> Result<()> {
    let now = now_unix();
    let path = path.to_string_lossy();
    conn.execute(
        r#"
        INSERT INTO library_roots (path, active, added_at, updated_at)
        VALUES (?1, 1, ?2, ?2)
        ON CONFLICT(path) DO UPDATE SET
            active = 1,
            updated_at = excluded.updated_at
        "#,
        params![path, now],
    )?;
    Ok(())
}

pub fn mark_library_root_scanned(conn: &Connection, path: &Path) -> Result<()> {
    let now = now_unix();
    let path = path.to_string_lossy();
    conn.execute(
        "UPDATE library_roots SET updated_at = ?1, last_scanned_at = ?1 WHERE path = ?2",
        params![now, path],
    )?;
    Ok(())
}

pub fn deactivate_library_root(conn: &Connection, path: &Path) -> Result<bool> {
    set_library_root_active(conn, path, false)
}

pub fn set_library_root_active(conn: &Connection, path: &Path, active: bool) -> Result<bool> {
    let now = now_unix();
    let path = path.to_string_lossy();
    let changed = conn.execute(
        "UPDATE library_roots SET active = ?1, updated_at = ?2 WHERE path = ?3",
        params![i64::from(active), now, path],
    )?;
    Ok(changed > 0)
}

pub fn library_roots(conn: &Connection) -> Result<Vec<LibraryRoot>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT path, active
        FROM library_roots
        ORDER BY active DESC, path
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LibraryRoot {
            path: row.get(0)?,
            active: row.get::<_, i64>(1)? != 0,
        })
    })?;

    let mut roots = Vec::new();
    for row in rows {
        roots.push(row?);
    }
    Ok(roots)
}

pub fn active_library_roots(conn: &Connection) -> Result<Vec<LibraryRoot>> {
    Ok(library_roots(conn)?
        .into_iter()
        .filter(|root| root.active)
        .collect())
}

pub fn playlists(conn: &Connection) -> Result<Vec<Playlist>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, name
        FROM playlists
        ORDER BY name COLLATE NOCASE
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Playlist {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;

    let mut playlists = Vec::new();
    for row in rows {
        playlists.push(row?);
    }
    Ok(playlists)
}

pub fn playlist_by_name(conn: &Connection, name: &str) -> Result<Option<Playlist>> {
    conn.query_row(
        r#"
        SELECT id, name
        FROM playlists
        WHERE name = ?1 COLLATE NOCASE
        "#,
        params![name.trim()],
        |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn create_playlist(conn: &Connection, name: &str) -> Result<Playlist> {
    let name = normalize_playlist_name(name);
    let now = now_unix();
    conn.execute(
        r#"
        INSERT INTO playlists (name, created_at, updated_at)
        VALUES (?1, ?2, ?2)
        ON CONFLICT(name) DO UPDATE SET updated_at = playlists.updated_at
        "#,
        params![name, now],
    )?;
    playlist_by_name(conn, &name)?.ok_or_else(|| anyhow::anyhow!("playlist not found: {name}"))
}

pub fn delete_playlist(conn: &Connection, name: &str) -> Result<bool> {
    let changed = conn.execute(
        "DELETE FROM playlists WHERE name = ?1 COLLATE NOCASE",
        params![name.trim()],
    )?;
    Ok(changed > 0)
}

#[cfg(test)]
pub fn playlist_track_ids(conn: &Connection, playlist_id: i64) -> Result<Vec<i64>> {
    Ok(playlist_tracks(conn, playlist_id)?
        .into_iter()
        .map(|track| track.media_item_id)
        .collect())
}

pub fn playlist_tracks(conn: &Connection, playlist_id: i64) -> Result<Vec<PlaylistTrack>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id, media_item_id
        FROM playlist_tracks
        WHERE playlist_id = ?1
        ORDER BY position, added_at, id
        "#,
    )?;
    let rows = stmt.query_map(params![playlist_id], |row| {
        Ok(PlaylistTrack {
            id: row.get(0)?,
            media_item_id: row.get(1)?,
        })
    })?;

    let mut tracks = Vec::new();
    for row in rows {
        tracks.push(row?);
    }
    Ok(tracks)
}

pub fn browser_selection(conn: &Connection) -> Result<Option<SavedBrowserSelection>> {
    conn.query_row(
        r#"
        SELECT tree_kind, artist, album, playlist_id, media_item_id
        FROM app_browser_selection
        WHERE id = 1
        "#,
        [],
        |row| {
            Ok(SavedBrowserSelection {
                tree_kind: row.get(0)?,
                artist: row.get(1)?,
                album: row.get(2)?,
                playlist_id: row.get(3)?,
                media_item_id: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn save_browser_selection(conn: &Connection, selection: &SavedBrowserSelection) -> Result<()> {
    let now = now_unix();
    conn.execute(
        r#"
        INSERT INTO app_browser_selection (
            id, tree_kind, artist, album, playlist_id, media_item_id, updated_at
        ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(id) DO UPDATE SET
            tree_kind = excluded.tree_kind,
            artist = excluded.artist,
            album = excluded.album,
            playlist_id = excluded.playlist_id,
            media_item_id = excluded.media_item_id,
            updated_at = excluded.updated_at
        "#,
        params![
            selection.tree_kind.as_str(),
            selection.artist.as_deref(),
            selection.album.as_deref(),
            selection.playlist_id,
            selection.media_item_id,
            now
        ],
    )?;
    Ok(())
}

pub fn key_bindings(conn: &Connection) -> Result<Vec<SavedKeyBinding>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT action, key
        FROM app_key_bindings
        ORDER BY action, key
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SavedKeyBinding {
            action: row.get(0)?,
            key: row.get(1)?,
        })
    })?;

    let mut bindings = Vec::new();
    for row in rows {
        bindings.push(row?);
    }
    Ok(bindings)
}

pub fn save_key_binding(conn: &Connection, binding: &SavedKeyBinding) -> Result<()> {
    let now = now_unix();
    conn.execute(
        r#"
        INSERT INTO app_key_bindings (action, key, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(action, key) DO UPDATE SET
            updated_at = excluded.updated_at
        "#,
        params![binding.action.as_str(), binding.key.as_str(), now],
    )?;
    Ok(())
}

pub fn delete_key_binding(conn: &Connection, action: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM app_key_bindings WHERE action = ?1",
        params![action],
    )?;
    Ok(())
}

pub fn delete_key_bindings(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM app_key_bindings", [])?;
    Ok(())
}

pub fn delete_key_binding_key(conn: &Connection, action: &str, key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM app_key_bindings WHERE action = ?1 AND key = ?2",
        params![action, key],
    )?;
    Ok(())
}

pub fn restore_filter_enabled(conn: &Connection) -> Result<bool> {
    app_setting_bool(conn, "restore-filter", true)
}

pub fn save_restore_filter_enabled(conn: &Connection, enabled: bool) -> Result<()> {
    save_app_setting_bool(conn, "restore-filter", enabled)
}

pub fn restore_track_enabled(conn: &Connection) -> Result<bool> {
    app_setting_bool(conn, "restore-track", true)
}

pub fn save_restore_track_enabled(conn: &Connection, enabled: bool) -> Result<()> {
    save_app_setting_bool(conn, "restore-track", enabled)
}

pub fn column_layout_width(conn: &Connection, default: u16) -> Result<u16> {
    Ok(app_setting_value(conn, "column-layout-width")?
        .and_then(|value| value.trim().parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default))
}

pub fn save_column_layout_width(conn: &Connection, width: u16) -> Result<()> {
    save_app_setting(conn, "column-layout-width", &width.to_string())
}

pub fn pane_layout(conn: &Connection) -> Result<SavedPaneLayout> {
    Ok(SavedPaneLayout {
        library_percent_offset: app_setting_i16(conn, "library-pane-percent-offset", 0)?,
        info_height_offset: app_setting_i16(conn, "info-pane-height-offset", 0)?,
    })
}

pub fn save_pane_layout(conn: &Connection, layout: SavedPaneLayout) -> Result<()> {
    save_app_setting_i16(
        conn,
        "library-pane-percent-offset",
        layout.library_percent_offset,
    )?;
    save_app_setting_i16(conn, "info-pane-height-offset", layout.info_height_offset)?;
    Ok(())
}

pub fn saved_filter(conn: &Connection) -> Result<Option<String>> {
    conn.query_row(
        "SELECT filter FROM app_filter_state WHERE id = 1",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

pub fn save_filter(conn: &Connection, filter: &str) -> Result<()> {
    let now = now_unix();
    conn.execute(
        r#"
        INSERT INTO app_filter_state (id, filter, updated_at)
        VALUES (1, ?1, ?2)
        ON CONFLICT(id) DO UPDATE SET
            filter = excluded.filter,
            updated_at = excluded.updated_at
        "#,
        params![filter, now],
    )?;
    Ok(())
}

fn app_setting_value(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn app_setting_bool(conn: &Connection, key: &str, default: bool) -> Result<bool> {
    let value = app_setting_value(conn, key)?;
    let Some(value) = value else {
        return Ok(default);
    };
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    ))
}

fn app_setting_i16(conn: &Connection, key: &str, default: i16) -> Result<i16> {
    Ok(app_setting_value(conn, key)?
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default))
}

fn save_app_setting_bool(conn: &Connection, key: &str, enabled: bool) -> Result<()> {
    let value = if enabled { "1" } else { "0" };
    save_app_setting(conn, key, value)
}

fn save_app_setting_i16(conn: &Connection, key: &str, value: i16) -> Result<()> {
    save_app_setting(conn, key, &value.to_string())
}

fn save_app_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    let now = now_unix();
    conn.execute(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at
        "#,
        params![key, value, now],
    )?;
    Ok(())
}

pub fn add_tracks_to_playlist(
    conn: &Connection,
    playlist_id: i64,
    media_item_ids: &[i64],
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut position = tx.query_row(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |row| row.get::<_, i64>(0),
    )?;
    let now = now_unix();
    let mut added = 0;
    for media_item_id in media_item_ids {
        let changed = tx.execute(
            r#"
            INSERT INTO playlist_tracks (
                playlist_id, media_item_id, position, added_at
            ) VALUES (?1, ?2, ?3, ?4)
            "#,
            params![playlist_id, media_item_id, position, now],
        )?;
        if changed > 0 {
            position += 1;
            added += 1;
        }
    }
    touch_playlist(&tx, playlist_id)?;
    tx.commit()?;
    Ok(added)
}

pub fn remove_tracks_from_playlist(
    conn: &Connection,
    playlist_id: i64,
    media_item_ids: &[i64],
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut removed = 0;
    for media_item_id in unique_media_item_ids(media_item_ids) {
        removed += tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND media_item_id = ?2",
            params![playlist_id, media_item_id],
        )?;
    }
    if removed > 0 {
        compact_playlist_positions(&tx, playlist_id)?;
        touch_playlist(&tx, playlist_id)?;
    }
    tx.commit()?;
    Ok(removed)
}

pub fn remove_latest_tracks_from_playlist(
    conn: &Connection,
    playlist_id: i64,
    media_item_ids: &[i64],
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut removed = 0;
    for media_item_id in media_item_ids {
        let entry_id = tx
            .query_row(
                r#"
                SELECT id
                FROM playlist_tracks
                WHERE playlist_id = ?1 AND media_item_id = ?2
                ORDER BY position DESC, added_at DESC, id DESC
                LIMIT 1
                "#,
                params![playlist_id, media_item_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(entry_id) = entry_id {
            removed += tx.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND id = ?2",
                params![playlist_id, entry_id],
            )?;
        }
    }
    if removed > 0 {
        compact_playlist_positions(&tx, playlist_id)?;
        touch_playlist(&tx, playlist_id)?;
    }
    tx.commit()?;
    Ok(removed)
}

pub fn remove_playlist_track_entries(
    conn: &Connection,
    playlist_id: i64,
    playlist_track_ids: &[i64],
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut removed = 0;
    for playlist_track_id in playlist_track_ids {
        removed += tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND id = ?2",
            params![playlist_id, playlist_track_id],
        )?;
    }
    if removed > 0 {
        compact_playlist_positions(&tx, playlist_id)?;
        touch_playlist(&tx, playlist_id)?;
    }
    tx.commit()?;
    Ok(removed)
}

pub fn clear_playlist(conn: &Connection, playlist_id: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let removed = tx.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
    )?;
    if removed > 0 {
        touch_playlist(&tx, playlist_id)?;
    }
    tx.commit()?;
    Ok(removed)
}

fn normalize_playlist_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "Default".to_string()
    } else {
        name.to_string()
    }
}

fn unique_media_item_ids(media_item_ids: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    media_item_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect()
}

fn ensure_playlist_tracks_allow_duplicates(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(playlist_tracks)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_entry_id = false;
    for row in rows {
        if row? == "id" {
            has_entry_id = true;
            break;
        }
    }
    if has_entry_id {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_playlist_tracks_playlist_position;
        ALTER TABLE playlist_tracks RENAME TO playlist_tracks_old;

        CREATE TABLE playlist_tracks (
            id              INTEGER PRIMARY KEY,
            playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
            media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
            position        INTEGER NOT NULL,
            added_at        INTEGER NOT NULL
        );

        INSERT INTO playlist_tracks (playlist_id, media_item_id, position, added_at)
        SELECT playlist_id, media_item_id, position, added_at
        FROM playlist_tracks_old
        ORDER BY playlist_id, position, added_at, media_item_id;

        DROP TABLE playlist_tracks_old;
        "#,
    )?;
    Ok(())
}

fn ensure_key_bindings_allow_duplicates(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(app_key_bindings)")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
    })?;
    let mut action_pk = 0;
    let mut key_pk = 0;
    for row in rows {
        let (column, pk) = row?;
        match column.as_str() {
            "action" => action_pk = pk,
            "key" => key_pk = pk,
            _ => {}
        }
    }
    if action_pk == 1 && key_pk == 2 {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        ALTER TABLE app_key_bindings RENAME TO app_key_bindings_old;

        CREATE TABLE app_key_bindings (
            action          TEXT NOT NULL,
            key             TEXT NOT NULL,
            updated_at      INTEGER NOT NULL,
            PRIMARY KEY (action, key)
        );

        INSERT OR IGNORE INTO app_key_bindings (action, key, updated_at)
        SELECT action, key, updated_at
        FROM app_key_bindings_old;

        DROP TABLE app_key_bindings_old;
        "#,
    )?;
    Ok(())
}

fn touch_playlist(conn: &Connection, playlist_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
        params![now_unix(), playlist_id],
    )?;
    Ok(())
}

fn compact_playlist_positions(conn: &Connection, playlist_id: i64) -> Result<()> {
    let ids = playlist_track_entry_ids(conn, playlist_id)?;
    for (position, playlist_track_id) in ids.into_iter().enumerate() {
        conn.execute(
            "UPDATE playlist_tracks SET position = ?1 WHERE playlist_id = ?2 AND id = ?3",
            params![position as i64, playlist_id, playlist_track_id],
        )?;
    }
    Ok(())
}

fn playlist_track_entry_ids(conn: &Connection, playlist_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT id
        FROM playlist_tracks
        WHERE playlist_id = ?1
        ORDER BY position, added_at, id
        "#,
    )?;
    let rows = stmt.query_map(params![playlist_id], |row| row.get(0))?;

    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

pub fn upsert_track(conn: &Connection, track: &TrackMetadata) -> Result<StoredTrack> {
    let tx = conn.unchecked_transaction()?;
    let now = now_unix();
    let fingerprint = track.fingerprint();

    tx.execute(
        r#"
        INSERT INTO media_items (
            fingerprint, title, artist, album, album_artist, album_year, release_date,
            composer, genre, track_number, track_total, disc_number, disc_total,
            duration_ms, compilation, first_seen_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
        ON CONFLICT(fingerprint) DO UPDATE SET
            title = excluded.title,
            artist = excluded.artist,
            album = excluded.album,
            album_artist = excluded.album_artist,
            album_year = excluded.album_year,
            release_date = excluded.release_date,
            composer = excluded.composer,
            genre = excluded.genre,
            track_number = excluded.track_number,
            track_total = excluded.track_total,
            disc_number = excluded.disc_number,
            disc_total = excluded.disc_total,
            duration_ms = excluded.duration_ms,
            compilation = excluded.compilation,
            updated_at = excluded.updated_at
        "#,
        params![
            fingerprint,
            track.title,
            track.artist,
            track.album,
            track.album_artist,
            track.album_year,
            track.release_date,
            track.composer,
            track.genre,
            track.track_number,
            track.track_total,
            track.disc_number,
            track.disc_total,
            track.duration_ms,
            i64::from(track.compilation),
            now
        ],
    )?;

    let media_item_id: i64 = tx.query_row(
        "SELECT id FROM media_items WHERE fingerprint = ?1",
        params![fingerprint],
        |row| row.get(0),
    )?;

    let path = track.path.to_string_lossy();
    if let Some(existing_media_item_id) = media_item_id_for_path(&tx, &path)? {
        merge_media_item(&tx, media_item_id, existing_media_item_id)?;
    }

    tx.execute(
        r#"
        INSERT INTO locations (
            media_item_id, path, file_size, modified_at, seen_at, missing
        ) VALUES (?1, ?2, ?3, ?4, ?5, 0)
        ON CONFLICT(path) DO UPDATE SET
            media_item_id = excluded.media_item_id,
            file_size = excluded.file_size,
            modified_at = excluded.modified_at,
            seen_at = excluded.seen_at,
            missing = 0
        "#,
        params![media_item_id, path, track.file_size, track.modified_at, now],
    )?;

    let location_id: i64 = tx.query_row(
        "SELECT id FROM locations WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )?;

    tx.execute(
        r#"
        INSERT INTO media_stats (media_item_id)
        VALUES (?1)
        ON CONFLICT(media_item_id) DO NOTHING
        "#,
        params![media_item_id],
    )?;

    tx.commit()?;
    Ok(StoredTrack {
        media_item_id,
        location_id,
    })
}

fn media_item_id_for_path(conn: &Connection, path: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT media_item_id FROM locations WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
pub fn mark_locations_missing_under_root(conn: &Connection, root: &Path) -> Result<usize> {
    mark_locations_missing_under_root_except(conn, root, &[])
}

pub fn mark_locations_missing_under_root_except(
    conn: &Connection,
    root: &Path,
    seen_paths: &[PathBuf],
) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        r#"
        CREATE TEMP TABLE IF NOT EXISTS gmus_scan_seen_paths (
            path TEXT PRIMARY KEY
        );
        DELETE FROM gmus_scan_seen_paths;
        "#,
    )?;

    {
        let mut stmt =
            tx.prepare("INSERT OR IGNORE INTO gmus_scan_seen_paths (path) VALUES (?1)")?;
        for path in seen_paths {
            stmt.execute(params![path.to_string_lossy()])?;
        }
    }

    let root = root.to_string_lossy();
    let sql = format!(
        r#"
        UPDATE locations
        SET missing = 1
        WHERE missing = 0
            AND {}
            AND NOT EXISTS (
                SELECT 1
                FROM gmus_scan_seen_paths
                WHERE gmus_scan_seen_paths.path = locations.path
            )
        "#,
        path_matches_root_sql("path", "?1")
    );
    let marked = tx.execute(&sql, params![root])?;
    tx.execute("DELETE FROM gmus_scan_seen_paths", [])?;
    tx.commit()?;
    Ok(marked)
}

pub fn merge_similar_media_items(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut groups: HashMap<String, Vec<MergeCandidate>> = HashMap::new();
    {
        let sql = format!(
            r#"
            SELECT
                id,
                title,
                artist,
                album,
                album_artist,
                track_number,
                disc_number,
                duration_ms,
                updated_at,
                COALESCE(
                    (
                        SELECT library_roots.path
                        FROM locations
                        JOIN library_roots
                            ON {}
                        WHERE locations.media_item_id = media_items.id
                        ORDER BY locations.missing ASC, length(library_roots.path) DESC
                        LIMIT 1
                    ),
                    ''
                ),
                (
                    SELECT COUNT(*)
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                        AND locations.missing = 0
                )
            FROM media_items
            ORDER BY id
            "#,
            path_matches_root_sql("locations.path", "library_roots.path")
        );
        let mut stmt = tx.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(MergeCandidate {
                id: row.get(0)?,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                album_artist: row.get(4)?,
                track_number: row.get(5)?,
                disc_number: row.get(6)?,
                duration_ms: row.get(7)?,
                updated_at: row.get(8)?,
                library_root: row.get(9)?,
                present_locations: row.get(10)?,
            })
        })?;

        for row in rows {
            let candidate = row?;
            if let Some(key) = candidate.similarity_key() {
                groups.entry(key).or_default().push(candidate);
            }
        }
    }

    let mut merged = 0;
    for mut candidates in groups
        .into_values()
        .filter(|candidates| candidates.len() > 1)
    {
        candidates.sort_by(|left, right| {
            right
                .present_locations
                .cmp(&left.present_locations)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| right.id.cmp(&left.id))
        });
        let canonical_id = candidates[0].id;
        for duplicate in candidates.into_iter().skip(1) {
            merge_media_item(&tx, canonical_id, duplicate.id)?;
            merged += 1;
        }
    }
    tx.commit()?;
    Ok(merged)
}

#[derive(Debug)]
struct MergeCandidate {
    id: i64,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<i64>,
    disc_number: Option<i64>,
    duration_ms: Option<i64>,
    updated_at: i64,
    library_root: String,
    present_locations: i64,
}

impl MergeCandidate {
    fn similarity_key(&self) -> Option<String> {
        let title = normalize_identity_part(self.title.as_deref())?;
        let artist =
            normalize_identity_part(self.album_artist.as_deref().or(self.artist.as_deref()))?;
        let album = normalize_identity_part(self.album.as_deref())?;
        let disc = self.disc_number.unwrap_or(0);
        let track = self.track_number.unwrap_or(0);
        let duration_bucket = self.duration_ms.unwrap_or_default().max(0) / 3_000;
        Some(format!(
            "{}|{artist}|{album}|{disc}|{track}|{title}|{duration_bucket}",
            self.library_root
        ))
    }
}

#[derive(Debug, Default)]
struct MediaStatsRow {
    play_count: i64,
    last_played_at: Option<i64>,
    total_play_ms: i64,
    skip_count: i64,
}

fn merge_media_item(conn: &Connection, canonical_id: i64, duplicate_id: i64) -> Result<()> {
    if canonical_id == duplicate_id {
        return Ok(());
    }

    conn.execute(
        r#"
        INSERT INTO media_stats (media_item_id)
        VALUES (?1)
        ON CONFLICT(media_item_id) DO NOTHING
        "#,
        params![canonical_id],
    )?;

    let duplicate_stats = media_stats_row(conn, duplicate_id)?.unwrap_or_default();
    if duplicate_stats.play_count > 0
        || duplicate_stats.total_play_ms > 0
        || duplicate_stats.skip_count > 0
        || duplicate_stats.last_played_at.is_some()
    {
        conn.execute(
            r#"
            UPDATE media_stats
            SET play_count = play_count + ?2,
                last_played_at = MAX(COALESCE(last_played_at, 0), COALESCE(?3, 0)),
                total_play_ms = total_play_ms + ?4,
                skip_count = skip_count + ?5
            WHERE media_item_id = ?1
            "#,
            params![
                canonical_id,
                duplicate_stats.play_count,
                duplicate_stats.last_played_at,
                duplicate_stats.total_play_ms,
                duplicate_stats.skip_count
            ],
        )?;
        conn.execute(
            "UPDATE media_stats SET last_played_at = NULL WHERE media_item_id = ?1 AND last_played_at = 0",
            params![canonical_id],
        )?;
    }

    conn.execute(
        "UPDATE play_events SET media_item_id = ?1 WHERE media_item_id = ?2",
        params![canonical_id, duplicate_id],
    )?;
    conn.execute(
        "UPDATE locations SET media_item_id = ?1 WHERE media_item_id = ?2",
        params![canonical_id, duplicate_id],
    )?;
    conn.execute(
        "UPDATE playlist_tracks SET media_item_id = ?1 WHERE media_item_id = ?2",
        params![canonical_id, duplicate_id],
    )?;
    conn.execute(
        "DELETE FROM media_stats WHERE media_item_id = ?1",
        params![duplicate_id],
    )?;
    conn.execute(
        "DELETE FROM media_items WHERE id = ?1",
        params![duplicate_id],
    )?;
    Ok(())
}

fn media_stats_row(conn: &Connection, media_item_id: i64) -> Result<Option<MediaStatsRow>> {
    conn.query_row(
        r#"
        SELECT play_count, last_played_at, total_play_ms, skip_count
        FROM media_stats
        WHERE media_item_id = ?1
        "#,
        params![media_item_id],
        |row| {
            Ok(MediaStatsRow {
                play_count: row.get(0)?,
                last_played_at: row.get(1)?,
                total_play_ms: row.get(2)?,
                skip_count: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn normalize_identity_part(value: Option<&str>) -> Option<String> {
    let normalized = value?
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

pub fn record_play(
    conn: &Connection,
    media_item_id: i64,
    location_id: i64,
    duration_ms: i64,
    completed: bool,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let now = now_unix();
    let completed_i64 = i64::from(completed);
    tx.execute(
        r#"
        INSERT INTO play_events (
            media_item_id, location_id, played_at, duration_ms, completed
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            media_item_id,
            location_id,
            now,
            duration_ms.max(0),
            completed_i64
        ],
    )?;

    tx.execute(
        r#"
        INSERT INTO media_stats (
            media_item_id, play_count, last_played_at, total_play_ms
        ) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(media_item_id) DO UPDATE SET
            play_count = media_stats.play_count + excluded.play_count,
            last_played_at = COALESCE(excluded.last_played_at, media_stats.last_played_at),
            total_play_ms = media_stats.total_play_ms + excluded.total_play_ms,
            skip_count = media_stats.skip_count + CASE WHEN excluded.play_count = 0 THEN 1 ELSE 0 END
        "#,
        params![
            media_item_id,
            completed_i64,
            if completed { Some(now) } else { None },
            duration_ms.max(0)
        ],
    )?;

    tx.commit()?;
    Ok(())
}

pub fn set_cover_path(conn: &Connection, media_item_id: i64, path: &Path) -> Result<()> {
    conn.execute(
        "UPDATE media_items SET cover_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![path.to_string_lossy(), now_unix(), media_item_id],
    )?;
    Ok(())
}

pub fn stats(conn: &Connection) -> Result<DbStats> {
    Ok(DbStats {
        media_items: count(conn, "media_items")?,
        locations: count(conn, "locations")?,
        play_events: count(conn, "play_events")?,
        completed_plays: conn.query_row(
            "SELECT COALESCE(SUM(completed), 0) FROM play_events",
            [],
            |row| row.get(0),
        )?,
    })
}

pub fn library_tracks(conn: &Connection) -> Result<Vec<LibraryTrack>> {
    let active_root_matches_location =
        path_matches_root_sql("locations.path", "library_roots.path");
    let sql = format!(
        r#"
        WITH visible_tracks AS (
        SELECT
            media_items.id AS media_item_id,
            locations.id AS location_id,
            locations.path AS path,
            media_items.title AS title,
            media_items.artist AS artist,
            media_items.album AS album,
            media_items.album_artist AS album_artist,
            media_items.album_year AS album_year,
            media_items.release_date AS release_date,
            media_items.composer AS composer,
            media_items.genre AS genre,
            media_items.cover_path AS cover_path,
            media_items.track_number AS track_number,
            media_items.track_total AS track_total,
            media_items.disc_number AS disc_number,
            media_items.disc_total AS disc_total,
            media_items.duration_ms AS duration_ms,
            media_items.compilation AS compilation,
            (
                SELECT library_roots.path
                FROM library_roots
                WHERE library_roots.active = 1
                    AND {active_root_matches_location}
                ORDER BY length(library_roots.path) DESC
                LIMIT 1
            ) AS library_root,
            COALESCE(media_stats.play_count, 0) AS play_count,
            COALESCE(media_items.album_artist, media_items.artist, '') AS artist_sort,
            COALESCE(media_items.album, '') AS album_sort
        FROM locations
        JOIN media_items ON media_items.id = locations.media_item_id
        LEFT JOIN media_stats ON media_stats.media_item_id = media_items.id
        WHERE locations.missing = 0
            AND (
                NOT EXISTS (SELECT 1 FROM library_roots)
                OR EXISTS (
                    SELECT 1
                    FROM library_roots
                    WHERE library_roots.active = 1
                        AND {active_root_matches_location}
                )
            )
        )
        SELECT
            media_item_id,
            location_id,
            path,
            title,
            artist,
            album,
            album_artist,
            album_year,
            release_date,
            composer,
            genre,
            cover_path,
            track_number,
            track_total,
            disc_number,
            disc_total,
            duration_ms,
            compilation,
            library_root,
            play_count
        FROM visible_tracks
        ORDER BY
            artist_sort,
            MIN(COALESCE(album_year, 9223372036854775807))
                OVER (PARTITION BY artist_sort, album_sort),
            album_sort,
            COALESCE(disc_number, 0),
            COALESCE(track_number, 0),
            COALESCE(title, path)
        "#,
    );
    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map([], |row| {
        Ok(LibraryTrack {
            media_item_id: row.get(0)?,
            location_id: row.get(1)?,
            path: row.get(2)?,
            library_root: row.get(18)?,
            title: row.get(3)?,
            artist: row.get(4)?,
            album: row.get(5)?,
            album_artist: row.get(6)?,
            album_year: row.get(7)?,
            release_date: row.get(8)?,
            composer: row.get(9)?,
            genre: row.get(10)?,
            cover_path: row.get(11)?,
            track_number: row.get(12)?,
            track_total: row.get(13)?,
            disc_number: row.get(14)?,
            disc_total: row.get(15)?,
            duration_ms: row.get(16)?,
            compilation: row.get::<_, i64>(17)? != 0,
            play_count: row.get(19)?,
        })
    })?;

    let mut tracks = Vec::new();
    for row in rows {
        tracks.push(row?);
    }
    Ok(tracks)
}

pub fn format_duration(duration_ms: Option<i64>) -> String {
    let Some(duration_ms) = duration_ms else {
        return "--:--".to_string();
    };
    let total_seconds = (duration_ms / 1000).max(0);
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}:{seconds:02}")
}

fn count(conn: &Connection, table: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(Into::into)
}

fn path_matches_root_sql(path: &str, root: &str) -> String {
    format!(
        "({path} = {root} OR {root} = '/' OR substr({path}, 1, length({root}) + 1) = {root} || '/')"
    )
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::TrackMetadata;

    #[test]
    fn migrate_sets_schema_user_version() {
        let conn = Connection::open_in_memory().unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_rejects_newer_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();

        let error = migrate(&conn).unwrap_err();

        assert!(error.to_string().contains("newer than this GMUS build"));
    }

    #[test]
    fn browser_selection_round_trips() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let selection = SavedBrowserSelection {
            tree_kind: "album".to_string(),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            playlist_id: None,
            media_item_id: Some(42),
        };

        save_browser_selection(&conn, &selection).unwrap();

        assert_eq!(browser_selection(&conn).unwrap(), Some(selection));
    }

    #[test]
    fn key_bindings_round_trip_and_delete() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "none:char:o".to_string(),
        };
        let second = SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "none:char:m".to_string(),
        };

        save_key_binding(&conn, &first).unwrap();
        save_key_binding(&conn, &second).unwrap();
        assert_eq!(
            key_bindings(&conn).unwrap(),
            vec![second.clone(), first.clone()]
        );

        delete_key_binding_key(&conn, &first.action, &first.key).unwrap();
        assert_eq!(key_bindings(&conn).unwrap(), vec![second.clone()]);

        delete_key_binding(&conn, &second.action).unwrap();
        assert!(key_bindings(&conn).unwrap().is_empty());
    }

    #[test]
    fn restore_settings_and_filter_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert!(restore_filter_enabled(&conn).unwrap());
        assert!(restore_track_enabled(&conn).unwrap());
        assert_eq!(saved_filter(&conn).unwrap(), None);

        save_restore_filter_enabled(&conn, false).unwrap();
        save_restore_track_enabled(&conn, false).unwrap();
        save_filter(&conn, "artist:eno").unwrap();

        assert!(!restore_filter_enabled(&conn).unwrap());
        assert!(!restore_track_enabled(&conn).unwrap());
        assert_eq!(saved_filter(&conn).unwrap().as_deref(), Some("artist:eno"));
    }

    #[test]
    fn pane_layout_round_trips_through_app_settings() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert_eq!(pane_layout(&conn).unwrap(), SavedPaneLayout::default());

        let layout = SavedPaneLayout {
            library_percent_offset: 6,
            info_height_offset: -2,
        };
        save_pane_layout(&conn, layout).unwrap();

        assert_eq!(pane_layout(&conn).unwrap(), layout);
    }

    #[test]
    fn column_layout_width_round_trips_through_app_settings() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert_eq!(column_layout_width(&conn, 75).unwrap(), 75);

        save_column_layout_width(&conn, 92).unwrap();

        assert_eq!(column_layout_width(&conn, 75).unwrap(), 92);
    }

    #[test]
    fn records_completed_play_without_library_membership() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let track = TrackMetadata {
            path: "/tmp/song.flac".into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some("Track".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018-05-11".into()),
            composer: Some("Composer".into()),
            genre: Some("Ambient".into()),
            track_number: Some(1),
            track_total: Some(9),
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        };

        let stored = upsert_track(&conn, &track).unwrap();
        record_play(
            &conn,
            stored.media_item_id,
            stored.location_id,
            120_000,
            true,
        )
        .unwrap();

        let stats = stats(&conn).unwrap();
        assert_eq!(stats.media_items, 1);
        assert_eq!(stats.completed_plays, 1);

        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks[0].album_year, Some(2018));
        assert_eq!(tracks[0].release_date.as_deref(), Some("2018-05-11"));
        assert_eq!(tracks[0].composer.as_deref(), Some("Composer"));
        assert_eq!(tracks[0].genre.as_deref(), Some("Ambient"));
        assert_eq!(tracks[0].track_total, Some(9));
    }

    #[test]
    fn library_tracks_sort_albums_by_year_before_title() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let newer = TrackMetadata {
            path: "/tmp/alpha.flac".into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some("Newer Track".into()),
            artist: Some("Artist".into()),
            album: Some("Alpha".into()),
            album_artist: None,
            album_year: Some(2020),
            release_date: Some("2020-01-01".into()),
            composer: None,
            genre: None,
            track_number: Some(1),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        };
        let older = TrackMetadata {
            path: "/tmp/zulu.flac".into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some("Older Track".into()),
            artist: Some("Artist".into()),
            album: Some("Zulu".into()),
            album_artist: None,
            album_year: Some(1999),
            release_date: Some("1999-01-01".into()),
            composer: None,
            genre: None,
            track_number: Some(1),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        };

        upsert_track(&conn, &newer).unwrap();
        upsert_track(&conn, &older).unwrap();

        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks[0].album.as_deref(), Some("Zulu"));
        assert_eq!(tracks[1].album.as_deref(), Some("Alpha"));
    }

    #[test]
    fn library_tracks_keep_same_album_together_when_disc_years_differ() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let mut dropsonde_one = test_track_metadata(
            "/tmp/dropsonde-disc-1.flac",
            "Dissolving Clouds",
            1,
            120_000,
        );
        dropsonde_one.artist = Some("Biosphere".into());
        dropsonde_one.album_artist = Some("Biosphere".into());
        dropsonde_one.album = Some("Dropsonde".into());
        dropsonde_one.album_year = Some(2006);
        dropsonde_one.disc_number = Some(1);

        let mut n_plants = test_track_metadata("/tmp/n-plants.flac", "Sendai-1", 1, 120_000);
        n_plants.artist = Some("Biosphere".into());
        n_plants.album_artist = Some("Biosphere".into());
        n_plants.album = Some("N-Plants".into());
        n_plants.album_year = Some(2011);
        n_plants.disc_number = Some(1);

        let mut black_mesa = test_track_metadata("/tmp/black-mesa.flac", "Black Mesa", 1, 120_000);
        black_mesa.artist = Some("Biosphere".into());
        black_mesa.album_artist = Some("Biosphere".into());
        black_mesa.album = Some("Black Mesa".into());
        black_mesa.album_year = Some(2017);
        black_mesa.disc_number = Some(1);

        let mut dropsonde_two = test_track_metadata(
            "/tmp/dropsonde-disc-2.flac",
            "Fair Winds For Escort",
            1,
            120_000,
        );
        dropsonde_two.artist = Some("Biosphere".into());
        dropsonde_two.album_artist = Some("Biosphere".into());
        dropsonde_two.album = Some("Dropsonde".into());
        dropsonde_two.album_year = Some(2020);
        dropsonde_two.disc_number = Some(2);

        upsert_track(&conn, &dropsonde_one).unwrap();
        upsert_track(&conn, &n_plants).unwrap();
        upsert_track(&conn, &black_mesa).unwrap();
        upsert_track(&conn, &dropsonde_two).unwrap();

        let tracks = library_tracks(&conn).unwrap();
        let order: Vec<(&str, Option<i64>)> = tracks
            .iter()
            .map(|track| (track.album.as_deref().unwrap(), track.disc_number))
            .collect();

        assert_eq!(
            order,
            vec![
                ("Dropsonde", Some(1)),
                ("Dropsonde", Some(2)),
                ("N-Plants", Some(1)),
                ("Black Mesa", Some(1)),
            ]
        );
    }

    #[test]
    fn library_roots_limit_visible_locations_without_deleting_history() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let in_root = TrackMetadata {
            path: "/tmp/music/song.flac".into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some("In Root".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018".into()),
            composer: None,
            genre: None,
            track_number: Some(1),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        };
        let outside_root = TrackMetadata {
            path: "/tmp/other/song.flac".into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some("Outside Root".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018".into()),
            composer: None,
            genre: None,
            track_number: Some(2),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
        };
        let stored = upsert_track(&conn, &in_root).unwrap();
        upsert_track(&conn, &outside_root).unwrap();
        record_play(
            &conn,
            stored.media_item_id,
            stored.location_id,
            120_000,
            true,
        )
        .unwrap();

        assert_eq!(library_tracks(&conn).unwrap().len(), 2);

        upsert_library_root(&conn, Path::new("/tmp/music")).unwrap();
        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title.as_deref(), Some("In Root"));

        assert!(deactivate_library_root(&conn, Path::new("/tmp/music")).unwrap());
        assert!(library_tracks(&conn).unwrap().is_empty());
        assert_eq!(stats(&conn).unwrap().completed_plays, 1);
    }

    #[test]
    fn playlists_preserve_order_and_allow_duplicate_adds() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = upsert_track(
            &conn,
            &test_track_metadata("/tmp/music/one.flac", "One", 1, 120_000),
        )
        .unwrap();
        let second = upsert_track(
            &conn,
            &test_track_metadata("/tmp/music/two.flac", "Two", 2, 120_000),
        )
        .unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();

        let added = add_tracks_to_playlist(
            &conn,
            playlist.id,
            &[
                second.media_item_id,
                first.media_item_id,
                second.media_item_id,
            ],
        )
        .unwrap();

        assert_eq!(added, 3);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![
                second.media_item_id,
                first.media_item_id,
                second.media_item_id
            ]
        );
        assert_eq!(playlists(&conn).unwrap()[0].name, "Mix");

        let removed =
            remove_tracks_from_playlist(&conn, playlist.id, &[second.media_item_id]).unwrap();

        assert_eq!(removed, 2);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![first.media_item_id]
        );
    }

    #[test]
    fn playlist_remove_latest_deletes_one_duplicate_entry() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = upsert_track(
            &conn,
            &test_track_metadata("/tmp/music/one.flac", "One", 1, 120_000),
        )
        .unwrap();
        let second = upsert_track(
            &conn,
            &test_track_metadata("/tmp/music/two.flac", "Two", 2, 120_000),
        )
        .unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();
        add_tracks_to_playlist(
            &conn,
            playlist.id,
            &[
                second.media_item_id,
                first.media_item_id,
                second.media_item_id,
            ],
        )
        .unwrap();

        let removed =
            remove_latest_tracks_from_playlist(&conn, playlist.id, &[second.media_item_id])
                .unwrap();

        assert_eq!(removed, 1);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![second.media_item_id, first.media_item_id]
        );
    }

    #[test]
    fn migrate_playlist_tracks_preserves_existing_rows_and_allows_duplicates() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = upsert_track(
            &conn,
            &test_track_metadata("/tmp/music/one.flac", "One", 1, 120_000),
        )
        .unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();
        conn.execute_batch(
            r#"
            DROP INDEX IF EXISTS idx_playlist_tracks_playlist_position;
            DROP TABLE playlist_tracks;
            CREATE TABLE playlist_tracks (
                playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                media_item_id   INTEGER NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
                position        INTEGER NOT NULL,
                added_at        INTEGER NOT NULL,
                PRIMARY KEY (playlist_id, media_item_id)
            );
            "#,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO playlist_tracks (playlist_id, media_item_id, position, added_at) VALUES (?1, ?2, 0, 1)",
            params![playlist.id, first.media_item_id],
        )
        .unwrap();

        migrate(&conn).unwrap();
        let added = add_tracks_to_playlist(&conn, playlist.id, &[first.media_item_id]).unwrap();

        assert_eq!(added, 1);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![first.media_item_id, first.media_item_id]
        );
    }

    #[test]
    fn mark_locations_missing_under_root_hides_nested_stale_locations() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let track = test_track_metadata("/tmp/music/album/song.flac", "Nested Track", 1, 120_000);

        upsert_track(&conn, &track).unwrap();
        assert_eq!(library_tracks(&conn).unwrap().len(), 1);

        let marked = mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();

        assert_eq!(marked, 1);
        assert!(library_tracks(&conn).unwrap().is_empty());
    }

    #[test]
    fn filesystem_root_matches_all_library_locations() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let track = test_track_metadata("/tmp/music/album/song.flac", "Nested Track", 1, 120_000);

        upsert_track(&conn, &track).unwrap();
        upsert_library_root(&conn, Path::new("/")).unwrap();

        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].library_root.as_deref(), Some("/"));

        let marked = mark_locations_missing_under_root(&conn, Path::new("/")).unwrap();

        assert_eq!(marked, 1);
        assert!(library_tracks(&conn).unwrap().is_empty());
    }

    #[test]
    fn merge_similar_media_items_combines_play_counts_for_renamed_tracks() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/wrong-name.flac", "Same Track", 1, 120_000);
        let mut renamed =
            test_track_metadata("/tmp/music/right-name.flac", "Same Track", 1, 121_000);
        renamed.modified_at = Some(2);

        let old_stored = upsert_track(&conn, &old).unwrap();
        record_play(
            &conn,
            old_stored.media_item_id,
            old_stored.location_id,
            120_000,
            true,
        )
        .unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        upsert_track(&conn, &renamed).unwrap();

        let merged = merge_similar_media_items(&conn).unwrap();
        let tracks = library_tracks(&conn).unwrap();

        assert_eq!(merged, 1);
        assert_eq!(stats(&conn).unwrap().media_items, 1);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, "/tmp/music/right-name.flac");
        assert_eq!(tracks[0].play_count, 1);
    }

    #[test]
    fn merge_similar_media_items_preserves_playlist_entries() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/wrong-name.flac", "Same Track", 1, 120_000);
        let mut renamed =
            test_track_metadata("/tmp/music/right-name.flac", "Same Track", 1, 121_000);
        renamed.modified_at = Some(2);

        let old_stored = upsert_track(&conn, &old).unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();
        add_tracks_to_playlist(&conn, playlist.id, &[old_stored.media_item_id]).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        let renamed_stored = upsert_track(&conn, &renamed).unwrap();

        let merged = merge_similar_media_items(&conn).unwrap();

        assert_eq!(merged, 1);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![renamed_stored.media_item_id]
        );
    }

    #[test]
    fn upsert_track_preserves_history_when_same_path_fingerprint_changes() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let original = test_track_metadata("/tmp/music/song.flac", "Same Track", 1, 120_000);
        let mut retagged = original.clone();
        retagged.duration_ms = Some(125_000);
        retagged.modified_at = Some(2);

        let original_stored = upsert_track(&conn, &original).unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();
        add_tracks_to_playlist(&conn, playlist.id, &[original_stored.media_item_id]).unwrap();
        record_play(
            &conn,
            original_stored.media_item_id,
            original_stored.location_id,
            120_000,
            true,
        )
        .unwrap();

        let retagged_stored = upsert_track(&conn, &retagged).unwrap();
        let tracks = library_tracks(&conn).unwrap();

        assert_ne!(original_stored.media_item_id, retagged_stored.media_item_id);
        assert_eq!(stats(&conn).unwrap().media_items, 1);
        assert_eq!(stats(&conn).unwrap().completed_plays, 1);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].media_item_id, retagged_stored.media_item_id);
        assert_eq!(tracks[0].duration_ms, Some(125_000));
        assert_eq!(tracks[0].play_count, 1);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![retagged_stored.media_item_id]
        );
    }

    fn test_track_metadata(
        path: &str,
        title: &str,
        track_number: i64,
        duration_ms: i64,
    ) -> TrackMetadata {
        TrackMetadata {
            path: path.into(),
            file_size: 10,
            modified_at: Some(1),
            title: Some(title.into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018-05-11".into()),
            composer: None,
            genre: None,
            track_number: Some(track_number),
            track_total: Some(10),
            disc_number: None,
            disc_total: None,
            duration_ms: Some(duration_ms),
            compilation: false,
        }
    }
}
