use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::media::{FileStamp, TrackMetadata, SCAN_VERSION};

use super::now_unix;
use super::path;

#[derive(Debug, Clone, Copy)]
pub struct StoredTrack {
    pub media_item_id: i64,
    pub location_id: i64,
}

#[derive(Debug, Clone)]
pub struct LibraryTrack {
    pub media_item_id: i64,
    pub location_id: i64,
    pub path: String,
    pub file_path: PathBuf,
    pub library_root: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_year: Option<i64>,
    pub release_date: Option<String>,
    pub composer: Option<String>,
    pub genre: Option<String>,
    pub cover_path: Option<PathBuf>,
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

pub fn upsert_track(conn: &Connection, track: &TrackMetadata) -> Result<StoredTrack> {
    let tx = conn.unchecked_transaction()?;
    let now = now_unix();
    let path = path::encode(&track.path);
    let media_item_id = match location_identity_for_path(&tx, &path)? {
        Some((location_id, media_item_id))
            if media_item_has_other_present_location(&tx, media_item_id, location_id)? =>
        {
            detach_location(&tx, location_id, media_item_id)?
        }
        Some((_location_id, media_item_id)) => media_item_id,
        None => insert_media_item(&tx, track, now)?,
    };
    update_media_item(&tx, media_item_id, track, now)?;

    tx.execute(
        r#"
        INSERT INTO locations (
            media_item_id, path, file_size, modified_at, modified_at_ns,
            fs_device, fs_inode, seen_at, missing, scan_version
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, NULL)
        ON CONFLICT(path) DO UPDATE SET
            media_item_id = excluded.media_item_id,
            file_size = excluded.file_size,
            modified_at = excluded.modified_at,
            modified_at_ns = excluded.modified_at_ns,
            fs_device = excluded.fs_device,
            fs_inode = excluded.fs_inode,
            seen_at = excluded.seen_at,
            missing = 0,
            scan_version = NULL
        "#,
        params![
            media_item_id,
            path,
            track.file_size,
            track.modified_at,
            track.modified_at_ns,
            track.fs_device,
            track.fs_inode,
            now
        ],
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

pub fn location_scan_is_current(conn: &Connection, path: &Path, stamp: FileStamp) -> Result<bool> {
    conn.query_row(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM locations
            WHERE path = ?1
                AND missing = 0
                AND file_size IS ?2
                AND modified_at_ns IS ?3
                AND fs_device IS ?4
                AND fs_inode IS ?5
                AND scan_version = ?6
        )
        "#,
        params![
            path::encode(path),
            stamp.file_size,
            stamp.modified_at_ns,
            stamp.fs_device,
            stamp.fs_inode,
            SCAN_VERSION
        ],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn mark_location_scan_current(conn: &Connection, path: &Path) -> Result<()> {
    conn.execute(
        "UPDATE locations SET scan_version = ?1 WHERE path = ?2 AND missing = 0",
        params![SCAN_VERSION, path::encode(path)],
    )?;
    Ok(())
}

fn insert_media_item(conn: &Connection, track: &TrackMetadata, now: i64) -> Result<i64> {
    conn.execute(
        r#"
        INSERT INTO media_items (
            duplicate_key, title, artist, album, album_artist, album_year, release_date,
            composer, genre, track_number, track_total, disc_number, disc_total,
            duration_ms, compilation, first_seen_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
        "#,
        params![
            track.duplicate_key(),
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
    Ok(conn.last_insert_rowid())
}

fn update_media_item(
    conn: &Connection,
    media_item_id: i64,
    track: &TrackMetadata,
    now: i64,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE media_items
        SET duplicate_key = ?1,
            title = ?2,
            artist = ?3,
            album = ?4,
            album_artist = ?5,
            album_year = ?6,
            release_date = ?7,
            composer = ?8,
            genre = ?9,
            track_number = ?10,
            track_total = ?11,
            disc_number = ?12,
            disc_total = ?13,
            duration_ms = ?14,
            compilation = ?15,
            updated_at = ?16
        WHERE id = ?17
        "#,
        params![
            track.duplicate_key(),
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
            now,
            media_item_id
        ],
    )?;
    Ok(())
}

fn location_identity_for_path(conn: &Connection, path: &str) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT id, media_item_id FROM locations WHERE path = ?1",
        params![path],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn media_item_has_other_present_location(
    conn: &Connection,
    media_item_id: i64,
    location_id: i64,
) -> Result<bool> {
    conn.query_row(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM locations
            WHERE media_item_id = ?1 AND id != ?2 AND missing = 0
        )
        "#,
        params![media_item_id, location_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn detach_location(conn: &Connection, location_id: i64, media_item_id: i64) -> Result<i64> {
    detach_location_with_event_policy(conn, location_id, media_item_id, false)
}

pub(super) fn split_legacy_location(
    conn: &Connection,
    location_id: i64,
    media_item_id: i64,
) -> Result<i64> {
    detach_location_with_event_policy(conn, location_id, media_item_id, true)
}

fn detach_location_with_event_policy(
    conn: &Connection,
    location_id: i64,
    media_item_id: i64,
    events_follow_location: bool,
) -> Result<i64> {
    let stats_residual = media_stats_row(conn, media_item_id)?
        .unwrap_or_default()
        .residual_after(&play_event_stats_row(conn, media_item_id)?);
    conn.execute(
        r#"
        INSERT INTO media_items (
            duplicate_key, title, artist, album, album_artist, album_year, release_date,
            composer, genre, cover_path, track_number, track_total, disc_number, disc_total,
            duration_ms, compilation, first_seen_at, updated_at
        )
        SELECT
            duplicate_key, title, artist, album, album_artist, album_year, release_date,
            composer, genre, NULL, track_number, track_total, disc_number, disc_total,
            duration_ms, compilation, first_seen_at, updated_at
        FROM media_items
        WHERE id = ?1
        "#,
        params![media_item_id],
    )?;
    let detached_media_item_id = conn.last_insert_rowid();
    if events_follow_location {
        conn.execute(
            "UPDATE play_events SET media_item_id = ?1 WHERE location_id = ?2",
            params![detached_media_item_id, location_id],
        )?;
    } else {
        conn.execute(
            "UPDATE play_events SET location_id = NULL WHERE location_id = ?1 AND media_item_id = ?2",
            params![location_id, media_item_id],
        )?;
    }
    conn.execute(
        "UPDATE locations SET media_item_id = ?1 WHERE id = ?2",
        params![detached_media_item_id, location_id],
    )?;
    rebuild_media_stats(conn, media_item_id)?;
    rebuild_media_stats(conn, detached_media_item_id)?;
    add_media_stats(conn, media_item_id, &stats_residual)?;
    Ok(detached_media_item_id)
}

fn rebuild_media_stats(conn: &Connection, media_item_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM media_stats WHERE media_item_id = ?1",
        params![media_item_id],
    )?;
    conn.execute(
        r#"
        INSERT INTO media_stats (
            media_item_id, play_count, last_played_at, total_play_ms, skip_count
        )
        SELECT
            media_item_id,
            SUM(completed),
            MAX(CASE WHEN completed = 1 THEN played_at END),
            SUM(duration_ms),
            SUM(CASE WHEN completed = 0 THEN 1 ELSE 0 END)
        FROM play_events
        WHERE media_item_id = ?1
        GROUP BY media_item_id
        "#,
        params![media_item_id],
    )?;
    Ok(())
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
            stmt.execute(params![path::encode(path)])?;
        }
    }

    let root = path::encode(root);
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

pub fn reconcile_renamed_media_items(conn: &Connection) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut groups: HashMap<(i64, i64), Vec<MergeCandidate>> = HashMap::new();
    {
        let sql = r#"
            SELECT
                id,
                (
                    SELECT COUNT(*)
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                        AND locations.missing = 0
                ),
                (
                    SELECT fs_device
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                    ORDER BY locations.missing ASC, locations.seen_at DESC, locations.id DESC
                    LIMIT 1
                ),
                (
                    SELECT fs_inode
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                    ORDER BY locations.missing ASC, locations.seen_at DESC, locations.id DESC
                    LIMIT 1
                ),
                (
                    SELECT file_size
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                    ORDER BY locations.missing ASC, locations.seen_at DESC, locations.id DESC
                    LIMIT 1
                ),
                (
                    SELECT modified_at
                    FROM locations
                    WHERE locations.media_item_id = media_items.id
                    ORDER BY locations.missing ASC, locations.seen_at DESC, locations.id DESC
                    LIMIT 1
                )
            FROM media_items
            ORDER BY id
            "#;
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(MergeCandidate {
                id: row.get(0)?,
                present_locations: row.get(1)?,
                fs_device: row.get(2)?,
                fs_inode: row.get(3)?,
                file_size: row.get(4)?,
                modified_at: row.get(5)?,
            })
        })?;

        for row in rows {
            let candidate = row?;
            if let Some(key) = candidate.filesystem_identity() {
                groups.entry(key).or_default().push(candidate);
            }
        }
    }

    let mut merged = 0;
    for candidates in groups
        .into_values()
        .filter(|candidates| candidates.len() == 2)
    {
        let mut present = candidates
            .iter()
            .filter(|candidate| candidate.present_locations > 0);
        let Some(canonical) = present.next() else {
            continue;
        };
        if present.next().is_some() {
            continue;
        }
        let Some(duplicate) = candidates
            .iter()
            .find(|candidate| candidate.present_locations == 0)
        else {
            continue;
        };
        if canonical.file_signature().is_none()
            || canonical.file_signature() != duplicate.file_signature()
        {
            continue;
        }
        merge_media_item(&tx, canonical.id, duplicate.id)?;
        merged += 1;
    }
    tx.commit()?;
    Ok(merged)
}

#[derive(Debug)]
struct MergeCandidate {
    id: i64,
    present_locations: i64,
    fs_device: Option<i64>,
    fs_inode: Option<i64>,
    file_size: Option<i64>,
    modified_at: Option<i64>,
}

impl MergeCandidate {
    fn filesystem_identity(&self) -> Option<(i64, i64)> {
        Some((self.fs_device?, self.fs_inode?))
    }

    fn file_signature(&self) -> Option<(i64, i64)> {
        Some((self.file_size?, self.modified_at?))
    }
}

#[derive(Debug, Default)]
pub(super) struct MediaStatsRow {
    pub(super) play_count: i64,
    pub(super) last_played_at: Option<i64>,
    pub(super) total_play_ms: i64,
    pub(super) skip_count: i64,
}

impl MediaStatsRow {
    fn residual_after(&self, event_stats: &Self) -> Self {
        Self {
            play_count: self
                .play_count
                .saturating_sub(event_stats.play_count)
                .max(0),
            last_played_at: match (self.last_played_at, event_stats.last_played_at) {
                (Some(stats), Some(events)) if stats > events => Some(stats),
                (Some(stats), None) => Some(stats),
                _ => None,
            },
            total_play_ms: self
                .total_play_ms
                .saturating_sub(event_stats.total_play_ms)
                .max(0),
            skip_count: self
                .skip_count
                .saturating_sub(event_stats.skip_count)
                .max(0),
        }
    }

    fn is_empty(&self) -> bool {
        self.play_count == 0
            && self.last_played_at.is_none()
            && self.total_play_ms == 0
            && self.skip_count == 0
    }
}

fn merge_media_item(conn: &Connection, canonical_id: i64, duplicate_id: i64) -> Result<()> {
    if canonical_id == duplicate_id {
        return Ok(());
    }

    conn.execute(
        r#"
        UPDATE media_items
        SET first_seen_at = MIN(first_seen_at, (
                SELECT first_seen_at FROM media_items WHERE id = ?2
            ))
        WHERE id = ?1
        "#,
        params![canonical_id, duplicate_id],
    )?;
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
        r#"
        UPDATE app_browser_selection
        SET media_item_id = ?1,
            artist = (
                SELECT COALESCE(album_artist, artist)
                FROM media_items
                WHERE id = ?1
            ),
            album = (
                SELECT album
                FROM media_items
                WHERE id = ?1
            )
        WHERE media_item_id = ?2
        "#,
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

pub(super) fn media_stats_row(
    conn: &Connection,
    media_item_id: i64,
) -> Result<Option<MediaStatsRow>> {
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

fn play_event_stats_row(conn: &Connection, media_item_id: i64) -> Result<MediaStatsRow> {
    conn.query_row(
        r#"
        SELECT
            COALESCE(SUM(completed), 0),
            MAX(CASE WHEN completed = 1 THEN played_at END),
            COALESCE(SUM(duration_ms), 0),
            COALESCE(SUM(CASE WHEN completed = 0 THEN 1 ELSE 0 END), 0)
        FROM play_events
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
    .map_err(Into::into)
}

fn add_media_stats(conn: &Connection, media_item_id: i64, stats: &MediaStatsRow) -> Result<()> {
    if stats.is_empty() {
        return Ok(());
    }
    conn.execute(
        r#"
        INSERT INTO media_stats (
            media_item_id, play_count, last_played_at, total_play_ms, skip_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(media_item_id) DO UPDATE SET
            play_count = media_stats.play_count + excluded.play_count,
            last_played_at = CASE
                WHEN media_stats.last_played_at IS NULL THEN excluded.last_played_at
                WHEN excluded.last_played_at IS NULL THEN media_stats.last_played_at
                ELSE MAX(media_stats.last_played_at, excluded.last_played_at)
            END,
            total_play_ms = media_stats.total_play_ms + excluded.total_play_ms,
            skip_count = media_stats.skip_count + excluded.skip_count
        "#,
        params![
            media_item_id,
            stats.play_count,
            stats.last_played_at,
            stats.total_play_ms,
            stats.skip_count
        ],
    )?;
    Ok(())
}

pub fn set_cover_path(conn: &Connection, media_item_id: i64, path: &Path) -> Result<()> {
    conn.execute(
        "UPDATE media_items SET cover_path = ?1, updated_at = ?2 WHERE id = ?3",
        params![path::encode(path), now_unix(), media_item_id],
    )?;
    Ok(())
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
        let encoded_path: String = row.get(2)?;
        let file_path = path::decode(&encoded_path);
        let encoded_root: Option<String> = row.get(18)?;
        let encoded_cover: Option<String> = row.get(11)?;
        Ok(LibraryTrack {
            media_item_id: row.get(0)?,
            location_id: row.get(1)?,
            path: path::display(&file_path),
            file_path,
            library_root: encoded_root.map(|root| path::display(&path::decode(&root))),
            title: row.get(3)?,
            artist: row.get(4)?,
            album: row.get(5)?,
            album_artist: row.get(6)?,
            album_year: row.get(7)?,
            release_date: row.get(8)?,
            composer: row.get(9)?,
            genre: row.get(10)?,
            cover_path: encoded_cover.map(|cover| path::decode(&cover)),
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

fn path_matches_root_sql(path: &str, root: &str) -> String {
    format!(
        r#"(
            {path} = {root}
            OR {root} = 'unix:2f'
            OR (
                substr({root}, 1, 5) = 'unix:'
                AND substr({path}, 1, length({root}) + 2) = {root} || '2f'
            )
            OR (
                substr({root}, 1, 5) != 'unix:'
                AND ({root} = '/' OR substr({path}, 1, length({root}) + 1) = {root} || '/')
            )
        )"#
    )
}
