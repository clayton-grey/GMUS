use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::now_unix;

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

pub(super) fn ensure_key_bindings_allow_duplicates(conn: &Connection) -> Result<()> {
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
