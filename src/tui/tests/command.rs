use super::*;

#[test]
fn command_mode_executes_library_commands() {
    let data_dir = tempdir().unwrap();
    let library_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(Vec::new());
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };

    set_command_input(&mut app, format!("add {}", library_dir.path().display()));
    app.execute_command(&conn);

    let roots = db::active_library_roots(&conn).unwrap();
    assert_eq!(roots.len(), 1);
    assert!(app.message.starts_with("added "));

    set_command_input(&mut app, String::from("library"));
    app.execute_command(&conn);
    assert!(app.message.contains(library_dir.path().to_str().unwrap()));
    assert!(app.command_output.is_focused());
    assert_eq!(app.command_output.kind(), CommandOutputKind::LibraryRoots);
    assert!(app.command_output.lines()[0].starts_with("library roots"));
    assert!(app.command_output.lines()[1].contains("[x]"));
    assert!(app.command_output.lines()[1].contains(library_dir.path().to_str().unwrap()));

    set_command_input(&mut app, format!("remove {}", library_dir.path().display()));
    app.execute_command(&conn);

    assert!(db::active_library_roots(&conn).unwrap().is_empty());
    assert!(app.message.starts_with("removed "));
}

#[test]
fn command_mode_executes_playlist_commands() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(Vec::new());

    set_command_input(&mut app, String::from("playlist Road"));
    app.execute_command(&conn);

    assert!(app.management_panel.playlist_open());
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Road");
    assert_eq!(
        app.management_panel.playlist.active_playlist_id(),
        Some(app.playlists[0].id)
    );

    set_command_input(&mut app, String::from("playlist-clear Road"));
    app.execute_command(&conn);
    assert!(app.message.starts_with("cleared 0 tracks from Road"));

    set_command_input(&mut app, String::from("playlist-delete Road"));
    app.execute_command(&conn);
    assert!(app.message.starts_with("deleted playlist Road"));
    assert!(app.playlists.is_empty());
}

#[test]
fn playlist_commands_move_focus_from_closed_keymap_pane() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(Vec::new());
    let playlist = db::create_playlist(&conn, "Road").unwrap();

    app.focus = FocusPane::Keymap;
    app.management_panel.show_keymap();
    app.command_playlist(&conn, "Road").unwrap();

    assert!(!app.management_panel.keymap_open());
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.focus, FocusPane::Playlist);

    app.focus = FocusPane::Keymap;
    app.management_panel.show_keymap();
    app.command_playlist_clear(&conn, "Road").unwrap();

    assert!(!app.management_panel.keymap_open());
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.focus, FocusPane::Playlist);
    assert_eq!(
        app.management_panel.playlist.active_playlist_id(),
        Some(playlist.id)
    );
}

#[test]
fn deleting_active_playlist_preserves_first_playlist_fallback() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let first = db::create_playlist(&conn, "First").unwrap();
    let active = db::create_playlist(&conn, "Active").unwrap();
    let last = db::create_playlist(&conn, "Last").unwrap();
    let mut app = test_app(Vec::new());
    app.playlists = db::playlists(&conn).unwrap();
    app.management_panel
        .playlist
        .set_active_playlist_id(Some(active.id));
    app.management_panel.show_playlist();
    app.sync_selection();
    let selected_row = app
        .view
        .playlist_entries
        .iter()
        .position(|entry| {
            matches!(
                entry,
                PlaylistPanelEntry::Playlist { playlist_id, .. } if *playlist_id == last.id
            )
        })
        .unwrap();
    app.management_panel.playlist.select_row(selected_row);

    app.command_playlist_delete(&conn, "Active").unwrap();

    assert_eq!(
        app.management_panel.playlist.active_playlist_id(),
        Some(first.id)
    );
    assert!(matches!(
        app.view.playlist_entries.get(app.management_panel.playlist.selected_row()),
        Some(PlaylistPanelEntry::Playlist { playlist_id, .. }) if *playlist_id == first.id
    ));
}

#[test]
fn rate_command_changes_and_reports_playback_rate() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    set_command_input(&mut app, String::from("rate 0.75"));
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "playback rate 0.75x");

    set_command_input(&mut app, String::from("rate"));
    app.execute_command(&conn);

    assert_eq!(app.message, "playback rate 0.75x");
}

#[test]
fn rate_command_accepts_percent_and_reset() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    set_command_input(&mut app, String::from("rate 125%"));
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 1.25);
    assert_eq!(app.message, "playback rate 1.25x");

    set_command_input(&mut app, String::from("rate 75"));
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "playback rate 0.75x");

    set_command_input(&mut app, String::from("rate reset"));
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 1.0);
    assert_eq!(app.message, "playback rate 1.00x");
}

#[test]
fn column_layout_width_command_persists_and_resets_layout_threshold() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    assert_eq!(app.layout.column_layout_width(), 75);

    set_command_input(&mut app, String::from("column-layout-width 92"));
    app.execute_command(&conn);

    assert_eq!(app.layout.column_layout_width(), 92);
    assert_eq!(db::column_layout_width(&conn, 75).unwrap(), 92);
    assert_eq!(app.message, "column layout width 92 (columns above 92)");

    set_command_input(&mut app, String::from("column-layout-width status"));
    app.execute_command(&conn);

    assert_eq!(app.message, "column layout width 92 (columns above 92)");

    set_command_input(&mut app, String::from("column-layout-width reset"));
    app.execute_command(&conn);

    assert_eq!(app.layout.column_layout_width(), 75);
    assert_eq!(db::column_layout_width(&conn, 75).unwrap(), 75);
}

#[test]
fn column_layout_width_command_rejects_invalid_values_and_old_names() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    for command in [
        "column-layout-width 0",
        "column-layout-width wide",
        "column-layout-width 65536",
    ] {
        set_command_input(&mut app, command.to_string());
        app.execute_command(&conn);

        assert_eq!(app.layout.column_layout_width(), 75);
        assert_eq!(
            app.message,
            "usage: :column-layout-width [WIDTH | reset | status]"
        );
    }

    for command in ["column-width 92", "columns 92", "layout-width 92"] {
        set_command_input(&mut app, command.to_string());
        app.execute_command(&conn);

        assert_eq!(app.layout.column_layout_width(), 75);
        assert_eq!(
            app.message,
            format!(
                "unknown command: {}",
                command.split_whitespace().next().unwrap()
            )
        );
    }
}

#[test]
fn rate_command_rejects_invalid_values_without_changing_rate() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());
    app.player.set_rate(0.75).unwrap();

    for command in ["rate 0", "rate 10", "rate 401", "rate NaN", "rate fast"] {
        set_command_input(&mut app, command.to_string());
        app.execute_command(&conn);

        assert_eq!(app.player.rate(), 0.75);
        assert_eq!(
            app.message,
            "usage: :rate [0.25..4.0 | 25..400 | 25%..400% | reset]"
        );
    }
}

#[test]
fn rate_hotkey_opens_rate_input_and_applies_percentage() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Rate);
    assert!(app.input_bar_visible());
    assert!(app.info_area_visible());
    assert_eq!(command_info_title(&app), "Rate");
    assert_eq!(line_text(&rate_line(&app, 30)), " rate: 0.75 or 75_");
    assert!(lines_text(&rate_info_lines(&app, 80, 8)).contains("75 means 75%"));

    for key in ['7', '5'] {
        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            .unwrap();
    }
    assert_eq!(line_text(&input_line(&app, 30)), " rate: 75_");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.input.rate().is_empty());
    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "playback rate 0.75x");
}

#[test]
fn invalid_rate_input_stays_open_and_escape_preserves_rate() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player.set_rate(0.75).unwrap();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    for key in ['5', '0', '0'] {
        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            .unwrap();
    }
    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Rate);
    assert_eq!(app.input.rate(), "500");
    assert_eq!(app.player.rate(), 0.75);
    assert!(lines_text(&rate_info_lines(&app, 80, 8)).contains("invalid rate"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.input.rate().is_empty());
    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "rate cancelled");
}

#[test]
fn rate_backend_failure_preserves_input_mode_and_value() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(FailingRatePlayer);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    for key in ['7', '5'] {
        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            .unwrap();
    }

    let error = app
        .handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap_err();

    assert_eq!(error.to_string(), "decoder refused rate change");
    assert_eq!(app.input.kind(), InputKind::Rate);
    assert_eq!(app.input.rate(), "75");
}

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
#[test]
fn command_mode_toggles_track_notifications() {
    let conn = test_conn();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = test_app(Vec::new());
    app.integration.backend = Box::new(RecordingIntegration {
        events: Rc::clone(&events),
    });

    set_command_input(&mut app, String::from("notifications off"));
    app.execute_command(&conn);

    assert!(!app.integration.track_notifications_visible);
    assert_eq!(app.message, "track notifications hidden");
    assert_eq!(
        events.borrow().as_slice(),
        &[IntegrationEvent::TrackNotificationsVisible(false)]
    );

    set_command_input(&mut app, String::from("notifications toggle"));
    app.execute_command(&conn);

    assert!(app.integration.track_notifications_visible);
    assert_eq!(app.message, "track notifications visible");
}

#[test]
fn library_command_focuses_root_list_and_toggles_roots() {
    let data_dir = tempdir().unwrap();
    let root_a = tempdir().unwrap();
    let root_b = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_library_root(&conn, root_a.path()).unwrap();
    db::upsert_library_root(&conn, root_b.path()).unwrap();
    let mut app = test_app(Vec::new());

    set_command_input(&mut app, String::from("library"));
    app.execute_command(&conn);

    assert!(app.command_output.is_focused());
    assert_eq!(app.command_output.kind(), CommandOutputKind::LibraryRoots);
    assert_eq!(app.command_output.roots().len(), 2);
    assert_eq!(app.command_output.selected_index(), 0);
    assert_eq!(command_info_title(&app), "Library");
    assert_eq!(
        command_info_lines(&app, 80, 10)[1].spans[0].style,
        pane_highlight_style(true)
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.command_output.selected_index(), 1);
    let toggled_path = app.command_output.roots()[1].path.clone();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    let roots = db::library_roots(&conn).unwrap();
    assert!(
        !roots
            .iter()
            .find(|root| root.path == toggled_path)
            .unwrap()
            .active
    );
    assert!(app.command_output.is_focused());
    assert_eq!(
        app.command_output.roots()[app.command_output.selected_index()].path,
        toggled_path
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .unwrap();

    let roots = db::library_roots(&conn).unwrap();
    assert!(
        roots
            .iter()
            .find(|root| root.path == toggled_path)
            .unwrap()
            .active
    );
}

#[test]
fn colon_opens_command_bar() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Command);
    assert!(app.input_bar_visible());
    assert_eq!(line_text(&input_line(&app, 20)), " :l_");
    assert_eq!(
        input_line(&app, 20).spans[1].style,
        Style::default().fg(Color::White).bg(Color::Blue)
    );
}

#[test]
fn command_escape_clears_an_active_filter() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.input.set_filter(String::from("keep"));
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.input.filter().is_empty());
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    assert_eq!(app.message, "filter cleared");
}

#[test]
fn library_output_renders_in_info_pane() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.show_command_output(vec![
        String::from("library roots (1 active / 1 total)"),
        String::from("[x] /tmp/music"),
    ]);

    let lines = command_info_lines(&app, 80, 10);

    assert!(app.command_output_visible());
    assert_eq!(app.command_output_height(), 2);
    assert_eq!(line_text(&lines[0]), " library roots (1 active / 1 total)");
    assert_eq!(line_text(&lines[1]), " [x] /tmp/music");
    assert_eq!(
        lines[0].spans[0].style,
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD)
    );
}

#[test]
fn command_help_lists_available_commands() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.enter_command();
    set_command_input(&mut app, String::from("library"));

    let text = lines_text(&command_info_lines(&app, 120, 10));

    assert!(text.contains("commands: add remove update library playlist"));
    assert!(text.contains("playlist-clear playlist-delete keymap keymap-reset"));
    assert!(text.contains("column-layout-width"));
    assert!(text.contains("restore-filter"));
    assert!(text.contains("restore-track"));
    assert!(text.contains("filter"));
    assert!(text.contains("clear"));
    assert!(text.contains("clear-output"));
    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    assert!(text.contains("notifications"));
    #[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
    assert!(!text.contains("notifications"));
    assert!(!text.contains(":library_"));
    assert_eq!(
        command_info_lines(&app, 120, 10)[0].spans[0].style,
        Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    );
}

#[test]
fn command_help_wraps_command_list() {
    let lines = command_help_lines(28, Style::default().fg(Color::Black).bg(Color::White));
    let text = lines_text(&lines);
    let command_lines: Vec<String> = lines
        .iter()
        .map(line_text)
        .filter(|line| line.contains("commands:") || line.starts_with("           "))
        .collect();

    assert!(command_lines.len() > 1);
    assert!(text.contains("commands: add remove"));
    assert!(text.contains("clear-output"));
    assert!(command_lines.iter().all(|line| display_width(line) <= 28));
}

#[test]
fn info_panel_toggle_preserves_command_info_overlay() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.layout.info_panel_visible());
    assert!(!app.info_area_visible());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.info_area_visible());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.input.kind(), InputKind::None);
    assert!(!app.layout.info_panel_visible());
    assert!(!app.info_area_visible());

    app.show_command_output(vec![String::from("library roots")]);
    assert!(app.info_area_visible());
    app.clear_command_output();
    assert!(!app.info_area_visible());
}

#[test]
fn escape_clears_command_output_before_filter() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.input.set_filter(String::from("keep"));
    app.show_command_output(vec![
        String::from("library roots"),
        String::from("[x] /tmp/music"),
    ]);
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.command_output.is_visible());
    assert_eq!(app.input.filter(), "keep");
    assert_eq!(app.playback_sequence_indices(), &[0]);
}

#[test]
fn normal_navigation_clears_command_output() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    let conn = test_conn();
    app.show_command_output(vec![
        String::from("library roots"),
        String::from("[x] /tmp/music"),
    ]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.command_output.is_visible());
}

#[test]
fn scan_commands_start_background_job_before_finishing() {
    let data_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };
    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.library_job.is_some());
    assert!(app.command_output.lines()[0].contains("working: :update"));
    assert!(app.command_output.lines()[1].contains("scanning files"));

    assert!(wait_for_library_job(&mut app, &conn));
}

#[test]
fn background_scan_completion_returns_to_idle() {
    let data_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(Vec::new());
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };
    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(wait_for_library_job(&mut app, &conn));
    assert!(app.library_job.is_none());
    assert_eq!(app.message, "no active library roots; use :add PATH");
}

#[test]
fn shutdown_joins_active_background_scan() {
    let data_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(Vec::new());
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };
    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.library_job.is_some());

    app.shutdown(&conn).unwrap();

    assert!(app.library_job.is_none());
}

#[test]
fn playlist_commands_run_while_background_scan_is_active() {
    let data_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(Vec::new());
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };
    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.library_job.is_some());

    app.input.enter_command();
    set_command_input(&mut app, String::from("playlist Road"));
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.library_job.is_some());
    assert!(app.management_panel.playlist_open());
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Road");

    assert!(wait_for_library_job(&mut app, &conn));
}

#[test]
fn second_scan_command_reports_active_background_scan() {
    let data_dir = tempdir().unwrap();
    let db_path = data_dir.path().join("gmus.sqlite3");
    let conn = db::open(&db_path).unwrap();
    let mut app = test_app(Vec::new());
    app.paths = AppPaths {
        data_dir: data_dir.path().to_path_buf(),
        db_path,
        art_dir: data_dir.path().join("art"),
    };
    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    app.input.enter_command();
    set_command_input(&mut app, String::from("update"));
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.library_job.is_some());
    assert_eq!(app.message, "scan already running: :update");
    assert!(wait_for_library_job(&mut app, &conn));
}

#[test]
fn tab_completes_command_names() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();
    app.input.enter_command();
    set_command_input(&mut app, String::from("lib"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.command(), "library ");
}

#[test]
fn tab_completes_filesystem_paths_for_add() {
    let parent = tempdir().unwrap();
    let music = parent.path().join("MusicRoot");
    fs::create_dir(&music).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();
    app.input.enter_command();
    set_command_input(&mut app, format!("add {}/Mu", parent.path().display()));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.input.command(),
        format!("add {}/MusicRoot/", parent.path().display())
    );
}

#[test]
fn tab_completes_active_roots_for_remove() {
    let data_dir = tempdir().unwrap();
    let library_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_library_root(&conn, library_dir.path()).unwrap();
    let root = library_dir.path().to_string_lossy();
    let prefix_len = root.len().saturating_sub(2);
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.enter_command();
    set_command_input(&mut app, format!("remove {}", &root[..prefix_len]));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.command(), format!("remove {root} "));
}
