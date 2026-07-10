use anyhow::{Context, Result};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};

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
    expected_media_item_id: i64,
    location_id: i64,
    duration_ms: i64,
    completed: bool,
) -> Result<i64> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let expected_media_item_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM media_items WHERE id = ?1)",
        params![expected_media_item_id],
        |row| row.get(0),
    )?;
    let (media_item_id, event_location_id) = if expected_media_item_exists {
        let location_matches: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM locations WHERE id = ?1 AND media_item_id = ?2)",
            params![location_id, expected_media_item_id],
            |row| row.get(0),
        )?;
        (
            expected_media_item_id,
            location_matches.then_some(location_id),
        )
    } else {
        let media_item_id = tx
            .query_row(
                "SELECT media_item_id FROM locations WHERE id = ?1",
                params![location_id],
                |row| row.get(0),
            )
            .with_context(|| format!("resolving media item for location {location_id}"))?;
        (media_item_id, Some(location_id))
    };

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
            event_location_id,
            now,
            duration_ms.max(0),
            completed_i64
        ],
    )?;

    tx.execute(
        r#"
        INSERT INTO media_stats (
            media_item_id, play_count, last_played_at, total_play_ms, skip_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(media_item_id) DO UPDATE SET
            play_count = media_stats.play_count + excluded.play_count,
            last_played_at = COALESCE(excluded.last_played_at, media_stats.last_played_at),
            total_play_ms = media_stats.total_play_ms + excluded.total_play_ms,
            skip_count = media_stats.skip_count + excluded.skip_count
        "#,
        params![
            media_item_id,
            completed_i64,
            if completed { Some(now) } else { None },
            duration_ms.max(0),
            i64::from(!completed)
        ],
    )?;

    tx.commit()?;
    Ok(media_item_id)
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
