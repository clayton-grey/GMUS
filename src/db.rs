use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::art::ArtworkMode;

mod catalog;
mod history;
mod migrations;
mod path;
mod playlists;
mod roots;
mod settings;

#[cfg(test)]
pub use catalog::location_scan_is_current;
#[cfg(test)]
pub use catalog::mark_locations_missing_under_root;
#[cfg(test)]
use catalog::media_stats_row;
#[allow(unused_imports)]
pub use catalog::{
    clear_cover_path, library_tracks, mark_locations_missing_under_root_except,
    media_item_id_for_location, reconcile_renamed_media_items, set_cover_path, upsert_track,
    LibraryTrack, StoredTrack,
};
pub(crate) use catalog::{location_scan_is_current_for_mode, mark_location_scan_current_for_mode};
#[cfg(test)]
use history::count;
#[allow(unused_imports)]
pub use history::{record_play, stats, DbStats};
use migrations::migrate;
#[cfg(test)]
use migrations::{user_version, SCHEMA_VERSION};
#[cfg(test)]
pub use playlists::playlist_track_ids;
#[allow(unused_imports)]
pub use playlists::PlaylistTrack;
pub use playlists::{
    add_tracks_to_playlist, clear_playlist, create_playlist, delete_playlist, playlist_by_name,
    playlist_tracks, playlists, remove_latest_tracks_from_playlist, remove_playlist_track_entries,
    remove_tracks_from_playlist, Playlist,
};
pub use roots::{
    active_library_roots, deactivate_library_root, library_roots, mark_library_root_scanned,
    set_library_root_active, upsert_library_root, LibraryRoot,
};
pub use settings::{
    browser_selection, column_layout_width, delete_key_binding, delete_key_binding_key,
    delete_key_bindings, key_bindings, pane_layout, restore_filter_enabled, restore_track_enabled,
    save_browser_selection, save_column_layout_width, save_filter, save_key_binding,
    save_pane_layout, save_restore_filter_enabled, save_restore_track_enabled, saved_filter,
    SavedBrowserSelection, SavedKeyBinding, SavedPaneLayout,
};

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn prepare_art_cache(conn: &Connection, art_dir: &Path) -> Result<()> {
    prepare_art_cache_with_mode(conn, art_dir, ArtworkMode::from_env())
}

fn prepare_art_cache_with_mode(
    conn: &Connection,
    art_dir: &Path,
    artwork_mode: ArtworkMode,
) -> Result<()> {
    const NAMESPACE_KEY: &str = "art-cache-namespace";

    fs::create_dir_all(art_dir)
        .with_context(|| format!("creating cover-art directory {}", art_dir.display()))?;
    let art_dir = art_dir
        .canonicalize()
        .with_context(|| format!("resolving cover-art directory {}", art_dir.display()))?;

    if !artwork_mode.caches_during_scan() {
        clear_disabled_art_cache(conn, &art_dir)?;
        return Ok(());
    }

    let namespace = path::encode(&art_dir);
    let saved_namespace = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![NAMESPACE_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if saved_namespace.as_deref() == Some(namespace.as_str()) {
        return Ok(());
    }

    let covers = {
        let mut stmt =
            conn.prepare("SELECT id, cover_path FROM media_items WHERE cover_path IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let migrated = covers
        .into_iter()
        .map(|(media_item_id, cover_path)| {
            let source = path::decode(&cover_path);
            migrate_cached_cover(&source, &art_dir, media_item_id)
                .map(|destination| (media_item_id, destination))
        })
        .collect::<Result<Vec<_>>>()?;

    let tx = conn.unchecked_transaction()?;
    for (media_item_id, destination) in migrated {
        tx.execute(
            "UPDATE media_items SET cover_path = ?1 WHERE id = ?2",
            params![destination.as_deref().map(path::encode), media_item_id],
        )?;
    }
    tx.execute("UPDATE locations SET scan_version = NULL", [])?;
    tx.execute(
        r#"
        INSERT INTO app_settings (key, value, updated_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at
        "#,
        params![NAMESPACE_KEY, namespace, now_unix()],
    )?;
    tx.commit()?;
    Ok(())
}

fn clear_disabled_art_cache(conn: &Connection, art_dir: &Path) -> Result<()> {
    let cached_cover_ids = {
        let mut stmt =
            conn.prepare("SELECT id, cover_path FROM media_items WHERE cover_path IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let tx = conn.unchecked_transaction()?;
    for (media_item_id, cover_path) in cached_cover_ids {
        let cover_path = path::decode(&cover_path);
        if path_is_inside(&cover_path, art_dir) {
            tx.execute(
                "UPDATE media_items SET cover_path = NULL WHERE id = ?1",
                params![media_item_id],
            )?;
        }
    }
    tx.commit()?;

    remove_art_cache_contents(art_dir)
}

fn path_is_inside(path: &Path, parent: &Path) -> bool {
    path.starts_with(parent)
        || path
            .canonicalize()
            .is_ok_and(|canonical| canonical.starts_with(parent))
}

fn remove_art_cache_contents(art_dir: &Path) -> Result<()> {
    for entry in
        fs::read_dir(art_dir).with_context(|| format!("reading art cache {}", art_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading art cache {}", art_dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading art cache entry {}", path.display()))?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("removing cached artwork directory {}", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("removing cached artwork {}", path.display()))?;
        }
    }
    Ok(())
}

fn migrate_cached_cover(
    source: &Path,
    art_dir: &Path,
    media_item_id: i64,
) -> Result<Option<PathBuf>> {
    if !source.is_file() {
        return Ok(None);
    }

    let mut destination = art_dir.join(media_item_id.to_string());
    if let Some(extension) = source.extension() {
        destination.set_extension(extension);
    }
    let same_file = source == destination
        || source
            .canonicalize()
            .ok()
            .zip(destination.canonicalize().ok())
            .is_some_and(|(source, destination)| source == destination);
    if !same_file {
        fs::copy(source, &destination).with_context(|| {
            format!(
                "copying cached cover art from {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(Some(destination))
}

#[cfg(test)]
pub(crate) fn open_in_memory_for_tests() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
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

pub(super) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::ArtworkMode;
    use crate::media::TrackMetadata;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn migrate_sets_schema_user_version() {
        let conn = migration_test_connection();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_rejects_newer_schema_version() {
        let conn = migration_test_connection();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();

        let error = migrate(&conn).unwrap_err();

        assert!(error.to_string().contains("newer than this GMUS build"));
    }

    #[test]
    fn migrate_rejects_negative_schema_version_without_mutation() {
        let conn = migration_test_connection();
        conn.pragma_update(None, "user_version", -1).unwrap();

        let error = migrate(&conn).unwrap_err();

        assert!(error.to_string().contains("schema version -1 is invalid"));
        assert_eq!(user_version(&conn).unwrap(), -1);
        assert_eq!(count(&conn, "sqlite_schema").unwrap(), 0);
        assert!(foreign_keys_enabled(&conn));
    }

    #[test]
    fn migration_is_idempotent_after_reaching_latest_version() {
        let conn = migration_test_connection();
        migrate(&conn).unwrap();
        let schema_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row.get(0))
            .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            schema_before
        );
        assert!(foreign_keys_enabled(&conn));
    }

    #[test]
    fn migration_rolls_back_schema_and_version_after_late_failure() {
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO media_items (
                id, fingerprint, title, first_seen_at, updated_at
            ) VALUES (1, 'legacy', 'Legacy', 1, 1);
            CREATE INDEX idx_media_items_duplicate_key ON locations(path);
            "#,
        )
        .unwrap();

        let error = migrate(&conn).unwrap_err();

        assert!(error.to_string().contains("already exists"));
        assert_eq!(user_version(&conn).unwrap(), 1);
        assert!(table_has_column(&conn, "media_items", "fingerprint"));
        assert!(!table_has_column(&conn, "media_items", "duplicate_key"));
        assert_eq!(count(&conn, "media_items").unwrap(), 1);
        assert!(foreign_keys_enabled(&conn));
    }

    #[test]
    fn migration_splits_legacy_multiple_present_locations_deterministically() {
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO media_items (
                id, fingerprint, title, artist, album, cover_path, track_number, duration_ms,
                first_seen_at, updated_at
            ) VALUES (1, 'same', 'Same Track', 'Artist', 'Album', '/tmp/1.jpg', 1, 120000, 1, 1);
            INSERT INTO locations (
                id, media_item_id, path, file_size, modified_at, seen_at, missing
            ) VALUES
                (1, 1, '/tmp/music/one.flac', 10, 1, 1, 0),
                (2, 1, '/tmp/music/two.flac', 10, 1, 1, 0),
                (3, 1, '/tmp/music/old.flac', 10, 1, 1, 1);
            INSERT INTO play_events (
                id, media_item_id, location_id, played_at, duration_ms, completed
            ) VALUES
                (1, 1, 1, 10, 100, 1),
                (2, 1, 2, 20, 50, 0),
                (3, 1, NULL, 30, 25, 1);
            INSERT INTO media_stats (
                media_item_id, play_count, last_played_at, total_play_ms, skip_count
            ) VALUES (1, 7, 99, 700, 3);
            INSERT INTO playlists (id, name, created_at, updated_at)
            VALUES (1, 'Mix', 1, 1);
            INSERT INTO playlist_tracks (id, playlist_id, media_item_id, position, added_at)
            VALUES (1, 1, 1, 0, 1);
            INSERT INTO app_browser_selection (
                id, tree_kind, media_item_id, updated_at
            ) VALUES (1, 'track', 1, 1);
            "#,
        )
        .unwrap();

        migrate(&conn).unwrap();

        let second_media_item_id: i64 = conn
            .query_row(
                "SELECT media_item_id FROM locations WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(second_media_item_id, 1);
        assert_eq!(
            conn.query_row(
                "SELECT cover_path FROM media_items WHERE id = ?1",
                params![second_media_item_id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
        assert_eq!(
            conn.query_row(
                "SELECT media_item_id FROM locations WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT media_item_id FROM locations WHERE id = 3",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT media_item_id FROM play_events WHERE location_id = 2",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            second_media_item_id
        );
        assert_eq!(media_stats_row(&conn, 1).unwrap().unwrap().play_count, 7);
        assert_eq!(
            media_stats_row(&conn, second_media_item_id)
                .unwrap()
                .unwrap()
                .skip_count,
            1
        );
        assert_eq!(
            conn.query_row(
                "SELECT SUM(play_count), MAX(last_played_at), SUM(total_play_ms), SUM(skip_count) FROM media_stats",
                [],
                |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            )
            .unwrap(),
            (7, 99, 700, 3)
        );
        assert_eq!(playlist_track_ids(&conn, 1).unwrap(), vec![1]);
        assert_eq!(
            browser_selection(&conn).unwrap().unwrap().media_item_id,
            Some(1)
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM media_items WHERE duplicate_key = 'same'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert!(foreign_key_violation(&conn).is_none());
        assert!(foreign_keys_enabled(&conn));
        assert_eq!(integrity_check(&conn), "ok");
        assert!(conn
            .execute(
                "INSERT INTO locations (media_item_id, path, seen_at, missing) VALUES (1, '/tmp/music/third.flac', 1, 0)",
                [],
            )
            .is_err());
    }

    #[test]
    fn migration_repairs_sparse_version_one_schema_before_exact_upgrade() {
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE app_browser_selection;
            DROP TABLE app_key_bindings;
            DROP TABLE app_settings;
            DROP TABLE app_filter_state;
            "#,
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert!(table_has_column(
            &conn,
            "app_browser_selection",
            "media_item_id"
        ));
        assert!(table_has_column(&conn, "app_key_bindings", "key"));
        assert!(table_has_column(&conn, "app_settings", "value"));
        assert!(table_has_column(&conn, "app_filter_state", "filter"));
        assert!(foreign_key_violation(&conn).is_none());
    }

    #[test]
    fn file_backed_version_one_fixture_reopens_at_latest_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gmus.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            load_original_v1_schema(&conn);
            conn.execute_batch(
                r#"
                INSERT INTO media_items (
                    id, fingerprint, title, cover_path, first_seen_at, updated_at
                ) VALUES (1, 'legacy', 'Legacy', '/tmp/shared-art/1.jpg', 1, 1);
                INSERT INTO locations (
                    id, media_item_id, path, seen_at, missing
                ) VALUES (1, 1, '/tmp/legacy.flac', 1, 0);
                "#,
            )
            .unwrap();
        }

        let conn = open(&path).unwrap();

        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].file_path, Path::new("/tmp/legacy.flac"));
        assert!(table_has_column(&conn, "media_items", "duplicate_key"));
        assert!(table_has_column(&conn, "locations", "fs_device"));
        assert!(table_has_column(&conn, "locations", "fs_inode"));
        assert!(table_has_column(&conn, "locations", "modified_at_ns"));
        assert!(table_has_column(&conn, "locations", "scan_version"));
        assert!(table_has_column(&conn, "locations", "folder_art_signature"));
        assert_eq!(
            tracks[0].cover_path.as_deref(),
            Some(Path::new("/tmp/shared-art/1.jpg"))
        );
        assert_eq!(
            conn.query_row(
                "SELECT fs_device, fs_inode, modified_at_ns, scan_version, folder_art_signature FROM locations WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                }
            )
            .unwrap(),
            (None, None, None, None, None)
        );
        assert!(foreign_keys_enabled(&conn));
        assert_eq!(integrity_check(&conn), "ok");
    }

    #[test]
    fn unversioned_historical_schema_upgrades_to_latest_version() {
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.pragma_update(None, "user_version", 0).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO media_items (
                id, fingerprint, title, first_seen_at, updated_at
            ) VALUES (1, 'legacy', 'Legacy', 1, 1);
            INSERT INTO locations (
                id, media_item_id, path, seen_at, missing
            ) VALUES (1, 1, '/tmp/legacy.flac', 1, 0);
            "#,
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(library_tracks(&conn).unwrap().len(), 1);
        assert!(foreign_key_violation(&conn).is_none());
    }

    #[test]
    fn concurrent_file_backed_migrations_serialize() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("gmus.sqlite3");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            load_original_v1_schema(&conn);
        }
        let barrier = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    let conn = open(&path).unwrap();
                    assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
                    assert_eq!(integrity_check(&conn), "ok");
                })
            })
            .collect::<Vec<_>>();

        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }

        let conn = open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        assert!(foreign_key_violation(&conn).is_none());
    }

    #[test]
    fn art_cache_namespace_migrates_covers_and_invalidates_scan_state() {
        let dir = tempdir().unwrap();
        let legacy_dir = dir.path().join("legacy-art");
        let first_dir = dir.path().join("first-art");
        let second_dir = dir.path().join("second-art");
        fs::create_dir_all(&legacy_dir).unwrap();
        let legacy_cover = legacy_dir.join("cover.jpg");
        fs::write(&legacy_cover, b"cover").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let track = test_track_metadata("/tmp/music/song.flac", "Song", 1, 120_000);
        let stored = upsert_track(&conn, &track).unwrap();
        set_cover_path(&conn, stored.media_item_id, &legacy_cover).unwrap();
        mark_location_scan_current_for_mode(&conn, &track.path, None, ArtworkMode::Cached).unwrap();

        prepare_art_cache_with_mode(&conn, &first_dir, ArtworkMode::Cached).unwrap();

        let first_cover = first_dir
            .canonicalize()
            .unwrap()
            .join(format!("{}.jpg", stored.media_item_id));
        assert_eq!(fs::read(&first_cover).unwrap(), b"cover");
        assert_eq!(
            library_tracks(&conn).unwrap()[0].cover_path.as_deref(),
            Some(first_cover.as_path())
        );
        assert_eq!(
            conn.query_row(
                "SELECT scan_version FROM locations WHERE id = ?1",
                params![stored.location_id],
                |row| row.get::<_, Option<i64>>(0)
            )
            .unwrap(),
            None
        );

        mark_location_scan_current_for_mode(&conn, &track.path, None, ArtworkMode::Cached).unwrap();
        prepare_art_cache_with_mode(&conn, &first_dir, ArtworkMode::Cached).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT scan_version FROM locations WHERE id = ?1",
                params![stored.location_id],
                |row| row.get::<_, Option<i64>>(0)
            )
            .unwrap(),
            Some(ArtworkMode::Cached.scan_version())
        );

        prepare_art_cache_with_mode(&conn, &second_dir, ArtworkMode::Cached).unwrap();
        let second_cover = second_dir
            .canonicalize()
            .unwrap()
            .join(format!("{}.jpg", stored.media_item_id));
        assert_eq!(fs::read(&second_cover).unwrap(), b"cover");
        assert_eq!(
            library_tracks(&conn).unwrap()[0].cover_path.as_deref(),
            Some(second_cover.as_path())
        );
        assert_eq!(
            conn.query_row(
                "SELECT scan_version FROM locations WHERE id = ?1",
                params![stored.location_id],
                |row| row.get::<_, Option<i64>>(0)
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn disabled_art_cache_clears_cached_paths_and_files() {
        let dir = tempdir().unwrap();
        let art_dir = dir.path().join("art");
        let external_dir = dir.path().join("album");
        fs::create_dir_all(&art_dir).unwrap();
        fs::create_dir_all(&external_dir).unwrap();
        let cached_cover = art_dir.join("1.jpg");
        let stale_cover = art_dir.join("stale.png");
        let nested_dir = art_dir.join("nested");
        let nested_cover = nested_dir.join("old.jpg");
        let external_cover = external_dir.join("cover.jpg");
        fs::write(&cached_cover, b"cached").unwrap();
        fs::write(&stale_cover, b"stale").unwrap();
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(&nested_cover, b"nested").unwrap();
        fs::write(&external_cover, b"external").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let cached = test_track_metadata("/tmp/music/cached.flac", "Cached", 1, 120_000);
        let external = test_track_metadata("/tmp/music/external.flac", "External", 2, 120_000);
        let cached_stored = upsert_track(&conn, &cached).unwrap();
        let external_stored = upsert_track(&conn, &external).unwrap();
        set_cover_path(&conn, cached_stored.media_item_id, &cached_cover).unwrap();
        set_cover_path(&conn, external_stored.media_item_id, &external_cover).unwrap();

        prepare_art_cache_with_mode(&conn, &art_dir, ArtworkMode::OnDemand).unwrap();

        assert!(!cached_cover.exists());
        assert!(!stale_cover.exists());
        assert!(!nested_dir.exists());
        let tracks = library_tracks(&conn).unwrap();
        assert!(tracks
            .iter()
            .find(|track| track.media_item_id == cached_stored.media_item_id)
            .unwrap()
            .cover_path
            .is_none());
        assert_eq!(
            tracks
                .iter()
                .find(|track| track.media_item_id == external_stored.media_item_id)
                .unwrap()
                .cover_path
                .as_deref(),
            Some(external_cover.as_path())
        );
        assert_eq!(fs::read(&external_cover).unwrap(), b"external");
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
    fn migrate_key_bindings_preserves_existing_rows_and_allows_multiple_keys() {
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE app_key_bindings;
            CREATE TABLE app_key_bindings (
                action          TEXT PRIMARY KEY,
                key             TEXT NOT NULL,
                updated_at      INTEGER NOT NULL
            );
            INSERT INTO app_key_bindings (action, key, updated_at)
            VALUES ('toggle-info', 'none:char:i', 1);
            "#,
        )
        .unwrap();

        migrate(&conn).unwrap();
        save_key_binding(
            &conn,
            &SavedKeyBinding {
                action: "toggle-info".to_string(),
                key: "none:char:o".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            key_bindings(&conn).unwrap(),
            vec![
                SavedKeyBinding {
                    action: "toggle-info".to_string(),
                    key: "none:char:i".to_string(),
                },
                SavedKeyBinding {
                    action: "toggle-info".to_string(),
                    key: "none:char:o".to_string(),
                },
            ]
        );
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: None,
            fs_inode: None,
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

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_and_roots_round_trip_without_lossy_collisions() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let root = std::path::PathBuf::from(OsString::from_vec(b"/tmp/music-\xff".to_vec()));
        let first_path = root.join(OsString::from_vec(b"\xfe.flac".to_vec()));
        let second_path = root.join(OsString::from_vec(b"\xff.flac".to_vec()));
        let mut first = test_track_metadata("/tmp/placeholder.flac", "First", 1, 120_000);
        first.path = first_path.clone();
        let mut second = first.clone();
        second.path = second_path.clone();
        second.title = Some("Second".into());
        second.fs_inode = Some(2);

        upsert_track(&conn, &first).unwrap();
        upsert_track(&conn, &second).unwrap();
        upsert_library_root(&conn, &root).unwrap();

        let tracks = library_tracks(&conn).unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().any(|track| track.file_path == first_path));
        assert!(tracks.iter().any(|track| track.file_path == second_path));
        let roots = active_library_roots(&conn).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].file_path, root);
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
        let conn = migration_test_connection();
        load_original_v1_schema(&conn);
        conn.execute_batch(
            r#"
            INSERT INTO media_items (
                id, fingerprint, title, artist, album, track_number, duration_ms,
                first_seen_at, updated_at
            ) VALUES (1, 'one', 'One', 'Artist', 'Album', 1, 120000, 1, 1);
            INSERT INTO locations (
                id, media_item_id, path, file_size, modified_at, seen_at, missing
            ) VALUES (1, 1, '/tmp/music/one.flac', 10, 1, 1, 0);
            INSERT INTO playlists (id, name, created_at, updated_at)
            VALUES (1, 'Mix', 1, 1);

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
            params![1, 1],
        )
        .unwrap();

        migrate(&conn).unwrap();
        let added = add_tracks_to_playlist(&conn, 1, &[1]).unwrap();

        assert_eq!(added, 1);
        assert_eq!(playlist_track_ids(&conn, 1).unwrap(), vec![1, 1]);
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
    fn rename_reconciliation_combines_play_counts() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/wrong-name.flac", "Same Track", 1, 120_000);
        let renamed = test_track_metadata("/tmp/music/right-name.flac", "Same Track", 1, 121_000);

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

        let merged = reconcile_renamed_media_items(&conn).unwrap();
        let tracks = library_tracks(&conn).unwrap();

        assert_eq!(merged, 1);
        assert_eq!(stats(&conn).unwrap().media_items, 1);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, "/tmp/music/right-name.flac");
        assert_eq!(tracks[0].play_count, 1);
    }

    #[test]
    fn rename_reconciliation_preserves_playlist_entries() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/wrong-name.flac", "Same Track", 1, 120_000);
        let mut renamed =
            test_track_metadata("/tmp/music/right-name.flac", "Same Track", 1, 121_000);
        renamed.artist = Some("artist".to_string());

        let old_stored = upsert_track(&conn, &old).unwrap();
        let playlist = create_playlist(&conn, "Mix").unwrap();
        add_tracks_to_playlist(&conn, playlist.id, &[old_stored.media_item_id]).unwrap();
        set_cover_path(
            &conn,
            old_stored.media_item_id,
            Path::new("/tmp/old-id-cover.jpg"),
        )
        .unwrap();
        save_browser_selection(
            &conn,
            &SavedBrowserSelection {
                tree_kind: "track".to_string(),
                artist: Some("Artist".to_string()),
                album: Some("Album".to_string()),
                playlist_id: None,
                media_item_id: Some(old_stored.media_item_id),
            },
        )
        .unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        let renamed_stored = upsert_track(&conn, &renamed).unwrap();

        let merged = reconcile_renamed_media_items(&conn).unwrap();

        assert_eq!(merged, 1);
        assert_eq!(
            playlist_track_ids(&conn, playlist.id).unwrap(),
            vec![renamed_stored.media_item_id]
        );
        let selection = browser_selection(&conn).unwrap().unwrap();
        assert_eq!(selection.media_item_id, Some(renamed_stored.media_item_id));
        assert_eq!(selection.artist.as_deref(), Some("artist"));
        assert_eq!(selection.album.as_deref(), Some("Album"));
        assert_eq!(
            conn.query_row(
                "SELECT cover_path FROM media_items WHERE id = ?1",
                params![renamed_stored.media_item_id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
            Some(path::encode(Path::new("/tmp/old-id-cover.jpg")))
        );
    }

    #[test]
    fn rename_reconciliation_refuses_groups_with_multiple_present_candidates() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "Same Track", 1, 120_000);
        let second = test_track_metadata("/tmp/music/second.flac", "Same Track", 1, 121_000);

        upsert_track(&conn, &first).unwrap();
        upsert_track(&conn, &second).unwrap();

        let merged = reconcile_renamed_media_items(&conn).unwrap();

        assert_eq!(merged, 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
        assert_eq!(library_tracks(&conn).unwrap().len(), 2);
    }

    #[test]
    fn matching_duplicate_keys_never_merge_distinct_present_paths() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "Same Track", 1, 120_000);
        let mut second = first.clone();
        second.path = "/tmp/music/second.flac".into();

        let first_stored = upsert_track(&conn, &first).unwrap();
        let second_stored = upsert_track(&conn, &second).unwrap();
        let merged = reconcile_renamed_media_items(&conn).unwrap();
        second.track_number = Some(2);
        let retagged_second = upsert_track(&conn, &second).unwrap();
        let tracks = library_tracks(&conn).unwrap();

        assert_ne!(first_stored.media_item_id, second_stored.media_item_id);
        assert_eq!(second_stored.media_item_id, retagged_second.media_item_id);
        assert_eq!(merged, 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.track_number)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
    }

    #[test]
    fn rename_reconciliation_requires_matching_filesystem_identity() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "Same Track", 1, 120_000);
        let mut second = first.clone();
        second.path = "/tmp/music/second.flac".into();
        second.fs_inode = Some(2);

        let first_stored = upsert_track(&conn, &first).unwrap();
        let second_stored = upsert_track(&conn, &second).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        upsert_track(&conn, &first).unwrap();

        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
        assert_ne!(first_stored.media_item_id, second_stored.media_item_id);
    }

    #[test]
    fn rename_reconciliation_rejects_reused_identity_with_different_file_signature() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "Same Track", 1, 120_000);
        let mut second = first.clone();
        second.path = "/tmp/music/second.flac".into();
        second.modified_at_ns = Some(1_000_000_001);

        upsert_track(&conn, &first).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        upsert_track(&conn, &second).unwrap();

        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
    }

    #[test]
    fn rename_reconciliation_does_not_guess_legacy_rename_without_filesystem_identity() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut old = test_track_metadata("/tmp/music/old.flac", "Same Track", 1, 120_000);
        old.fs_device = None;
        old.fs_inode = None;
        let mut renamed = old.clone();
        renamed.path = "/tmp/music/renamed.flac".into();

        upsert_track(&conn, &old).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        upsert_track(&conn, &renamed).unwrap();

        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
    }

    #[test]
    fn reappearing_historical_path_detaches_from_still_present_rename() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/old.flac", "Same Track", 1, 120_000);
        let mut renamed = old.clone();
        renamed.path = "/tmp/music/renamed.flac".into();

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
        let renamed_stored = upsert_track(&conn, &renamed).unwrap();
        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 1);
        set_cover_path(
            &conn,
            renamed_stored.media_item_id,
            Path::new("/tmp/renamed-cover.jpg"),
        )
        .unwrap();

        let reappeared_stored = upsert_track(&conn, &old).unwrap();

        assert_ne!(
            reappeared_stored.media_item_id,
            renamed_stored.media_item_id
        );
        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 0);
        assert_eq!(library_tracks(&conn).unwrap().len(), 2);
        assert_eq!(
            media_stats_row(&conn, renamed_stored.media_item_id)
                .unwrap()
                .unwrap()
                .play_count,
            1
        );
        assert_eq!(
            media_stats_row(&conn, reappeared_stored.media_item_id)
                .unwrap()
                .unwrap()
                .play_count,
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT media_item_id, location_id FROM play_events",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
            )
            .unwrap(),
            (renamed_stored.media_item_id, None)
        );
        assert_eq!(
            conn.query_row(
                "SELECT cover_path FROM media_items WHERE id = ?1",
                params![reappeared_stored.media_item_id],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn rename_reconciliation_refuses_groups_without_a_present_candidate() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "Same Track", 1, 120_000);
        let second = test_track_metadata("/tmp/music/second.flac", "Same Track", 1, 121_000);

        upsert_track(&conn, &first).unwrap();
        upsert_track(&conn, &second).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();

        let merged = reconcile_renamed_media_items(&conn).unwrap();

        assert_eq!(merged, 0);
        assert_eq!(stats(&conn).unwrap().media_items, 2);
        assert!(library_tracks(&conn).unwrap().is_empty());
    }

    #[test]
    fn record_play_resolves_media_item_after_rename_reconciliation() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let old = test_track_metadata("/tmp/music/old.flac", "Same Track", 1, 120_000);
        let mut renamed = old.clone();
        renamed.path = "/tmp/music/renamed.flac".into();
        let old_stored = upsert_track(&conn, &old).unwrap();
        mark_locations_missing_under_root(&conn, Path::new("/tmp/music")).unwrap();
        let renamed_stored = upsert_track(&conn, &renamed).unwrap();
        assert_eq!(reconcile_renamed_media_items(&conn).unwrap(), 1);

        record_play(
            &conn,
            old_stored.media_item_id,
            old_stored.location_id,
            120_000,
            true,
        )
        .unwrap();

        assert_eq!(stats(&conn).unwrap().play_events, 1);
        assert_eq!(
            media_stats_row(&conn, renamed_stored.media_item_id)
                .unwrap()
                .unwrap()
                .play_count,
            1
        );
    }

    #[test]
    fn record_play_preserves_existing_media_when_location_is_reassigned() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = test_track_metadata("/tmp/music/first.flac", "First Track", 1, 120_000);
        let second = test_track_metadata("/tmp/music/second.flac", "Second Track", 2, 120_000);
        let first_stored = upsert_track(&conn, &first).unwrap();
        let second_stored = upsert_track(&conn, &second).unwrap();
        conn.execute(
            "UPDATE locations SET missing = 1 WHERE id = ?1",
            params![second_stored.location_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE locations SET media_item_id = ?1 WHERE id = ?2",
            params![second_stored.media_item_id, first_stored.location_id],
        )
        .unwrap();

        let recorded_media_item_id = record_play(
            &conn,
            first_stored.media_item_id,
            first_stored.location_id,
            120_000,
            true,
        )
        .unwrap();

        assert_eq!(recorded_media_item_id, first_stored.media_item_id);
        assert_eq!(
            media_stats_row(&conn, first_stored.media_item_id)
                .unwrap()
                .unwrap()
                .play_count,
            1
        );
        assert_eq!(
            media_stats_row(&conn, second_stored.media_item_id)
                .unwrap()
                .unwrap()
                .play_count,
            0
        );
        assert_eq!(
            conn.query_row("SELECT location_id FROM play_events", [], |row| row
                .get::<_, Option<i64>>(0))
                .unwrap(),
            None
        );
    }

    #[test]
    fn upsert_track_preserves_identity_and_history_when_same_path_metadata_changes() {
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

        assert_eq!(original_stored.media_item_id, retagged_stored.media_item_id);
        assert_eq!(original_stored.location_id, retagged_stored.location_id);
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

    fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let found = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .any(|name| name.unwrap() == column);
        found
    }

    fn foreign_key_violation(conn: &Connection) -> Option<(String, i64, String)> {
        conn.query_row("PRAGMA foreign_key_check", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()
        .unwrap()
    }

    fn migration_test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    fn load_original_v1_schema(conn: &Connection) {
        conn.execute_batch(include_str!("db/test_fixtures/v1.sql"))
            .unwrap();
    }

    fn foreign_keys_enabled(conn: &Connection) -> bool {
        conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap()
    }

    fn integrity_check(conn: &Connection) -> String {
        conn.pragma_query_value(None, "integrity_check", |row| row.get(0))
            .unwrap()
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
            modified_at_ns: Some(1_000_000_000),
            fs_device: Some(1),
            fs_inode: Some(track_number),
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
