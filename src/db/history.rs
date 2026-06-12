use anyhow::Result;
use rusqlite::{params, Connection};

use super::now_unix;

#[derive(Debug, Clone, Copy)]
pub struct DbStats {
    pub media_items: i64,
    pub locations: i64,
    pub play_events: i64,
    pub completed_plays: i64,
}

pub fn record_play(
    conn: &Connection,
    media_item_id: i64,
    location_id: i64,
    duration_ms: i64,
    completed: bool,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let location_matches_media_item: bool = tx.query_row(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM locations
            WHERE id = ?1 AND media_item_id = ?2
        )
        "#,
        params![location_id, media_item_id],
        |row| row.get(0),
    )?;
    if !location_matches_media_item {
        anyhow::bail!("location {location_id} does not belong to media item {media_item_id}");
    }

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

pub(super) fn count(conn: &Connection, table: &str) -> Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(Into::into)
}
