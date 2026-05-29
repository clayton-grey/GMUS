use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::media::TrackMetadata;

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

#[derive(Debug, Clone)]
pub struct LibraryTrack {
    pub media_item_id: i64,
    pub location_id: i64,
    pub path: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_year: Option<i64>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub cover_path: Option<String>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration_ms: Option<i64>,
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
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
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
            composer        TEXT,
            genre           TEXT,
            cover_path      TEXT,
            track_number    INTEGER,
            disc_number     INTEGER,
            duration_ms     INTEGER,
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

        CREATE INDEX IF NOT EXISTS idx_locations_media_item
            ON locations(media_item_id);
        CREATE INDEX IF NOT EXISTS idx_play_events_media_item
            ON play_events(media_item_id, played_at);
        CREATE INDEX IF NOT EXISTS idx_media_items_artist_album
            ON media_items(album_artist, artist, album);
        "#,
    )?;
    ensure_column(conn, "media_items", "cover_path", "TEXT")?;
    ensure_column(conn, "media_items", "album_year", "INTEGER")?;
    ensure_column(conn, "media_items", "composer", "TEXT")?;
    ensure_column(conn, "media_items", "genre", "TEXT")?;
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

pub fn upsert_track(conn: &Connection, track: &TrackMetadata) -> Result<StoredTrack> {
    let now = now_unix();
    let fingerprint = track.fingerprint();

    conn.execute(
        r#"
        INSERT INTO media_items (
            fingerprint, title, artist, album, album_artist, album_year,
            composer, genre, track_number, disc_number, duration_ms, first_seen_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
        ON CONFLICT(fingerprint) DO UPDATE SET
            title = excluded.title,
            artist = excluded.artist,
            album = excluded.album,
            album_artist = excluded.album_artist,
            album_year = excluded.album_year,
            composer = excluded.composer,
            genre = excluded.genre,
            track_number = excluded.track_number,
            disc_number = excluded.disc_number,
            duration_ms = excluded.duration_ms,
            updated_at = excluded.updated_at
        "#,
        params![
            fingerprint,
            track.title,
            track.artist,
            track.album,
            track.album_artist,
            track.album_year,
            track.composer,
            track.genre,
            track.track_number,
            track.disc_number,
            track.duration_ms,
            now
        ],
    )?;

    let media_item_id: i64 = conn.query_row(
        "SELECT id FROM media_items WHERE fingerprint = ?1",
        params![fingerprint],
        |row| row.get(0),
    )?;

    let path = track.path.to_string_lossy();
    conn.execute(
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

    let location_id: i64 = conn.query_row(
        "SELECT id FROM locations WHERE path = ?1",
        params![path],
        |row| row.get(0),
    )?;

    conn.execute(
        r#"
        INSERT INTO media_stats (media_item_id)
        VALUES (?1)
        ON CONFLICT(media_item_id) DO NOTHING
        "#,
        params![media_item_id],
    )?;

    Ok(StoredTrack {
        media_item_id,
        location_id,
    })
}

pub fn record_play(
    conn: &Connection,
    media_item_id: i64,
    location_id: i64,
    duration_ms: i64,
    completed: bool,
) -> Result<()> {
    let now = now_unix();
    let completed_i64 = i64::from(completed);
    conn.execute(
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

    conn.execute(
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
    let mut stmt = conn.prepare(
        r#"
        SELECT
            media_items.id,
            locations.id,
            locations.path,
            media_items.title,
            media_items.artist,
            media_items.album,
            media_items.album_artist,
            media_items.album_year,
            media_items.composer,
            media_items.genre,
            media_items.cover_path,
            media_items.track_number,
            media_items.disc_number,
            media_items.duration_ms,
            COALESCE(media_stats.play_count, 0)
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
                        AND (
                            locations.path = library_roots.path
                            OR library_roots.path = '/'
                            OR substr(locations.path, 1, length(library_roots.path) + 1) =
                                library_roots.path || '/'
                        )
                )
            )
        ORDER BY
            COALESCE(media_items.album_artist, media_items.artist, ''),
            COALESCE(media_items.album_year, 9223372036854775807),
            COALESCE(media_items.album, ''),
            COALESCE(media_items.disc_number, 0),
            COALESCE(media_items.track_number, 0),
            COALESCE(media_items.title, locations.path)
        "#,
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(LibraryTrack {
            media_item_id: row.get(0)?,
            location_id: row.get(1)?,
            path: row.get(2)?,
            title: row.get(3)?,
            artist: row.get(4)?,
            album: row.get(5)?,
            album_artist: row.get(6)?,
            album_year: row.get(7)?,
            composer: row.get(8)?,
            genre: row.get(9)?,
            cover_path: row.get(10)?,
            track_number: row.get(11)?,
            disc_number: row.get(12)?,
            duration_ms: row.get(13)?,
            play_count: row.get(14)?,
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
            composer: Some("Composer".into()),
            genre: Some("Ambient".into()),
            track_number: Some(1),
            disc_number: None,
            duration_ms: Some(120_000),
            embedded_art: None,
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
        assert_eq!(tracks[0].composer.as_deref(), Some("Composer"));
        assert_eq!(tracks[0].genre.as_deref(), Some("Ambient"));
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
            composer: None,
            genre: None,
            track_number: Some(1),
            disc_number: None,
            duration_ms: Some(120_000),
            embedded_art: None,
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
            composer: None,
            genre: None,
            track_number: Some(1),
            disc_number: None,
            duration_ms: Some(120_000),
            embedded_art: None,
        };

        upsert_track(&conn, &newer).unwrap();
        upsert_track(&conn, &older).unwrap();

        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks[0].album.as_deref(), Some("Zulu"));
        assert_eq!(tracks[1].album.as_deref(), Some("Alpha"));
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
            composer: None,
            genre: None,
            track_number: Some(1),
            disc_number: None,
            duration_ms: Some(120_000),
            embedded_art: None,
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
            composer: None,
            genre: None,
            track_number: Some(2),
            disc_number: None,
            duration_ms: Some(120_000),
            embedded_art: None,
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
}
