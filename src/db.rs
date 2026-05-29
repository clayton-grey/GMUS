use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

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

pub fn mark_locations_missing_under_root(conn: &Connection, root: &Path) -> Result<usize> {
    let root = root.to_string_lossy();
    conn.execute(
        r#"
        UPDATE locations
        SET missing = 1
        WHERE missing = 0
            AND (
                path = ?1
                OR ?1 = '/'
                OR substr(path, 1, length(?1) + 1) = ?1 || '/'
            )
        "#,
        params![root],
    )
    .map_err(Into::into)
}

pub fn merge_similar_media_items(conn: &Connection) -> Result<usize> {
    let mut stmt = conn.prepare(
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
                        ON locations.path = library_roots.path
                        OR library_roots.path = '/'
                        OR substr(locations.path, 1, length(library_roots.path) + 1) =
                            library_roots.path || '/'
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
    )?;
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

    let mut groups: HashMap<String, Vec<MergeCandidate>> = HashMap::new();
    for row in rows {
        let candidate = row?;
        if let Some(key) = candidate.similarity_key() {
            groups.entry(key).or_default().push(candidate);
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
            merge_media_item(conn, canonical_id, duplicate.id)?;
            merged += 1;
        }
    }
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
                    AND (
                        locations.path = library_roots.path
                        OR library_roots.path = '/'
                        OR substr(locations.path, 1, length(library_roots.path) + 1) =
                            library_roots.path || '/'
                    )
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
                        AND (
                            locations.path = library_roots.path
                            OR library_roots.path = '/'
                            OR substr(locations.path, 1, length(library_roots.path) + 1) =
                                library_roots.path || '/'
                        )
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
                OVER (PARTITION BY artist_sort, COALESCE(library_root, ''), album_sort),
            COALESCE(library_root, ''),
            album_sort,
            COALESCE(disc_number, 0),
            COALESCE(track_number, 0),
            COALESCE(title, path)
        "#,
    )?;

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
            release_date: Some("2018-05-11".into()),
            composer: Some("Composer".into()),
            genre: Some("Ambient".into()),
            track_number: Some(1),
            track_total: Some(9),
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
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
            release_date: Some("1999-01-01".into()),
            composer: None,
            genre: None,
            track_number: Some(1),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
            embedded_art: None,
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
            release_date: Some("2018".into()),
            composer: None,
            genre: None,
            track_number: Some(2),
            track_total: None,
            disc_number: None,
            disc_total: None,
            duration_ms: Some(120_000),
            compilation: false,
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
            embedded_art: None,
        }
    }
}
