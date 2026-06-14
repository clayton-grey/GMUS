use super::*;

#[test]
fn playlist_panel_opens_and_adds_selected_track() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.focus = FocusPane::Tracks;
    app.browser.select_track_row(2);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.focus, FocusPane::Playlist);
    assert!(pane_active(&app, FocusPane::Playlist));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.focus, FocusPane::Tree);
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Tracks);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(0));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(1));
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();

    let playlist_id = app.management_panel.playlist.active_playlist_id().unwrap();
    assert!(app.management_panel.playlist_open());
    assert_eq!(db::playlist_track_ids(&conn, playlist_id).unwrap(), vec![2]);
    assert!(playlist_text(&app).contains("second track"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Playlist);
    assert!(pane_active(&app, FocusPane::Playlist));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.management_panel.playlist.selected_row(), 1);
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.focus, FocusPane::Playlist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.management_panel.playlist_open());
    assert!(app.layout.info_panel_visible());
}

#[test]
fn enter_on_playlist_panel_header_plays_first_playlist_track() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(&conn, playlist.id, &[1]).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(0));
    assert_eq!(
        app.current.as_ref().and_then(|current| current.source),
        Some(PlaybackSource::PlaylistTrack {
            playlist_id: playlist.id,
            playlist_track_id: app
                .playlist_cache
                .playable_entries(playlist.id)
                .next()
                .unwrap()
                .playlist_track_id
        })
    );
    assert!(!app.management_panel.playlist.playlist_expanded(playlist.id));
}

#[test]
fn playlist_cache_counts_unavailable_entries_but_only_exposes_playable_tracks() {
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
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(
        &conn,
        playlist.id,
        &[first.media_item_id, second.media_item_id],
    )
    .unwrap();
    let mut app = test_app(vec![test_track(first.media_item_id, "first track")]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel.playlist.expand_playlist(playlist.id);
    app.sync_selection();

    assert_eq!(app.playlist_cache.len(playlist.id), 2);
    assert_eq!(app.playlist_cache.playable_entries(playlist.id).count(), 1);
    assert_eq!(app.view.playlist_entries.len(), 2);
    assert!(playlist_entry_text(&app, &app.view.playlist_entries[0]).contains("Mix (2)"));
}

#[test]
fn space_on_playlist_panel_header_still_toggles_expansion() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(7));
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();

    app.space_action();

    assert!(app.management_panel.playlist.playlist_expanded(7));
    assert!(app.current.is_none());
}

#[test]
fn playlist_hotkeys_do_not_edit_when_playlist_panel_is_closed() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(&conn, playlist.id, &[1]).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.focus = FocusPane::Tracks;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.management_panel.playlist_open());
    assert_eq!(db::playlist_track_ids(&conn, playlist.id).unwrap(), vec![1]);
    assert_eq!(
        app.message,
        "open playlist panel with p before editing playlists"
    );
}

#[test]
fn playlist_add_hotkey_does_not_create_default_playlist_when_panel_is_closed() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.focus = FocusPane::Tracks;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();

    assert!(db::playlists(&conn).unwrap().is_empty());
    assert_eq!(
        app.message,
        "open playlist panel with p before editing playlists"
    );
}

#[test]
fn playlist_panel_removes_selected_playlist_track() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(&conn, playlist.id, &[1]).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.management_panel.playlist.expand_playlist(playlist.id);
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.management_panel.playlist.select_row(1);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert!(db::playlist_track_ids(&conn, playlist.id)
        .unwrap()
        .is_empty());
}

#[test]
fn playlist_track_numbers_are_playlist_relative() {
    let mut first = test_track(1, "first track");
    first.track_number = Some(7);
    let mut second = test_track(2, "second track");
    second.track_number = Some(9);
    let mut app = test_app(vec![first, second]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1, 2], vec![11, 12], vec![0, 1]);
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(7));
    app.management_panel.playlist.expand_playlist(7);
    app.management_panel.show_playlist();
    app.sync_selection();

    let text = playlist_text(&app);

    assert!(text.contains("01. Artist - first track"));
    assert!(text.contains("02. Artist - second track"));
    assert!(!text.contains("07. Artist - first track"));
    assert!(!text.contains("09. Artist - second track"));
}

#[test]
fn collapsing_playlist_panel_from_track_selects_playlist_parent() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(7));
    app.management_panel.playlist.expand_playlist(7);
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.management_panel.playlist.select_row(1);

    app.space_action();

    assert!(!app.management_panel.playlist.playlist_expanded(7));
    assert_eq!(app.management_panel.playlist.selected_row(), 0);
    assert!(matches!(
        app.view
            .playlist_entries
            .get(app.management_panel.playlist.selected_row()),
        Some(PlaylistPanelEntry::Playlist { playlist_id: 7, .. })
    ));
}

#[test]
fn playlist_panel_removes_exact_duplicate_entry() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(&conn, playlist.id, &[1, 2, 1]).unwrap();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.management_panel.playlist.expand_playlist(playlist.id);
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.management_panel.playlist.select_row(1);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        db::playlist_track_ids(&conn, playlist.id).unwrap(),
        vec![2, 1]
    );
}

#[test]
fn track_remove_deletes_most_recent_playlist_duplicate() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    db::add_tracks_to_playlist(&conn, playlist.id, &[1, 2, 1]).unwrap();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.management_panel.show_playlist();
    app.focus = FocusPane::Tracks;
    app.sync_selection();
    app.browser.select_track_row(0);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        db::playlist_track_ids(&conn, playlist.id).unwrap(),
        vec![1, 2]
    );
}

#[test]
fn artist_add_and_remove_apply_to_all_artist_tracks() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/second.flac", "second track", 2),
    )
    .unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/other.flac", "other track", 3),
    )
    .unwrap();
    let playlist = db::create_playlist(&conn, "Mix").unwrap();
    let mut other_artist = test_track(3, "other track");
    other_artist.artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
        other_artist,
    ]);
    app.playlists = db::playlists(&conn).unwrap();
    app.refresh_playlist_tracks(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(playlist.id));
    app.management_panel.show_playlist();
    app.sync_selection();
    app.focus = FocusPane::Tree;
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Artist { artist } if artist == "Artist"))
        .unwrap();
    app.browser.select_tree(selected_tree);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        db::playlist_track_ids(&conn, playlist.id).unwrap(),
        vec![1, 2, 1, 2]
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert!(db::playlist_track_ids(&conn, playlist.id)
        .unwrap()
        .is_empty());
}

#[test]
fn mouse_scroll_moves_playlist_pane_without_changing_focus() {
    let mut app = test_app(
        (1..=6)
            .map(|id| test_track(id, &format!("track {id}")))
            .collect(),
    );
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(
        &mut app,
        7,
        (1..=6).collect(),
        (11..=16).collect(),
        (0..6).collect(),
    );
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(7));
    app.management_panel.playlist.expand_playlist(7);
    app.management_panel.show_playlist();
    app.focus = FocusPane::Tracks;
    app.sync_selection();

    let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 60, 17), 100, 30);

    assert!(handled);
    assert_eq!(app.focus, FocusPane::Tracks);
    assert_eq!(app.management_panel.playlist.selected_row(), 1);
}

#[test]
fn mouse_hit_testing_allows_playlist_info_pane() {
    assert_eq!(
        mouse_pane(
            60,
            17,
            MouseLayout::new(100, 30, 2)
                .with_info(true, false)
                .with_playlist_info(true)
        ),
        Some(FocusPane::Playlist)
    );
    assert_eq!(
        mouse_pane(60, 17, MouseLayout::new(100, 30, 2).with_info(true, false)),
        None
    );
}

#[test]
fn playlist_list_keeps_selection_padded_from_bottom_when_possible() {
    let mut app = test_app(Vec::new());
    app.playlists = (0..20)
        .map(|id| db::Playlist {
            id,
            name: format!("List {id:02}"),
        })
        .collect();
    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.management_panel.playlist.select_row(10);
    app.apply_selection_state();
    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_playlist_info_pane(frame, &mut app, Rect::new(0, 0, 80, 10), 78))
        .unwrap();

    assert!(app.playlist_state.offset() > 0);
    assert!(app.management_panel.playlist.selected_row() - app.playlist_state.offset() <= 4);
}
