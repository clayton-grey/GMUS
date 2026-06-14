use super::*;

#[test]
fn app_start_restores_pane_layout_offsets() {
    let conn = test_conn();
    db::save_pane_layout(
        &conn,
        db::SavedPaneLayout {
            library_percent_offset: 4,
            info_height_offset: 3,
        },
    )
    .unwrap();
    db::save_column_layout_width(&conn, 96).unwrap();

    let app = test_app_from_db(&conn);

    assert_eq!(app.layout.library_pane_percent_offset(), 4);
    assert_eq!(app.layout.info_pane_height_offset(), 3);
    assert_eq!(app.layout.column_layout_width(), 96);
}

#[test]
fn restore_filter_command_toggles_persistent_setting() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.set_filter(String::from("artist:eno"));

    set_command_input(&mut app, String::from("restore-filter"));
    app.execute_command(&conn);

    assert!(!app.restore_filter);
    assert!(!db::restore_filter_enabled(&conn).unwrap());
    assert_eq!(app.message, "restore filter off");

    set_command_input(&mut app, String::from("restore-filter on"));
    app.execute_command(&conn);

    assert!(app.restore_filter);
    assert!(db::restore_filter_enabled(&conn).unwrap());
    assert_eq!(
        db::saved_filter(&conn).unwrap().as_deref(),
        Some("artist:eno")
    );
}

#[test]
fn restore_track_command_toggles_persistent_setting() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    set_command_input(&mut app, String::from("restore-track"));
    app.execute_command(&conn);

    assert!(!app.restore_track);
    assert!(!db::restore_track_enabled(&conn).unwrap());
    assert_eq!(app.message, "restore track off");

    set_command_input(&mut app, String::from("restore-track on"));
    app.execute_command(&conn);

    assert!(app.restore_track);
    assert!(db::restore_track_enabled(&conn).unwrap());
}

#[test]
fn saved_browser_selection_restores_album_and_track() {
    let first = test_track(1, "first track");
    let mut second = test_track(2, "second track");
    second.album = Some("Other Album".to_string());
    let mut app = test_app(vec![first, second]);

    let restored = app.restore_saved_browser_selection(&db::SavedBrowserSelection {
        tree_kind: "album".to_string(),
        artist: Some("Artist".to_string()),
        album: Some("Other Album".to_string()),
        playlist_id: None,
        media_item_id: Some(2),
    });

    assert!(restored);
    assert!(app.browser.artist_expanded("Artist"));
    assert!(matches!(
        app.selected_tree_entry(),
        Some(TreeEntry::Album { album, .. }) if album == "Other Album"
    ));
    assert_eq!(app.selected_playable_media_item_id(), Some(2));
}

#[test]
fn invalid_saved_browser_selection_resets_to_default_selection() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    let restored = app.restore_saved_browser_selection(&db::SavedBrowserSelection {
        tree_kind: "artist".to_string(),
        artist: Some("Artist".to_string()),
        album: None,
        playlist_id: None,
        media_item_id: Some(999),
    });

    assert!(!restored);
    assert_eq!(app.browser.selected_tree(), 0);
    assert_eq!(app.selected_playable_media_item_id(), Some(1));
}

#[test]
fn save_browser_selection_persists_selected_artist_and_track() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut other_artist = test_track(2, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);

    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Artist { artist } if artist == "Other Artist"))
        .unwrap();
    app.browser.select_tree(selected_tree);
    app.sync_selection();
    app.save_browser_selection(&conn).unwrap();

    let selection = db::browser_selection(&conn).unwrap().unwrap();
    assert_eq!(selection.tree_kind, "artist");
    assert_eq!(selection.artist.as_deref(), Some("Other Artist"));
    assert_eq!(selection.media_item_id, Some(2));
}

#[test]
fn playback_persists_last_played_track_for_restart() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let first = test_track(1, "first track");
    let mut second = test_track(2, "other artist track");
    second.artist = Some("Other Artist".to_string());
    second.album_artist = Some("Other Artist".to_string());
    second.album = Some("Other Album".to_string());
    let mut app = test_app(vec![first.clone(), second.clone()]);
    app.sync_selection();

    app.play_index(&conn, 1).unwrap();

    assert_eq!(app.focus, FocusPane::Tracks);
    assert_eq!(app.selected_playable_media_item_id(), Some(2));
    let selection = db::browser_selection(&conn).unwrap().unwrap();
    assert_eq!(selection.tree_kind, "artist");
    assert_eq!(selection.artist.as_deref(), Some("Other Artist"));
    assert_eq!(selection.album, None);
    assert_eq!(selection.media_item_id, Some(2));

    let mut restored = test_app(vec![first, second]);
    assert!(restored.restore_saved_browser_selection(&selection));
    restored.focus = FocusPane::Tracks;
    restored.apply_selection_state();
    assert!(matches!(
        restored.selected_tree_entry(),
        Some(TreeEntry::Artist { artist }) if artist == "Other Artist"
    ));
    assert_eq!(restored.selected_playable_media_item_id(), Some(2));
}

#[test]
fn app_start_restores_saved_track_when_enabled() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let second = db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    db::save_browser_selection(
        &conn,
        &db::SavedBrowserSelection {
            tree_kind: "artist".to_string(),
            artist: Some("Artist".to_string()),
            album: None,
            playlist_id: None,
            media_item_id: Some(second.media_item_id),
        },
    )
    .unwrap();

    let app = test_app_from_db(&conn);

    assert_eq!(app.focus, FocusPane::Tracks);
    assert_eq!(
        app.selected_playable_media_item_id(),
        Some(second.media_item_id)
    );
}

#[test]
fn app_start_ignores_saved_track_when_disabled() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let first = db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let second = db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    db::save_browser_selection(
        &conn,
        &db::SavedBrowserSelection {
            tree_kind: "artist".to_string(),
            artist: Some("Artist".to_string()),
            album: None,
            playlist_id: None,
            media_item_id: Some(second.media_item_id),
        },
    )
    .unwrap();
    db::save_restore_track_enabled(&conn, false).unwrap();

    let app = test_app_from_db(&conn);

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(
        app.selected_playable_media_item_id(),
        Some(first.media_item_id)
    );
}

#[test]
fn playback_does_not_track_restart_selection_when_disabled() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.restore_track = false;

    app.play_index(&conn, 1).unwrap();

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.selected_playable_media_item_id(), Some(1));
    assert_eq!(db::browser_selection(&conn).unwrap(), None);
}

#[test]
fn next_track_updates_restart_selection() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.play_next(&conn).unwrap();

    let selection = db::browser_selection(&conn).unwrap().unwrap();
    assert_eq!(selection.media_item_id, Some(2));
}

#[test]
fn shutdown_does_not_overwrite_restart_selection_with_browser_cursor() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let first = test_track(1, "first track");
    let mut second = test_track(2, "other artist track");
    second.artist = Some("Other Artist".to_string());
    second.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![first, second]);

    app.play_index(&conn, 1).unwrap();
    assert_eq!(app.selected_playable_media_item_id(), Some(2));
    app.shutdown(&conn).unwrap();

    let selection = db::browser_selection(&conn).unwrap().unwrap();
    assert_eq!(selection.media_item_id, Some(2));
}
