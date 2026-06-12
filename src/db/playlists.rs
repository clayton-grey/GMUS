use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::now_unix;

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
    let mut seen = HashSet::new();
    media_item_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect()
}

pub(super) fn ensure_playlist_tracks_allow_duplicates(conn: &Connection) -> Result<()> {
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
