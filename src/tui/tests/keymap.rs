use super::*;

#[test]
fn key_controls_match_cmus_style_bindings() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    assert!(!app
        .handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.logical_state(), PlaybackState::Playing);
    assert_eq!(
        app.current
            .as_ref()
            .map(|current| current.track.title.as_deref()),
        Some(Some("first track"))
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.playback_mode.continuous());
    assert_eq!(app.message, "continuous off");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.playback_mode.repeat());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.playback_mode.shuffle());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.playback_mode.target(), PlayTarget::Artist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.playback_mode.target(), PlayTarget::Artist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.input.kind(), InputKind::Rate);
    assert!(app.playback_mode.repeat());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.playback_mode.shuffle());
}

#[test]
fn keymap_key_toggles_keymap_pane() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.management_panel.keymap_open());
    assert_eq!(app.focus, FocusPane::Keymap);
    assert!(pane_active(&app, FocusPane::Keymap));
    assert_eq!(command_info_title(&app), "Keymap");
    assert!(app.info_area_visible());
    let keymap_text = keymap_text(&app);
    assert!(keymap_text.contains("k"));
    assert!(keymap_text.contains("toggle keymap pane"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.management_panel.keymap_open());
    assert_eq!(app.focus, FocusPane::Tree);
}

#[test]
fn keymap_lists_pane_resize_bindings() {
    let app = test_app(vec![test_track(1, "first track")]);
    let text = keymap_text(&app);

    assert!(text.contains("{"));
    assert!(text.contains("move boundary left/up"));
    assert!(text.contains("}"));
    assert!(text.contains("move boundary right/down"));
}

#[test]
fn keymap_pane_edits_mapping_and_persists_override() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.management_panel.keymap.capture_action(),
        Some(KeyAction::ToggleInfo)
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.management_panel.keymap.capture_action(), None);
    assert!(keymap_text(&app).contains("o"));
    assert!(keymap_text(&app).contains("default i"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.management_panel.keymap_open());
    assert!(app.layout.info_panel_visible());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.layout.info_panel_visible());

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();

    assert!(!reloaded.layout.info_panel_visible());
}

#[test]
fn keymap_pane_adds_multiple_bindings_for_one_action() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();

    let text = keymap_text(&app);
    assert!(text.contains("i / o / m"));
    assert!(text.contains("default i"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.management_panel.keymap_open());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.layout.info_panel_visible());
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.layout.info_panel_visible());

    let saved = db::key_bindings(&conn).unwrap();
    assert_eq!(saved.len(), 2);

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!reloaded.layout.info_panel_visible());
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert!(reloaded.layout.info_panel_visible());
}

#[test]
fn keymap_pane_marks_colon_as_reserved() {
    let app = test_app(vec![test_track(1, "first track")]);
    let lines = keymap_lines(&app, 80);
    let command_line = lines
        .iter()
        .find(|line| line_text(line).contains("enter command mode"))
        .unwrap();

    assert_eq!(command_line.spans[0].style, Style::default().fg(Color::Red));
    assert!(line_text(command_line).contains("(reserved)"));
}

#[test]
fn keymap_pane_marks_enter_as_reserved() {
    let app = test_app(vec![test_track(1, "first track")]);
    let lines = keymap_lines(&app, 80);
    let activate_line = lines
        .iter()
        .find(|line| line_text(line).contains("play or activate selection"))
        .unwrap();

    assert_eq!(
        activate_line.spans[0].style,
        Style::default().fg(Color::Red)
    );
    assert!(line_text(activate_line).contains("(reserved)"));
}

#[test]
fn keymap_pane_marks_esc_as_reserved() {
    let app = test_app(vec![test_track(1, "first track")]);
    let lines = keymap_lines(&app, 80);
    let escape_line = lines
        .iter()
        .find(|line| line_text(line).contains("cancel or clear active mode"))
        .unwrap();

    assert_eq!(escape_line.spans[0].style, Style::default().fg(Color::Red));
    assert!(line_text(escape_line).contains("(reserved)"));
}

#[test]
fn keymap_pane_blocks_editing_reserved_rows() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::Activate).unwrap());
    app.activate_keymap_selection();
    assert_eq!(app.management_panel.keymap.capture_action(), None);
    assert_eq!(
        app.message,
        "Enter is reserved for activation and confirmation"
    );

    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::CommandMode).unwrap());
    app.activate_keymap_selection();
    assert_eq!(app.management_panel.keymap.capture_action(), None);
    assert_eq!(app.message, "':' is reserved for command mode");

    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::Escape).unwrap());
    app.activate_keymap_selection();
    assert_eq!(app.management_panel.keymap.capture_action(), None);
    assert_eq!(app.message, "Esc is reserved for cancellation and recovery");
}

#[test]
fn keymap_pane_rejects_reserved_colon_mapping() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.management_panel.keymap.capture_action(),
        Some(KeyAction::ToggleInfo)
    );
    assert_eq!(app.message, "':' is reserved for command mode");
    assert!(db::key_bindings(&conn).unwrap().is_empty());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Command);
    assert!(app.layout.info_panel_visible());
}

#[test]
fn keymap_pane_rejects_reserved_enter_mapping() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.management_panel.keymap.capture_action(),
        Some(KeyAction::ToggleInfo)
    );
    assert_eq!(
        app.message,
        "Enter is reserved for activation and confirmation"
    );
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn keymap_pane_rejects_reserved_esc_mapping() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.management_panel.keymap.capture_action(), None);
    assert_eq!(app.message, "Esc is reserved for cancellation and recovery");
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn stale_reserved_colon_mapping_is_ignored_and_deleted() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "none:char::".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Command);
    assert!(app.layout.info_panel_visible());
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn stale_reserved_enter_mapping_is_ignored_and_deleted() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "command-mode".to_string(),
            key: "none:enter".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.current.is_some());
    assert_eq!(app.input.kind(), InputKind::None);
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn stale_reserved_action_mapping_is_ignored_and_deleted() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "command-mode".to_string(),
            key: "none:char:o".to_string(),
        },
    )
    .unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "activate".to_string(),
            key: "none:char:m".to_string(),
        },
    )
    .unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "escape".to_string(),
            key: "none:char:n".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();

    assert!(app.key_bindings.is_empty());
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn stale_ctrl_c_mapping_is_deleted_and_dispatches_quit() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "ctrl:char:c".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();

    assert!(app.key_bindings.is_empty());
    assert!(db::key_bindings(&conn).unwrap().is_empty());
    assert_eq!(
        app.key_action_for_event(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        )),
        Some(KeyAction::Quit)
    );
}

#[test]
fn modified_recovery_keys_can_be_captured() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.toggle_keymap_panel();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());

    app.activate_keymap_selection();
    app.capture_key_binding(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT))
        .unwrap();
    assert_eq!(
        app.key_action_for_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::ALT)),
        Some(KeyAction::ToggleInfo)
    );

    app.activate_keymap_selection();
    app.capture_key_binding(
        &conn,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(
        app.key_action_for_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
        Some(KeyAction::ToggleInfo)
    );
}

#[test]
fn stale_duplicate_custom_mapping_uses_keymap_order_and_deletes_conflict() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    for action in ["stop", "toggle-info"] {
        db::save_key_binding(
            &conn,
            &db::SavedKeyBinding {
                action: action.to_string(),
                key: "none:char:o".to_string(),
            },
        )
        .unwrap();
    }
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();

    assert_eq!(
        app.key_action_for_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        Some(KeyAction::ToggleInfo)
    );
    assert_eq!(
        db::key_bindings(&conn).unwrap(),
        vec![db::SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "none:char:o".to_string(),
        }]
    );
}

#[test]
fn noncanonical_mapping_is_normalized_before_reassignment() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "stop".to_string(),
            key: "alt,ctrl:char:o".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.load_key_bindings(&conn).unwrap();

    app.toggle_keymap_panel();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.activate_keymap_selection();
    app.capture_key_binding(
        &conn,
        KeyEvent::new(
            KeyCode::Char('o'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
    )
    .unwrap();

    assert_eq!(
        db::key_bindings(&conn).unwrap(),
        vec![db::SavedKeyBinding {
            action: "toggle-info".to_string(),
            key: "ctrl,alt:char:o".to_string(),
        }]
    );
    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    assert_eq!(
        reloaded.key_action_for_event(KeyEvent::new(
            KeyCode::Char('o'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        )),
        Some(KeyAction::ToggleInfo)
    );
}

#[test]
fn malformed_multi_character_mapping_is_deleted() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "stop".to_string(),
            key: "none:char:iv".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();

    assert!(app.key_bindings.is_empty());
    assert!(db::key_bindings(&conn).unwrap().is_empty());
}

#[test]
fn custom_mapping_hides_shadowed_default_from_keymap() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "stop".to_string(),
            key: "none:char:i".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.load_key_bindings(&conn).unwrap();

    assert_eq!(
        app.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::Stop)
    );
    let lines = keymap_lines(&app, 80);
    let info_line = lines
        .iter()
        .find(|line| line_text(line).contains("toggle track info pane"))
        .unwrap();
    let stop_line = lines
        .iter()
        .find(|line| line_text(line).contains("stop"))
        .unwrap();
    assert!(line_text(info_line).contains("unbound"));
    assert!(!line_text(info_line).contains("   i"));
    assert!(line_text(stop_line).contains("v / i"));
}

#[test]
fn stealing_default_key_persists_and_reset_restores_original_action() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.toggle_keymap_panel();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::Stop).unwrap());
    app.activate_keymap_selection();
    app.capture_key_binding(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    assert_eq!(
        reloaded.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::Stop)
    );

    reloaded.reset_key_binding(&conn, KeyAction::Stop).unwrap();
    assert_eq!(
        reloaded.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::ToggleInfo)
    );

    let mut reset_reloaded = test_app(vec![test_track(1, "first track")]);
    reset_reloaded.load_key_bindings(&conn).unwrap();
    assert_eq!(
        reset_reloaded.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::ToggleInfo)
    );
}

#[test]
fn resetting_shadowed_action_reclaims_its_default_key() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::save_key_binding(
        &conn,
        &db::SavedKeyBinding {
            action: "stop".to_string(),
            key: "none:char:i".to_string(),
        },
    )
    .unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.load_key_bindings(&conn).unwrap();

    app.reset_key_binding(&conn, KeyAction::ToggleInfo).unwrap();

    assert_eq!(
        app.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::ToggleInfo)
    );
    assert!(db::key_bindings(&conn).unwrap().is_empty());

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    assert_eq!(
        reloaded.key_action_for_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(KeyAction::ToggleInfo)
    );
}

#[test]
fn keymap_reset_command_clears_custom_bindings() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!db::key_bindings(&conn).unwrap().is_empty());

    set_command_input(&mut app, String::from("keymap-reset"));
    app.execute_command(&conn);

    assert!(db::key_bindings(&conn).unwrap().is_empty());
    assert!(app.key_bindings.is_empty());
    assert_eq!(app.message, "keymap reset to defaults");
}

#[test]
fn keymap_pane_resets_mapping_to_default() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.management_panel
        .keymap
        .select_row(keymap_row_for_action(KeyAction::ToggleInfo).unwrap());
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
        .unwrap();

    assert!(db::key_bindings(&conn).unwrap().is_empty());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.management_panel.keymap_open());

    app.layout.show_info_panel();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.layout.info_panel_visible());
}

#[test]
fn keymap_command_toggles_keymap_pane() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    set_command_input(&mut app, String::from("keymap"));
    app.execute_command(&conn);

    assert!(app.management_panel.keymap_open());
    assert_eq!(app.focus, FocusPane::Keymap);
    assert_eq!(app.message, "keymap panel");
}

#[test]
fn keymap_pane_uses_shared_bottom_slot() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.management_panel.keymap_open());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.management_panel.playlist_open());
    assert!(!app.management_panel.keymap_open());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.management_panel.keymap_open());
    assert!(!app.management_panel.playlist_open());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.management_panel.keymap_open());
    assert!(app.layout.info_panel_visible());
    assert_eq!(app.focus, FocusPane::Tree);
}

#[test]
fn tab_cycles_to_keymap_pane_when_open() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Tree);
    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Tracks);
    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Keymap);
}

#[test]
fn up_still_moves_after_k_becomes_keymap_toggle() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.browser.select_track_row(2);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.management_panel.keymap_open());

    app.focus = FocusPane::Tracks;
    app.handle_key(&conn, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.selected_playable_track_index(), Some(0));
}

#[test]
fn j_is_unbound_by_default() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.browser.select_track_row(1);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(0));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(1));
}

#[test]
fn mouse_hit_testing_allows_keymap_info_pane() {
    assert_eq!(
        mouse_pane(
            60,
            17,
            MouseLayout::new(100, 30, 2)
                .with_info(true, false)
                .with_keymap_info(true)
        ),
        Some(FocusPane::Keymap)
    );
}
