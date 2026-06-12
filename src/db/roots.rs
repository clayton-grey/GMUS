use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::now_unix;

#[derive(Debug, Clone)]
pub struct LibraryRoot {
    pub path: String,
    pub active: bool,
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
