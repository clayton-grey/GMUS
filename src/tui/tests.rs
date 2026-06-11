use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::Terminal;
use tempfile::tempdir;

use super::filter::FilterQuery;
use super::formatting::display_width;
use super::keymap::{keymap_lines, keymap_row_for_action, KeyAction};
use super::lines::{
    album_header_line, command_help_lines, command_info_lines, command_info_title,
    disc_divider_line, filter_info_lines, filter_line, input_line, metadata_lines,
    now_playing_line, now_playing_row_style, pane_active, pane_highlight_style, playback_line,
    playlist_entry_text, playlist_header_line, playlist_track_line, rate_info_lines, rate_line,
    track_line, tree_item_line,
};
use super::mouse::{mouse_pane, MouseLayout};
use super::renderer::{render, render_playlist_info_pane};
use super::*;
use crate::integration::{
    Integration, IntegrationCommand, IntegrationEvent, NoopIntegration, TrackSnapshot,
};
use crate::player::NullPlayer;

#[test]
fn playback_sequence_respects_filter() {
    let mut app = test_app(vec![
        test_track(1, "keep one"),
        test_track(2, "skip this"),
        test_track(3, "keep two"),
    ]);
    app.filter = "keep".to_string();
    app.sync_selection();
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    assert_eq!(app.next_playback_index(1), Some(2));
    assert_eq!(app.next_playback_index(-1), None);
}

#[test]
fn continuous_controls_auto_advance_only() {
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

    assert_eq!(app.next_auto_advance_index(), Some(1));

    app.toggle_continuous();

    assert!(!app.continuous);
    assert_eq!(app.next_auto_advance_index(), None);
    assert_eq!(app.next_playback_index(1), Some(1));
}

#[test]
fn playback_target_limits_sequence_to_current_artist_or_album() {
    let mut other_album = test_track(2, "same artist other album");
    other_album.album = Some("Other Album".to_string());
    let mut other_artist = test_track(3, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![
        test_track(1, "first track"),
        other_album,
        other_artist,
    ]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.play_target = PlayTarget::Artist;
    assert_eq!(app.playback_sequence_indices(), vec![0, 1]);

    app.play_target = PlayTarget::Album;
    assert_eq!(app.playback_sequence_indices(), vec![0]);
}

#[test]
fn repeat_wraps_playback_sequence() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    assert_eq!(app.next_playback_index(1), None);

    app.repeat = true;
    assert_eq!(app.next_playback_index(1), Some(0));
}

#[test]
fn shuffle_uses_a_permuted_playback_order() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
        test_track(3, "third track"),
    ]);
    app.shuffle = true;
    app.shuffle_seed = 1;

    let next = app.next_playback_index(1);

    assert!(next.is_some());
    assert_eq!(
        app.shuffle_scope
            .iter()
            .map(|entry| entry.track_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(app.shuffle_order.len(), 3);
    assert_ne!(
        app.shuffle_order
            .iter()
            .map(|entry| entry.track_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn now_playing_line_splits_track_and_album() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 0,
    });

    let text = line_text(&now_playing_line(&app, 80));

    assert_eq!(display_width(&text), 80);
    assert!(text.starts_with(" Artist - first track"));
    assert!(text.ends_with("Album (2018)"));
    assert_eq!(
        now_playing_row_style(),
        Style::default().fg(Color::Black).bg(Color::White)
    );
}

#[test]
fn playback_line_shows_time_bar_and_play_modes() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 0,
    });
    app.player.seek(Duration::from_millis(50_000)).unwrap();
    app.player.play().unwrap();
    app.play_target = PlayTarget::Album;
    app.repeat = true;
    app.shuffle = true;

    let line = playback_line(&app, 120);
    let text = line_text(&line);

    assert!(text.contains(" > 0:50 / 1:40 ["));
    assert!(text.contains("[============================----------------------------]"));
    assert!(text.contains("album from library | 50% | C R S"));
    assert_eq!(line.spans[0].style, Style::default().fg(Color::LightGreen));
    assert_eq!(
        line.spans[line.spans.len() - 5].style,
        Style::default().fg(Color::White)
    );
    assert_eq!(
        line.spans[line.spans.len() - 3].style,
        Style::default().fg(Color::White)
    );
    assert_eq!(
        line.spans[line.spans.len() - 1].style,
        Style::default().fg(Color::White)
    );
    assert!(!text.contains("(100%)"));
}

#[test]
fn playback_line_shows_rate_and_resizes_progress_bar() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 0,
    });
    app.suspended_position_ms = Some(50_000);

    let normal = line_text(&playback_line(&app, 80));
    app.player.set_rate(0.75).unwrap();
    let rated = line_text(&playback_line(&app, 80));

    assert!(rated.contains("0:50 / 1:40 (75%) ["));
    assert_eq!(display_width(&rated), 80);
    assert_eq!(
        playback_bar_width(&normal) - playback_bar_width(&rated),
        display_width(" (75%)")
    );
}

#[test]
fn playback_line_uses_bar_marker_when_not_playing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 0,
    });
    app.suspended_position_ms = Some(50_000);

    let line = playback_line(&app, 80);
    let text = line_text(&line);

    assert!(text.contains(" | 0:50 / 1:40 ["));
    assert!(text.contains("| C R S"));
    assert_eq!(line.spans[0].style, Style::default().fg(Color::DarkGray));
    assert_eq!(
        line.spans[line.spans.len() - 5].style,
        Style::default().fg(Color::White)
    );
    assert_eq!(
        line.spans[line.spans.len() - 3].style,
        Style::default().fg(Color::DarkGray)
    );
    assert_eq!(
        line.spans[line.spans.len() - 1].style,
        Style::default().fg(Color::DarkGray)
    );
}

#[test]
fn active_tick_interval_tracks_playback_rate() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.player.play().unwrap();

    app.player.set_rate(0.75).unwrap();
    assert_eq!(app.tick_interval().as_millis(), 1_333);

    app.player.set_rate(1.25).unwrap();
    assert_eq!(app.tick_interval().as_millis(), 800);
}

#[test]
fn mode_toggles_show_transient_playback_status() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.toggle_repeat();

    let text = line_text(&playback_line(&app, 80));
    assert!(text.contains(" repeat on"));
    assert!(!text.contains("| C R S"));
}

#[test]
fn continuous_flag_reflects_toggle_state() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.continuous = false;

    let line = playback_line(&app, 80);

    assert_eq!(
        line.spans[line.spans.len() - 5].style,
        Style::default().fg(Color::DarkGray)
    );
}

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
    assert!(!app.continuous);
    assert_eq!(app.message, "continuous off");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.repeat);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.shuffle);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.play_target, PlayTarget::Artist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.play_target, PlayTarget::Artist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.rate_mode);
    assert!(app.repeat);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.shuffle);
}

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

    app.command = format!("add {}", library_dir.path().display());
    app.execute_command(&conn);

    let roots = db::active_library_roots(&conn).unwrap();
    assert_eq!(roots.len(), 1);
    assert!(app.message.starts_with("added "));

    app.command = String::from("library");
    app.execute_command(&conn);
    assert!(app.message.contains(library_dir.path().to_str().unwrap()));
    assert!(app.command_focus);
    assert_eq!(app.command_output_kind, CommandOutputKind::LibraryRoots);
    assert!(app.command_output[0].starts_with("library roots"));
    assert!(app.command_output[1].contains("[x]"));
    assert!(app.command_output[1].contains(library_dir.path().to_str().unwrap()));

    app.command = format!("remove {}", library_dir.path().display());
    app.execute_command(&conn);

    assert!(db::active_library_roots(&conn).unwrap().is_empty());
    assert!(app.message.starts_with("removed "));
}

#[test]
fn command_mode_executes_playlist_commands() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(Vec::new());

    app.command = String::from("playlist Road");
    app.execute_command(&conn);

    assert!(app.playlist_panel_open);
    assert_eq!(app.playlists.len(), 1);
    assert_eq!(app.playlists[0].name, "Road");
    assert_eq!(app.active_playlist_id, Some(app.playlists[0].id));

    app.command = String::from("playlist-clear Road");
    app.execute_command(&conn);
    assert!(app.message.starts_with("cleared 0 tracks from Road"));

    app.command = String::from("playlist-delete Road");
    app.execute_command(&conn);
    assert!(app.message.starts_with("deleted playlist Road"));
    assert!(app.playlists.is_empty());
}

#[test]
fn rate_command_changes_and_reports_playback_rate() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    app.command = String::from("rate 0.75");
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "playback rate 0.75x");

    app.command = String::from("rate");
    app.execute_command(&conn);

    assert_eq!(app.message, "playback rate 0.75x");
}

#[test]
fn rate_command_accepts_percent_and_reset() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());

    app.command = String::from("rate 125%");
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 1.25);
    assert_eq!(app.message, "playback rate 1.25x");

    app.command = String::from("rate 75");
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "playback rate 0.75x");

    app.command = String::from("rate reset");
    app.execute_command(&conn);

    assert_eq!(app.player.rate(), 1.0);
    assert_eq!(app.message, "playback rate 1.00x");
}

#[test]
fn rate_command_rejects_invalid_values_without_changing_rate() {
    let conn = test_conn();
    let mut app = test_app(Vec::new());
    app.player.set_rate(0.75).unwrap();

    for command in ["rate 0", "rate 10", "rate 401", "rate NaN", "rate fast"] {
        app.command = command.to_string();
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

    assert!(app.rate_mode);
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

    assert!(!app.rate_mode);
    assert!(app.rate_input.is_empty());
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

    assert!(app.rate_mode);
    assert_eq!(app.rate_input, "500");
    assert_eq!(app.player.rate(), 0.75);
    assert!(lines_text(&rate_info_lines(&app, 80, 8)).contains("invalid rate"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.rate_mode);
    assert!(app.rate_input.is_empty());
    assert_eq!(app.player.rate(), 0.75);
    assert_eq!(app.message, "rate cancelled");
}

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
#[test]
fn command_mode_toggles_track_notifications() {
    let conn = test_conn();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = test_app(Vec::new());
    app.integration = Box::new(RecordingIntegration {
        events: Rc::clone(&events),
    });

    app.command = String::from("notifications off");
    app.execute_command(&conn);

    assert!(!app.track_notifications_visible);
    assert_eq!(app.message, "track notifications hidden");
    assert_eq!(
        events.borrow().as_slice(),
        &[IntegrationEvent::TrackNotificationsVisible(false)]
    );

    app.command = String::from("notifications toggle");
    app.execute_command(&conn);

    assert!(app.track_notifications_visible);
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

    app.command = String::from("library");
    app.execute_command(&conn);

    assert!(app.command_focus);
    assert_eq!(app.command_output_kind, CommandOutputKind::LibraryRoots);
    assert_eq!(app.command_roots.len(), 2);
    assert_eq!(app.command_selected, 0);
    assert_eq!(command_info_title(&app), "Library");
    assert_eq!(
        command_info_lines(&app, 80, 10)[1].spans[0].style,
        pane_highlight_style(true)
    );

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.command_selected, 1);
    let toggled_path = app.command_roots[1].path.clone();

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
    assert!(app.command_focus);
    assert_eq!(app.command_roots[app.command_selected].path, toggled_path);

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

    assert!(app.command_mode);
    assert!(app.input_bar_visible());
    assert_eq!(line_text(&input_line(&app, 20)), " :l_");
    assert_eq!(
        input_line(&app, 20).spans[1].style,
        Style::default().fg(Color::White).bg(Color::Blue)
    );
}

#[test]
fn library_output_renders_in_info_pane() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.command_output = vec![
        String::from("library roots (1 active / 1 total)"),
        String::from("[x] /tmp/music"),
    ];

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
    app.command_mode = true;
    app.command = String::from("library");

    let text = lines_text(&command_info_lines(&app, 120, 10));

    assert!(text.contains("commands: add remove update library playlist"));
    assert!(text.contains("playlist-clear playlist-delete keymap keymap-reset"));
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
fn keymap_key_toggles_keymap_pane() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.keymap_panel_open);
    assert_eq!(app.focus, FocusPane::Keymap);
    assert!(pane_active(&app, FocusPane::Keymap));
    assert_eq!(command_info_title(&app), "Keymap");
    assert!(app.info_area_visible());
    let keymap_text = keymap_text(&app);
    assert!(keymap_text.contains("k"));
    assert!(keymap_text.contains("toggle keymap pane"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.keymap_panel_open);
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
fn pane_resize_keys_adjust_library_boundary_for_tree_and_tracks() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.focus = FocusPane::Tree;
    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.library_pane_percent_offset, 2);
    assert!(app.message.contains("library pane larger"));
    assert_eq!(
        db::pane_layout(&conn).unwrap().library_percent_offset,
        app.library_pane_percent_offset
    );

    app.focus = FocusPane::Tracks;
    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.library_pane_percent_offset, 4);
    assert!(app.message.contains("tracks pane smaller"));

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.library_pane_percent_offset, 2);
    assert!(app.message.contains("tracks pane larger"));
}

#[test]
fn pane_resize_keys_adjust_bottom_management_pane_height() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.keymap_panel_open = true;
    app.focus = FocusPane::Keymap;

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.info_pane_height_offset, -1);
    assert!(app.message.contains("info pane smaller"));
    assert_eq!(
        db::pane_layout(&conn).unwrap().info_height_offset,
        app.info_pane_height_offset
    );

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.info_pane_height_offset, 0);
    assert!(app.message.contains("info pane larger"));
}

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

    let app = App::new(&conn, &test_paths()).unwrap();

    assert_eq!(app.library_pane_percent_offset, 4);
    assert_eq!(app.info_pane_height_offset, 3);
}

#[test]
fn keymap_pane_edits_mapping_and_persists_override() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.keymap_capture_action, Some(KeyAction::ToggleInfo));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.keymap_capture_action, None);
    assert!(keymap_text(&app).contains("o"));
    assert!(keymap_text(&app).contains("default i"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.keymap_panel_open);
    assert!(app.info_panel_visible);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.info_panel_visible);

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();

    assert!(!reloaded.info_panel_visible);
}

#[test]
fn keymap_pane_adds_multiple_bindings_for_one_action() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
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
    assert!(!app.keymap_panel_open);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.info_panel_visible);
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.info_panel_visible);

    let saved = db::key_bindings(&conn).unwrap();
    assert_eq!(saved.len(), 2);

    let mut reloaded = test_app(vec![test_track(1, "first track")]);
    reloaded.load_key_bindings(&conn).unwrap();
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!reloaded.info_panel_visible);
    reloaded
        .handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .unwrap();
    assert!(reloaded.info_panel_visible);
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

    app.selected_keymap_row = keymap_row_for_action(KeyAction::Activate).unwrap();
    app.activate_keymap_selection();
    assert_eq!(app.keymap_capture_action, None);
    assert_eq!(
        app.message,
        "Enter is reserved for activation and confirmation"
    );

    app.selected_keymap_row = keymap_row_for_action(KeyAction::CommandMode).unwrap();
    app.activate_keymap_selection();
    assert_eq!(app.keymap_capture_action, None);
    assert_eq!(app.message, "':' is reserved for command mode");

    app.selected_keymap_row = keymap_row_for_action(KeyAction::Escape).unwrap();
    app.activate_keymap_selection();
    assert_eq!(app.keymap_capture_action, None);
    assert_eq!(app.message, "Esc is reserved for cancellation and recovery");
}

#[test]
fn keymap_pane_rejects_reserved_colon_mapping() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.keymap_capture_action, Some(KeyAction::ToggleInfo));
    assert_eq!(app.message, "':' is reserved for command mode");
    assert!(db::key_bindings(&conn).unwrap().is_empty());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.command_mode);
    assert!(app.info_panel_visible);
}

#[test]
fn keymap_pane_rejects_reserved_enter_mapping() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.keymap_capture_action, Some(KeyAction::ToggleInfo));
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
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.keymap_capture_action, None);
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

    assert!(app.command_mode);
    assert!(app.info_panel_visible);
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
    assert!(!app.command_mode);
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
fn keymap_reset_command_clears_custom_bindings() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
    app.apply_selection_state();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(!db::key_bindings(&conn).unwrap().is_empty());

    app.command = String::from("keymap-reset");
    app.execute_command(&conn);

    assert!(db::key_bindings(&conn).unwrap().is_empty());
    assert!(app.key_bindings.is_empty());
    assert_eq!(app.message, "keymap reset to defaults");
}

#[test]
fn restore_filter_command_toggles_persistent_setting() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.filter = String::from("artist:eno");

    app.command = String::from("restore-filter");
    app.execute_command(&conn);

    assert!(!app.restore_filter);
    assert!(!db::restore_filter_enabled(&conn).unwrap());
    assert_eq!(app.message, "restore filter off");

    app.command = String::from("restore-filter on");
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

    app.command = String::from("restore-track");
    app.execute_command(&conn);

    assert!(!app.restore_track);
    assert!(!db::restore_track_enabled(&conn).unwrap());
    assert_eq!(app.message, "restore track off");

    app.command = String::from("restore-track on");
    app.execute_command(&conn);

    assert!(app.restore_track);
    assert!(db::restore_track_enabled(&conn).unwrap());
}

#[test]
fn keymap_pane_resets_mapping_to_default() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    app.selected_keymap_row = keymap_row_for_action(KeyAction::ToggleInfo).unwrap();
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
    assert!(!app.keymap_panel_open);

    app.info_panel_visible = true;
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.info_panel_visible);
}

#[test]
fn keymap_command_toggles_keymap_pane() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.command = String::from("keymap");
    app.execute_command(&conn);

    assert!(app.keymap_panel_open);
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
    assert!(app.keymap_panel_open);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.playlist_panel_open);
    assert!(!app.keymap_panel_open);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.keymap_panel_open);
    assert!(!app.playlist_panel_open);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.keymap_panel_open);
    assert!(app.info_panel_visible);
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
    app.selected_track_row = 2;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.keymap_panel_open);

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
    app.selected_track_row = 1;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(0));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playable_track_index(), Some(1));
}

#[test]
fn info_panel_toggle_preserves_command_info_overlay() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.info_panel_visible);
    assert!(!app.info_area_visible());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.info_area_visible());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    assert!(!app.command_mode);
    assert!(!app.info_panel_visible);
    assert!(!app.info_area_visible());

    app.show_command_output(vec![String::from("library roots")]);
    assert!(app.info_area_visible());
    app.clear_command_output();
    assert!(!app.info_area_visible());
}

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
    app.selected_track_row = 2;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.playlist_panel_open);
    assert_eq!(app.focus, FocusPane::Playlist);
    assert!(pane_active(&app, FocusPane::Playlist));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.playlist_panel_open);
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

    let playlist_id = app.active_playlist_id.unwrap();
    assert!(app.playlist_panel_open);
    assert_eq!(db::playlist_track_ids(&conn, playlist_id).unwrap(), vec![2]);
    assert!(playlist_text(&app).contains("second track"));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.focus, FocusPane::Playlist);
    assert!(pane_active(&app, FocusPane::Playlist));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.selected_playlist_row, 1);
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .unwrap();
    assert!(app.playlist_panel_open);
    assert_eq!(app.focus, FocusPane::Playlist);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();
    assert!(!app.playlist_panel_open);
    assert!(app.info_panel_visible);
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
    app.active_playlist_id = Some(playlist.id);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(0));
    assert_eq!(
        app.current.as_ref().and_then(|current| current.source),
        Some(PlaybackSource::PlaylistTrack {
            playlist_id: playlist.id,
            playlist_track_id: app.playlist_track_entry_ids[&playlist.id][0]
        })
    );
    assert!(!app.expanded_playlists.contains(&playlist.id));
}

#[test]
fn space_on_playlist_panel_header_still_toggles_expansion() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.active_playlist_id = Some(7);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();

    app.space_action();

    assert!(app.expanded_playlists.contains(&7));
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
    app.active_playlist_id = Some(playlist.id);
    app.focus = FocusPane::Tracks;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.playlist_panel_open);
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
    app.active_playlist_id = Some(playlist.id);
    app.expanded_playlists.insert(playlist.id);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.selected_playlist_row = 1;

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
    app.playlist_track_ids.insert(7, vec![1, 2]);
    app.playlist_track_entry_ids.insert(7, vec![11, 12]);
    app.playlist_track_indices.insert(7, vec![0, 1]);
    app.active_playlist_id = Some(7);
    app.expanded_playlists.insert(7);
    app.playlist_panel_open = true;
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
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.active_playlist_id = Some(7);
    app.expanded_playlists.insert(7);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.selected_playlist_row = 1;

    app.space_action();

    assert!(!app.expanded_playlists.contains(&7));
    assert_eq!(app.selected_playlist_row, 0);
    assert!(matches!(
        app.view.playlist_entries.get(app.selected_playlist_row),
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
    app.active_playlist_id = Some(playlist.id);
    app.expanded_playlists.insert(playlist.id);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.selected_playlist_row = 1;

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
    app.active_playlist_id = Some(playlist.id);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Tracks;
    app.sync_selection();
    app.selected_track_row = 0;

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
    app.active_playlist_id = Some(playlist.id);
    app.playlist_panel_open = true;
    app.sync_selection();
    app.focus = FocusPane::Tree;
    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Artist { artist } if artist == "Artist"))
        .unwrap();

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
fn escape_clears_command_output_before_filter() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.filter = String::from("keep");
    app.command_output = vec![
        String::from("library roots"),
        String::from("[x] /tmp/music"),
    ];
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(app.command_output.is_empty());
    assert_eq!(app.filter, "keep");
    assert_eq!(app.playback_sequence_indices(), &[0]);
}

#[test]
fn normal_navigation_clears_command_output() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    let conn = test_conn();
    app.command_output = vec![
        String::from("library roots"),
        String::from("[x] /tmp/music"),
    ];

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();

    assert!(app.command_output.is_empty());
}

#[test]
fn metadata_pane_shows_selected_track_details() {
    let mut track = test_track(1, "first track");
    track.composer = Some("Someone Quiet".to_string());
    track.genre = Some("Ambient".to_string());
    track.track_total = Some(12);
    track.disc_number = Some(1);
    track.disc_total = Some(2);
    let app = test_app(vec![track]);

    let lines = metadata_lines(&app, 80);
    let text = lines_text(&lines);
    let track_line = &lines[7];

    assert!(text.contains("selected track"));
    assert!(text.contains("title    first track"));
    assert!(text.contains("artist   Artist"));
    assert!(text.contains("album    Album"));
    assert!(text.contains("composer Someone Quiet"));
    assert!(text.contains("genre    Ambient"));
    assert!(text.contains("released 2018-05-11"));
    assert!(text.contains("track    1/12  disc 1/2"));
    assert!(text.contains("plays    0"));
    assert!(!text.contains("/tmp/first track.flac"));
    assert_eq!(
        track_line.spans[0].style,
        Style::default().fg(Color::DarkGray)
    );
    assert_eq!(track_line.spans[1].style, Style::default().fg(Color::White));
    assert_eq!(
        track_line.spans[2].style,
        Style::default().fg(Color::DarkGray)
    );
    assert_eq!(track_line.spans[3].style, Style::default().fg(Color::White));
}

#[test]
fn startup_info_pane_shows_application_intro_until_interaction() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.startup_info_visible = true;

    let lines = metadata_lines(&app, 80);

    assert_eq!(line_text(&lines[0]), "");
    assert_eq!(line_text(&lines[1]), " GMUS");
    assert_eq!(
        line_text(&lines[2]),
        " a CMUS inspired terminal music player"
    );
    assert_eq!(line_text(&lines[3]), " authors: Clayton Grey with Codex");
    assert_eq!(
        lines[1].spans[0].style,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        lines[2].spans[0].style,
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC)
    );
}

#[test]
fn first_key_interaction_dismisses_startup_info() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();
    app.startup_info_visible = true;

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.startup_info_visible);
    assert!(lines_text(&metadata_lines(&app, 80)).contains("selected track"));
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
    app.command_mode = true;
    app.command = String::from("update");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.command_mode);
    assert!(app.library_job.is_some());
    assert!(app.command_output[0].contains("working: :update"));
    assert!(app.command_output[1].contains("scanning files"));

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
    app.command_mode = true;
    app.command = String::from("update");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(wait_for_library_job(&mut app, &conn));
    assert!(app.library_job.is_none());
    assert_eq!(app.message, "no active library roots; use :add PATH");
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
    app.command_mode = true;
    app.command = String::from("update");
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(app.library_job.is_some());

    app.command_mode = true;
    app.command = String::from("playlist Road");
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(app.library_job.is_some());
    assert!(app.playlist_panel_open);
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
    app.command_mode = true;
    app.command = String::from("update");
    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    app.command_mode = true;
    app.command = String::from("update");
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
    app.command_mode = true;
    app.command = String::from("lib");

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.command, "library ");
}

#[test]
fn tab_completes_filesystem_paths_for_add() {
    let parent = tempdir().unwrap();
    let music = parent.path().join("MusicRoot");
    fs::create_dir(&music).unwrap();
    let mut app = test_app(vec![test_track(1, "first track")]);
    let conn = test_conn();
    app.command_mode = true;
    app.command = format!("add {}/Mu", parent.path().display());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(
        app.command,
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
    app.command_mode = true;
    app.command = format!("remove {}", &root[..prefix_len]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.command, format!("remove {root} "));
}

#[test]
fn expired_transient_status_clears_on_tick() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.transient_status = Some(TransientStatus {
        text: "repeat on".to_string(),
        until: Instant::now() - Duration::from_secs(1),
    });

    assert!(app.expire_transient_status());
    assert!(app.transient_status.is_none());
}

#[test]
fn render_uses_final_row_for_playback_status() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &mut app)).unwrap();

    let bottom = buffer_row_text(terminal.backend().buffer(), 11, 80);
    assert!(bottom.contains("["));
    assert!(bottom.contains("library"));
}

#[test]
fn playback_bar_scales_down_with_width() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 0,
    });
    app.suspended_position_ms = Some(50_000);

    let text = line_text(&playback_line(&app, 44));

    assert!(text.contains("[==--]"));

    app.player.set_rate(0.75).unwrap();
    let rated = line_text(&playback_line(&app, 44));

    assert!(rated.contains("(75%)"));
    assert!(!rated.contains('['));
    assert_eq!(display_width(&rated), 44);
}

#[test]
fn failed_seek_updates_message_without_crashing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(FailingSeekPlayer);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 197_500,
        listened_ms: 0,
    });

    app.seek_relative(5).unwrap();

    assert!(app.message.contains("seek failed"));
    assert!(app.message.contains("decoder refused seek"));
}

#[test]
fn filter_line_has_its_own_prompt() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.filter_mode = true;
    app.filter = "beat".to_string();
    let line = filter_line(&app, 40);

    assert_eq!(line_text(&line), " filter: beat_");
    assert_eq!(
        line.spans[1].style,
        Style::default().fg(Color::White).bg(Color::Blue)
    );
    assert_eq!(
        line.spans[2].style,
        Style::default().fg(Color::White).bg(Color::Blue)
    );
    assert!(!line_text(&playback_line(&app, 80)).contains("filter:"));
}

#[test]
fn filter_line_uses_gray_placeholder_and_persists_for_active_filter() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.filter_mode = true;
    let placeholder = filter_line(&app, 40);

    assert_eq!(line_text(&placeholder), " filter: none_");
    assert_eq!(
        placeholder.spans[2].style,
        Style::default().fg(Color::Gray).bg(Color::Blue)
    );

    app.filter_mode = false;
    app.filter = "beat".to_string();

    assert!(app.filter_bar_visible());
    let active_filter = filter_line(&app, 40);
    assert_eq!(line_text(&active_filter), " filter: beat");
    assert_eq!(
        active_filter.spans[1].style,
        Style::default().fg(Color::Black).bg(Color::White)
    );
}

#[test]
fn fielded_filter_matches_metadata_ranges_and_roots() {
    let mut ambient = test_track(1, "quiet one");
    ambient.genre = Some("Ambient".to_string());
    ambient.album_year = Some(2018);
    ambient.release_date = Some("2018-05-11".to_string());
    ambient.library_root = Some("/tmp/Instrumental".to_string());

    let mut rock = test_track(2, "loud one");
    rock.genre = Some("Rock".to_string());
    rock.album_year = Some(2024);
    rock.library_root = Some("/tmp/Vocal".to_string());

    let mut app = test_app(vec![ambient, rock]);
    app.filter = "genre:ambient year:2010..2020 root:instrumental".to_string();
    app.sync_selection();

    assert_eq!(app.playback_sequence_indices(), vec![0]);
}

#[test]
fn fielded_filter_supports_quoted_values_negation_booleans_and_counts() {
    let mut wanted = test_track(1, "wanted track");
    wanted.artist = Some("Other Artist".to_string());
    wanted.genre = Some("Ambient".to_string());
    wanted.play_count = 6;
    wanted.compilation = false;

    let mut skipped = test_track(2, "skipped track");
    skipped.artist = Some("Other Artist".to_string());
    skipped.genre = Some("Podcast".to_string());
    skipped.play_count = 10;
    skipped.compilation = true;

    let mut app = test_app(vec![wanted, skipped]);
    app.filter = "artist:\"Other Artist\" -genre:podcast compilation:false plays:>5".to_string();
    app.sync_selection();

    assert_eq!(app.playback_sequence_indices(), vec![0]);
}

#[test]
fn unknown_filter_field_shows_hint_and_matches_nothing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.filter = "mood:blue".to_string();
    app.sync_selection();

    let query = FilterQuery::parse(&app.filter);

    assert_eq!(app.playback_sequence_indices(), Vec::<usize>::new());
    assert_eq!(query.warning(), Some("unknown filter field: mood"));
}

#[test]
fn filter_info_pane_hints_fields_while_typing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.filter_mode = true;
    app.filter = "genre:ambient".to_string();

    let text = lines_text(&filter_info_lines(&app, 80, 8));

    assert!(app.info_area_visible());
    assert_eq!(command_info_title(&app), "Filter");
    assert!(text.contains("fields: title artist album"));
    assert!(text.contains("examples: genre:ambient year:2010..2020"));
}

#[test]
fn album_header_shows_year_and_right_aligned_duration() {
    let line = album_header_line("Album", Some(2018), 100_000, 24);
    let text = line_text(&line);

    assert_eq!(display_width(&text), 24);
    assert!(text.starts_with("Album"));
    assert!(text.contains("--------"));
    assert!(text.ends_with("2018 1:40"));
    assert_eq!(
        line.spans[1].style,
        Style::default().fg(Color::LightMagenta)
    );
}

#[test]
fn track_line_right_aligns_duration() {
    let app = test_app(vec![test_track(1, "first track")]);
    let line = track_line(&app, 0, false, 32);
    let text = line_text(&line);

    assert_eq!(display_width(&text), 32);
    assert!(text.starts_with("  01. first track"));
    assert!(text.ends_with("1:40"));
}

#[test]
fn single_disc_albums_hide_disc_number() {
    let mut track = test_track(1, "first track");
    track.disc_number = Some(1);
    let app = test_app(vec![track]);
    let line = track_line(&app, 0, false, 32);
    let text = line_text(&line);

    assert!(text.starts_with("  01. first track"));
    assert!(!text.contains("1.01."));
}

#[test]
fn multi_disc_albums_add_divider_and_show_disc_numbers() {
    let mut disc_one = test_track(1, "disc one track");
    disc_one.disc_number = Some(1);
    let mut disc_two = test_track(2, "disc two track");
    disc_two.disc_number = Some(2);
    disc_two.track_number = Some(1);
    let app = test_app(vec![disc_one, disc_two]);

    assert!(matches!(
        app.track_rows().get(1),
        Some(TrackRow::Track {
            show_disc_number: true,
            ..
        })
    ));
    assert!(matches!(
        app.track_rows().get(2),
        Some(TrackRow::DiscDivider {
            disc_number: Some(2)
        })
    ));
    let divider = match app.track_rows().get(2) {
        Some(TrackRow::DiscDivider { disc_number }) => disc_divider_line(*disc_number, 24),
        row => panic!("expected disc divider, got {row:?}"),
    };
    assert_eq!(divider.spans[0].style, Style::default().fg(Color::DarkGray));

    let line = track_line(&app, 1, true, 40);
    assert!(line_text(&line).starts_with("  2.01. disc two track"));
}

#[test]
fn album_headers_keep_scanned_years() {
    let app = test_app(vec![test_track(1, "first track")]);

    match app.track_rows().first() {
        Some(TrackRow::AlbumHeader { album_year, .. }) => {
            assert_eq!(*album_year, Some(2018));
        }
        row => panic!("expected album header, got {row:?}"),
    }
}

#[test]
fn tab_confirms_filter_and_focuses_library() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.filter_mode = true;
    app.filter = "keep".to_string();

    let should_quit = app
        .handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert!(!should_quit);
    assert!(!app.filter_mode);
    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.selected_tree, 0);
    assert_eq!(app.selected_track_row, 1);
    assert_eq!(app.playback_sequence_indices(), &[0]);
}

#[test]
fn enter_confirms_filter_and_focuses_library() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.filter_mode = true;
    app.filter = "keep".to_string();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.filter_mode);
    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.playback_sequence_indices(), &[0]);
    assert_eq!(db::saved_filter(&conn).unwrap().as_deref(), Some("keep"));
}

#[test]
fn app_start_restores_saved_filter_when_enabled() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(&conn, &test_track_metadata("/tmp/keep.flac", "keep one", 1)).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/skip.flac", "skip this", 2),
    )
    .unwrap();
    db::save_filter(&conn, "keep").unwrap();

    let app = App::new(&conn, &test_paths()).unwrap();

    assert_eq!(app.filter, "keep");
    assert_eq!(app.playback_sequence_indices(), vec![0]);
}

#[test]
fn app_start_ignores_saved_filter_when_disabled() {
    let data_dir = tempdir().unwrap();
    let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
    db::upsert_track(&conn, &test_track_metadata("/tmp/keep.flac", "keep one", 1)).unwrap();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/skip.flac", "skip this", 2),
    )
    .unwrap();
    db::save_filter(&conn, "keep").unwrap();
    db::save_restore_filter_enabled(&conn, false).unwrap();

    let app = App::new(&conn, &test_paths()).unwrap();

    assert!(app.filter.is_empty());
    assert_eq!(app.playback_sequence_indices(), vec![0, 1]);
}

#[test]
fn escape_clears_filter_entry() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.filter_mode = true;
    app.filter = "keep".to_string();
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.filter_mode);
    assert!(app.filter.is_empty());
    assert_eq!(app.message, "filter cleared");
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    assert_eq!(db::saved_filter(&conn).unwrap().as_deref(), Some(""));
}

#[test]
fn escape_clears_active_filter_outside_filter_entry() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.filter = "keep".to_string();
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.filter_mode);
    assert!(app.filter.is_empty());
    assert_eq!(app.message, "filter cleared");
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);
}

#[test]
fn escape_preserves_valid_selection_when_clearing_filter() {
    let mut other_artist = test_track(2, "other track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
    let conn = test_conn();
    app.filter = "other".to_string();
    app.sync_selection();
    assert_eq!(
        app.selected_tree_entry().map(TreeEntry::artist),
        Some("Other Artist")
    );
    assert_eq!(app.selected_playable_track_index(), Some(1));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(app.filter.is_empty());
    assert_eq!(
        app.selected_tree_entry().map(TreeEntry::artist),
        Some("Other Artist")
    );
    assert_eq!(app.selected_playable_track_index(), Some(1));
}

#[test]
fn track_pane_selection_skips_album_headers() {
    let mut second_album = test_track(3, "second album track");
    second_album.album = Some("Another Album".to_string());
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
        second_album,
    ]);
    app.sync_selection();

    assert_eq!(app.selected_track_row, 1);
    app.focus = FocusPane::Tracks;
    app.move_down();
    assert_eq!(app.selected_track_row, 2);
    app.move_down();
    assert_eq!(app.selected_track_row, 4);
    app.move_up();
    assert_eq!(app.selected_track_row, 2);
}

#[test]
fn mouse_scroll_moves_tree_pane_without_changing_focus() {
    let mut tracks = Vec::new();
    for id in 1..=6 {
        let mut track = test_track(id, &format!("track {id}"));
        track.artist = Some(format!("Artist {id}"));
        track.album_artist = track.artist.clone();
        tracks.push(track);
    }
    let mut app = test_app(tracks);
    app.focus = FocusPane::Tracks;

    let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 1), 100, 30);

    assert!(handled);
    assert_eq!(app.focus, FocusPane::Tracks);
    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
}

#[test]
fn mouse_scroll_moves_track_pane_and_skips_album_headers() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
        test_track(3, "third track"),
        test_track(4, "fourth track"),
    ]);

    let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 60, 10), 100, 30);

    assert!(handled);
    assert_eq!(app.selected_track_row, 2);
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
    app.playlist_track_ids.insert(7, (1..=6).collect());
    app.playlist_track_entry_ids.insert(7, (11..=16).collect());
    app.playlist_track_indices.insert(7, (0..6).collect());
    app.active_playlist_id = Some(7);
    app.expanded_playlists.insert(7);
    app.playlist_panel_open = true;
    app.focus = FocusPane::Tracks;
    app.sync_selection();

    let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 60, 17), 100, 30);

    assert!(handled);
    assert_eq!(app.focus, FocusPane::Tracks);
    assert_eq!(app.selected_playlist_row, 1);
}

#[test]
fn mouse_scroll_ignores_bottom_status_area_and_filter_mode() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    assert!(!app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 28), 100, 30,));
    app.filter_mode = true;
    assert!(!app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 1), 100, 30,));
}

#[test]
fn narrow_mouse_hit_testing_uses_stacked_panes() {
    assert_eq!(
        mouse_pane(10, 1, MouseLayout::new(74, 30, 2)),
        Some(FocusPane::Tree)
    );
    assert_eq!(
        mouse_pane(10, 20, MouseLayout::new(74, 30, 2)),
        Some(FocusPane::Tracks)
    );
    assert_eq!(mouse_pane(10, 28, MouseLayout::new(74, 30, 2)), None);
}

#[test]
fn wide_mouse_hit_testing_uses_split_panes() {
    assert_eq!(
        mouse_pane(10, 20, MouseLayout::new(100, 30, 2)),
        Some(FocusPane::Tree)
    );
    assert_eq!(
        mouse_pane(60, 20, MouseLayout::new(100, 30, 2)),
        Some(FocusPane::Tracks)
    );
    assert_eq!(
        mouse_pane(90, 20, MouseLayout::new(100, 30, 2)),
        Some(FocusPane::Tracks)
    );
}

#[test]
fn mouse_hit_testing_uses_persisted_library_split_offset() {
    assert_eq!(
        mouse_pane(
            50,
            20,
            MouseLayout::new(100, 30, 2).with_pane_offsets(20, 0)
        ),
        Some(FocusPane::Tree)
    );
    assert_eq!(
        mouse_pane(
            60,
            20,
            MouseLayout::new(100, 30, 2).with_pane_offsets(20, 0)
        ),
        Some(FocusPane::Tracks)
    );
}

#[test]
fn mouse_hit_testing_ignores_bottom_info_and_input_rows() {
    assert_eq!(
        mouse_pane(60, 5, MouseLayout::new(100, 30, 2).with_info(true, true)),
        Some(FocusPane::Tracks)
    );
    assert_eq!(
        mouse_pane(10, 12, MouseLayout::new(100, 30, 2).with_info(true, true)),
        Some(FocusPane::Tree)
    );
    assert_eq!(
        mouse_pane(60, 15, MouseLayout::new(100, 30, 2).with_info(true, true)),
        None
    );
    assert_eq!(
        mouse_pane(60, 20, MouseLayout::new(100, 30, 2).with_info(true, true)),
        None
    );
    assert_eq!(
        mouse_pane(10, 28, MouseLayout::new(100, 30, 2).with_info(true, true)),
        None
    );
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

#[test]
fn render_keeps_tree_selection_padded_from_bottom_when_possible() {
    let mut tracks = Vec::new();
    for id in 1..=20 {
        let mut track = test_track(id, &format!("track {id}"));
        track.artist = Some(format!("Artist {id:02}"));
        track.album_artist = track.artist.clone();
        tracks.push(track);
    }
    let mut app = test_app(tracks);
    app.selected_tree = 10;
    app.sync_selection();
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &mut app)).unwrap();

    assert!(app.tree_state.offset() > 0);
    assert!(app.selected_tree - app.tree_state.offset() <= 4);
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
    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    app.sync_selection();
    app.selected_playlist_row = 10;
    app.apply_selection_state();
    let backend = TestBackend::new(80, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|frame| render_playlist_info_pane(frame, &mut app, Rect::new(0, 0, 80, 10), 78))
        .unwrap();

    assert!(app.playlist_state.offset() > 0);
    assert!(app.selected_playlist_row - app.playlist_state.offset() <= 4);
}

#[test]
fn inactive_pane_selection_is_visible() {
    assert_eq!(
        pane_highlight_style(true),
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    );
    assert_eq!(
        pane_highlight_style(false),
        Style::default().bg(Color::White).fg(Color::Black)
    );
}

#[test]
fn command_and_filter_focus_make_both_pane_selections_inactive() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    assert!(pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.command_mode = true;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.command_mode = false;
    app.focus = FocusPane::Tracks;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.playlist_panel_open = true;
    app.focus = FocusPane::Playlist;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(pane_active(&app, FocusPane::Playlist));

    app.filter_mode = true;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.filter_mode = false;
    app.rate_mode = true;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.rate_mode = false;
    app.command_focus = true;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));
}

#[test]
fn tab_keeps_both_pane_selections() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.focus = FocusPane::Tracks;
    app.selected_tree = 0;
    app.selected_track_row = 2;
    app.apply_selection_state();

    app.toggle_focus();

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.selected_tree, 0);
    assert_eq!(app.selected_track_row, 2);
    assert_eq!(app.tree_state.selected(), Some(0));
    assert_eq!(app.track_state.selected(), Some(2));
}

#[test]
fn changing_tree_selection_resets_track_selection() {
    let mut second_artist = test_track(2, "second artist track");
    second_artist.artist = Some("Other Artist".to_string());
    second_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), second_artist]);
    app.focus = FocusPane::Tracks;
    app.selected_track_row = 1;
    app.toggle_focus();

    app.move_down();

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
    assert_eq!(app.track_state.selected(), Some(1));
}

#[test]
fn current_tree_marker_uses_artist_when_collapsed() {
    let mut second_album = test_track(2, "second album track");
    second_album.album = Some("Another Album".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), second_album]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    assert!(app.tree_entry_is_current(&app.tree_entries()[0]));
}

#[test]
fn current_tree_marker_uses_album_when_artist_expanded() {
    let mut second_album = test_track(2, "second album track");
    second_album.album = Some("Another Album".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), second_album]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.expanded_artists.insert("Artist".to_string());
    app.sync_selection();

    assert!(!app.tree_entry_is_current(&app.tree_entries()[0]));
    assert!(!app.tree_entry_is_current(&app.tree_entries()[1]));
    assert!(app.tree_entry_is_current(&app.tree_entries()[2]));
}

#[test]
fn collapsing_artist_from_album_selects_artist_parent() {
    let mut second_album = test_track(2, "second album track");
    second_album.album = Some("Another Album".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), second_album]);
    app.expanded_artists.insert("Artist".to_string());
    app.sync_selection();
    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(
            |entry| matches!(entry, TreeEntry::Album { album, .. } if album == "Another Album"),
        )
        .unwrap();

    app.space_action();

    assert!(!app.expanded_artists.contains("Artist"));
    assert!(matches!(
        app.selected_tree_entry(),
        Some(TreeEntry::Artist { artist }) if artist == "Artist"
    ));
}

#[test]
fn compilations_artist_appears_first_and_preserves_normal_artist() {
    let mut compilation = test_track(1, "compilation track");
    compilation.compilation = true;
    compilation.artist = Some("Contributing Artist".to_string());
    compilation.album_artist = Some("Contributing Artist".to_string());
    let mut app = test_app(vec![compilation]);

    assert!(matches!(
        app.tree_entries().first(),
        Some(TreeEntry::Compilation)
    ));
    assert!(app.tree_entries().iter().any(|entry| {
        matches!(
            entry,
            TreeEntry::Artist { artist } if artist == "Contributing Artist"
        )
    }));

    let artist_position = app
        .tree_entries()
        .iter()
        .position(|entry| {
            matches!(
                entry,
                TreeEntry::Artist { artist } if artist == "Contributing Artist"
            )
        })
        .unwrap();
    app.selected_tree = artist_position;
    app.sync_selection();

    assert_eq!(app.selected_scope_tracks().len(), 1);
}

#[test]
fn compilations_entry_expands_to_albums() {
    let mut first = test_track(1, "first compilation track");
    first.compilation = true;
    first.album = Some("First Collection".to_string());
    let mut second = test_track(2, "second compilation track");
    second.compilation = true;
    second.album = Some("Second Collection".to_string());
    let mut app = test_app(vec![first, second]);

    assert!(matches!(
        app.tree_entries().first(),
        Some(TreeEntry::Compilation)
    ));
    assert!(!app
        .tree_entries()
        .iter()
        .any(|entry| { matches!(entry, TreeEntry::CompilationAlbum { .. }) }));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[+] Compilations"));

    app.space_action();

    assert!(app.compilations_expanded);
    assert!(app.tree_entries().iter().any(|entry| {
        matches!(
            entry,
            TreeEntry::CompilationAlbum { album, .. } if album == "First Collection"
        )
    }));
    assert!(app.tree_entries().iter().any(|entry| {
        matches!(
            entry,
            TreeEntry::CompilationAlbum { album, .. } if album == "Second Collection"
        )
    }));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[-] Compilations"));
}

#[test]
fn playlists_entry_expands_to_playlists_and_plays_tracks() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "playlist track"),
    ]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![2]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![1]);
    app.sync_selection();

    assert!(matches!(
        app.tree_entries().first(),
        Some(TreeEntry::Playlists)
    ));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[+] Playlists"));

    app.space_action();

    assert!(app.playlists_expanded);
    assert!(matches!(
        app.tree_entries().get(1),
        Some(TreeEntry::Playlist { name, .. }) if name == "Road"
    ));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[-] Playlists"));

    app.selected_tree = 1;
    app.sync_selection();
    assert_eq!(app.selected_scope_tracks()[0].0, 1);
    assert_eq!(app.playback_sequence_indices(), vec![1]);

    let conn = test_conn();
    app.activate(&conn).unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
}

#[test]
fn collapsing_playlists_from_playlist_selects_playlists_parent() {
    let mut app = test_app(vec![test_track(1, "playlist track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.playlists_expanded = true;
    app.sync_selection();
    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();

    app.space_action();

    assert!(!app.playlists_expanded);
    assert!(matches!(
        app.selected_tree_entry(),
        Some(TreeEntry::Playlists)
    ));
}

#[test]
fn playlist_tree_track_pane_uses_playlist_row_style() {
    let mut first = test_track(1, "first track");
    first.track_number = Some(7);
    let mut second = test_track(2, "second track");
    second.track_number = Some(9);
    let mut app = test_app(vec![first, second]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1, 2]);
    app.playlist_track_entry_ids.insert(7, vec![11, 12]);
    app.playlist_track_indices.insert(7, vec![0, 1]);
    app.playlists_expanded = true;
    app.sync_selection();
    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.sync_selection();

    assert!(matches!(
        app.track_rows().first(),
        Some(TrackRow::PlaylistTrack {
            position: 1,
            track_index: 0,
            ..
        })
    ));
    let first_line = line_text(&playlist_track_line(&app, 0, 7, 11, 1, 40));
    let second_line = line_text(&playlist_track_line(&app, 1, 7, 12, 2, 40));

    assert!(first_line.contains("01. Artist - first track"));
    assert!(second_line.contains("02. Artist - second track"));
    assert!(first_line.ends_with("1:40"));
    assert!(!first_line.contains("07."));
    assert!(!second_line.contains("09."));
    assert!(!first_line.contains("x0"));
}

#[test]
fn duplicate_playlist_entries_mark_only_the_active_occurrence() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1, 1]);
    app.playlist_track_entry_ids.insert(7, vec![11, 12]);
    app.playlist_track_indices.insert(7, vec![0, 0]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: Some(PlaybackSource::PlaylistTrack {
            playlist_id: 7,
            playlist_track_id: 12,
        }),
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    let first = line_text(&playlist_track_line(&app, 0, 7, 11, 1, 40));
    let second = line_text(&playlist_track_line(&app, 0, 7, 12, 2, 40));

    assert!(first.starts_with("  01."));
    assert!(second.starts_with("> 02."));
}

#[test]
fn duplicate_playlist_playback_advances_by_entry_identity() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1, 1]);
    app.playlist_track_entry_ids.insert(7, vec![11, 12]);
    app.playlist_track_indices.insert(7, vec![0, 0]);
    app.playlists_expanded = true;
    app.sync_selection();
    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.sync_selection();
    app.current = Some(PlayingTrack {
        index: 0,
        source: Some(PlaybackSource::PlaylistTrack {
            playlist_id: 7,
            playlist_track_id: 11,
        }),
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    let next = app.next_playback_entry(1).unwrap();

    assert_eq!(next.track_index, 0);
    assert_eq!(
        next.source,
        Some(PlaybackSource::PlaylistTrack {
            playlist_id: 7,
            playlist_track_id: 12
        })
    );
}

#[test]
fn playlist_playback_does_not_mark_library_track_row_current() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: Some(PlaybackSource::PlaylistTrack {
            playlist_id: 7,
            playlist_track_id: 11,
        }),
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    let library_line = line_text(&track_line(&app, 0, false, 40));
    let playlist_line = line_text(&playlist_track_line(&app, 0, 7, 11, 1, 40));

    assert!(library_line.starts_with("  01."));
    assert!(playlist_line.starts_with("> 01."));
}

#[test]
fn playlist_playback_marks_playlist_tree_not_artist_tree() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: Some(PlaybackSource::PlaylistTrack {
            playlist_id: 7,
            playlist_track_id: 11,
        }),
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.sync_selection();

    assert!(matches!(app.tree_entries()[0], TreeEntry::Playlists));
    assert!(app.tree_entry_is_current(&app.tree_entries()[0]));
    assert!(app
        .tree_entries()
        .iter()
        .filter(|entry| matches!(entry, TreeEntry::Artist { .. }))
        .all(|entry| !app.tree_entry_is_current(entry)));
}

#[test]
fn library_playback_does_not_mark_playlist_track_row_current() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    let library_line = line_text(&track_line(&app, 0, false, 40));
    let playlist_line = line_text(&playlist_track_line(&app, 0, 7, 11, 1, 40));

    assert!(library_line.starts_with("> 01."));
    assert!(playlist_line.starts_with("  01."));
}

#[test]
fn library_playback_marks_artist_tree_not_playlist_tree() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    app.playlist_track_ids.insert(7, vec![1]);
    app.playlist_track_entry_ids.insert(7, vec![11]);
    app.playlist_track_indices.insert(7, vec![0]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.sync_selection();

    assert!(matches!(app.tree_entries()[0], TreeEntry::Playlists));
    assert!(!app.tree_entry_is_current(&app.tree_entries()[0]));
    assert!(app
        .tree_entries()
        .iter()
        .filter(|entry| matches!(entry, TreeEntry::Artist { .. }))
        .any(|entry| app.tree_entry_is_current(entry)));
}

#[test]
fn top_level_playlists_track_pane_groups_by_playlist() {
    let mut first = test_track(1, "first track");
    first.track_number = Some(7);
    let mut second = test_track(2, "second track");
    second.track_number = Some(9);
    let mut third = test_track(3, "third track");
    third.duration_ms = Some(200_000);
    let mut app = test_app(vec![first, second, third]);
    app.playlists = vec![
        db::Playlist {
            id: 7,
            name: "Road".to_string(),
        },
        db::Playlist {
            id: 8,
            name: "Night".to_string(),
        },
    ];
    app.playlist_track_ids.insert(7, vec![1, 2]);
    app.playlist_track_entry_ids.insert(7, vec![11, 12]);
    app.playlist_track_indices.insert(7, vec![0, 1]);
    app.playlist_track_ids.insert(8, vec![3, 1]);
    app.playlist_track_entry_ids.insert(8, vec![21, 22]);
    app.playlist_track_indices.insert(8, vec![2, 0]);
    app.sync_selection();

    assert!(matches!(
        app.selected_tree_entry(),
        Some(TreeEntry::Playlists)
    ));
    assert!(matches!(
        app.track_rows().first(),
        Some(TrackRow::PlaylistHeader { name, .. }) if name == "Road"
    ));
    assert!(matches!(
        app.track_rows().get(1),
        Some(TrackRow::PlaylistTrack {
            position: 1,
            track_index: 0,
            ..
        })
    ));
    assert!(matches!(
        app.track_rows().get(3),
        Some(TrackRow::PlaylistHeader { name, duration_ms }) if name == "Night" && *duration_ms == 300_000
    ));
    assert!(matches!(
        app.track_rows().get(4),
        Some(TrackRow::PlaylistTrack {
            position: 1,
            track_index: 2,
            ..
        })
    ));

    let header = line_text(&playlist_header_line("Road", 200_000, 40));
    let track = line_text(&playlist_track_line(&app, 0, 7, 11, 1, 40));

    assert!(header.starts_with("Road"));
    assert!(header.ends_with("3:20"));
    assert!(track.contains("01. Artist - first track"));
    assert!(!track.contains("07."));
}

#[test]
fn expanded_compilation_marks_current_album() {
    let mut first = test_track(1, "first compilation track");
    first.compilation = true;
    first.album = Some("First Collection".to_string());
    let mut second = test_track(2, "second compilation track");
    second.compilation = true;
    second.album = Some("Second Collection".to_string());
    let mut app = test_app(vec![first, second]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    assert!(app.tree_entry_is_current(&app.tree_entries()[0]));
    app.space_action();

    assert!(!app.tree_entry_is_current(&app.tree_entries()[0]));
    assert!(app.tree_entries().iter().any(|entry| {
        matches!(
            entry,
            TreeEntry::CompilationAlbum { album, .. }
                if album == "Second Collection" && app.tree_entry_is_current(entry)
        )
    }));
}

#[test]
fn enter_on_compilations_plays_first_compilation_track() {
    let mut compilation = test_track(2, "compilation track");
    compilation.compilation = true;
    let mut app = test_app(vec![test_track(1, "regular track"), compilation]);
    let conn = test_conn();

    app.activate(&conn).unwrap();

    assert!(matches!(
        app.tree_entries().first(),
        Some(TreeEntry::Compilation)
    ));
    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
}

#[test]
fn compilation_view_groups_albums_across_contributing_artists() {
    let mut first = test_track(1, "esper one");
    first.compilation = true;
    first.artist = Some("Vangelis".to_string());
    first.album = Some("Blade Runner Esper Edition".to_string());
    first.album_artist = Some("Vangelis".to_string());

    let mut other_album = test_track(2, "elsewhere");
    other_album.compilation = true;
    other_album.artist = Some("Another Artist".to_string());
    other_album.album = Some("Other Album".to_string());
    other_album.album_artist = Some("Another Artist".to_string());

    let mut second = test_track(3, "esper two");
    second.compilation = true;
    second.artist = Some("Dialog".to_string());
    second.album = Some("Blade Runner Esper Edition".to_string());
    second.album_artist = Some("Dialog".to_string());
    second.track_number = Some(2);

    let app = test_app(vec![first, other_album, second]);

    let album_headers: Vec<String> = app
        .track_rows()
        .iter()
        .filter_map(|row| match row {
            TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
            _ => None,
        })
        .collect();
    let track_indices: Vec<usize> = app
        .track_rows()
        .iter()
        .filter_map(|row| match row {
            TrackRow::Track { track_index, .. } => Some(*track_index),
            _ => None,
        })
        .collect();

    assert_eq!(
        album_headers,
        vec!["Blade Runner Esper Edition", "Other Album"]
    );
    assert_eq!(track_indices, vec![0, 2, 1]);
}

#[test]
fn compilation_view_merges_same_album_across_roots() {
    let mut vocal = test_track(1, "esper vocal");
    vocal.compilation = true;
    vocal.album = Some("Blade Runner Esper Edition".to_string());
    vocal.library_root = Some("/tmp/Vocal".to_string());

    let mut instrumental = test_track(2, "esper instrumental");
    instrumental.compilation = true;
    instrumental.album = Some("Blade Runner Esper Edition".to_string());
    instrumental.library_root = Some("/tmp/Instrumental".to_string());

    let app = test_app(vec![vocal, instrumental]);

    let album_headers: Vec<String> = app
        .track_rows()
        .iter()
        .filter_map(|row| match row {
            TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
            _ => None,
        })
        .collect();
    let track_indices: Vec<usize> = app
        .track_rows()
        .iter()
        .filter_map(|row| match row {
            TrackRow::Track { track_index, .. } => Some(*track_index),
            _ => None,
        })
        .collect();

    assert_eq!(album_headers, vec!["Blade Runner Esper Edition"]);
    assert_eq!(track_indices, vec![0, 1]);
}

#[test]
fn expanded_artist_merges_same_album_across_roots() {
    let mut vocal = test_track(1, "first side");
    vocal.artist = Some("Moby".to_string());
    vocal.album_artist = Some("Moby".to_string());
    vocal.album = Some("All Visible Objects".to_string());
    vocal.library_root = Some("/tmp/Vocal".to_string());

    let mut instrumental = test_track(2, "second side");
    instrumental.artist = Some("Moby".to_string());
    instrumental.album_artist = Some("Moby".to_string());
    instrumental.album = Some("All Visible Objects".to_string());
    instrumental.library_root = Some("/tmp/Instrumental".to_string());

    let mut app = test_app(vec![vocal, instrumental]);
    app.expanded_artists.insert("Moby".to_string());
    app.sync_selection();

    let album_entries: Vec<String> = app
        .tree_entries()
        .iter()
        .filter_map(|entry| match entry {
            TreeEntry::Album { album, .. } => Some(album.clone()),
            _ => None,
        })
        .collect();
    let album_headers: Vec<String> = app
        .track_rows()
        .iter()
        .filter_map(|row| match row {
            TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(album_entries, vec!["All Visible Objects"]);
    assert_eq!(album_headers, vec!["All Visible Objects"]);
}

#[test]
fn enter_on_artist_plays_first_listed_track() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    let conn = test_conn();
    app.sync_selection();

    app.activate(&conn).unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(0));
    assert_eq!(app.focus, FocusPane::Tree);
}

#[test]
fn selecting_current_track_does_not_change_focus() {
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.focus = FocusPane::Tree;

    app.select_track_index(1);

    assert_eq!(app.selected_track_row, 2);
    assert_eq!(app.focus, FocusPane::Tree);
}

#[test]
fn playback_moves_active_selection_to_played_track() {
    let mut other_artist = test_track(2, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
    let conn = test_conn();
    app.sync_selection();

    app.play_index(&conn, 1).unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
    assert_eq!(app.focus, FocusPane::Tracks);
}

#[test]
fn next_track_moves_active_selection_to_played_track() {
    let mut other_artist = test_track(2, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
    let conn = test_conn();
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.play_next(&conn).unwrap();

    assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
    assert_eq!(app.focus, FocusPane::Tracks);
}

#[test]
fn user_can_select_current_track_explicitly() {
    let mut other_artist = test_track(2, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.select_current_track();

    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
    assert_eq!(app.focus, FocusPane::Tree);
}

#[test]
fn current_track_selection_uses_media_item_id_after_reorder() {
    let first = test_track(1, "first track");
    let second = test_track(2, "second track");
    let mut app = test_app(vec![first.clone(), second.clone()]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: first.clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.tracks = vec![second, first];
    app.rebuild_search_cache();
    app.sync_selection();

    app.select_current_track();

    assert_eq!(app.selected_playable_media_item_id(), Some(1));
    assert_eq!(app.selected_track_row, 2);
}

#[test]
fn preserving_browser_selection_uses_media_item_id_after_reorder() {
    let first = test_track(1, "first track");
    let second = test_track(2, "second track");
    let mut app = test_app(vec![first.clone(), second.clone()]);
    app.focus = FocusPane::Tracks;
    app.selected_track_row = 2;
    app.apply_selection_state();
    let selected_tree_entry = app.selected_tree_entry().cloned();
    let selected_media_item_id = app.selected_playable_media_item_id();

    app.tracks = vec![second, first];
    app.rebuild_search_cache();
    app.sync_selection_preserving_browser_anchors(
        selected_tree_entry.as_ref(),
        selected_media_item_id,
    );

    assert_eq!(app.selected_playable_media_item_id(), Some(2));
    assert_eq!(app.selected_track_row, 1);
}

#[test]
fn playback_anchor_uses_media_item_id_after_reorder() {
    let first = test_track(1, "first track");
    let second = test_track(2, "second track");
    let mut app = test_app(vec![first.clone(), second.clone()]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: first.clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.tracks = vec![second, first];
    app.rebuild_search_cache();
    app.sync_selection();

    assert_eq!(app.next_playback_index(1), None);
    assert_eq!(app.next_playback_index(-1), Some(0));
}

#[test]
fn uppercase_i_selects_current_track_after_lowercase_i_toggles_info() {
    let mut other_artist = test_track(2, "other artist track");
    other_artist.artist = Some("Other Artist".to_string());
    other_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
    let conn = test_conn();
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .unwrap();

    assert!(!app.info_panel_visible);
    assert_eq!(app.selected_tree, 0);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.selected_tree, 1);
    assert_eq!(app.selected_track_row, 1);
}

#[test]
fn pause_suspends_player_until_resume() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.player.play().unwrap();

    app.suspend_current().unwrap();

    assert_eq!(app.logical_state(), PlaybackState::Paused);
    assert_eq!(app.player.state(), PlaybackState::Stopped);
    assert_eq!(app.suspended_position_ms, Some(0));

    app.resume_current().unwrap();

    assert_eq!(app.logical_state(), PlaybackState::Playing);
    assert_eq!(app.suspended_position_ms, None);
}

#[test]
fn failed_seek_during_resume_keeps_app_paused() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(FailingSeekPlayer);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });
    app.suspended_position_ms = Some(50_000);

    app.resume_current().unwrap();

    assert_eq!(app.logical_state(), PlaybackState::Paused);
    assert_eq!(app.suspended_position_ms, Some(50_000));
    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 50_000);
    assert!(app.message.contains("seek failed"));
    assert!(app.message.contains("decoder refused seek"));
}

#[test]
fn disconnected_audio_output_pauses_current_track_without_advancing() {
    let conn = test_conn();
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "second track"),
    ]);
    app.player = Box::new(OutputFailedPlayer);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 49_000,
        listened_ms: 49_000,
    });

    assert!(app.update_playback(&conn).unwrap());

    assert_eq!(app.current.as_ref().unwrap().index, 0);
    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 50_000);
    assert_eq!(app.suspended_position_ms, Some(50_000));
    assert_eq!(app.logical_state(), PlaybackState::Paused);
    assert_eq!(app.message, "audio output disconnected; paused");
}

#[test]
fn stalled_audio_output_shows_inactive_until_progress_resumes() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(StalledOutputPlayer { playing: false });
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });

    let stalled = line_text(&playback_line(&app, 80));
    assert!(stalled.contains(" | 0:50 / 1:40 ["));

    app.player.play().unwrap();

    let resumed = line_text(&playback_line(&app, 80));
    assert!(resumed.contains(" > 0:50 / 1:40 ["));
}

#[test]
fn stalled_audio_output_publishes_inactive_media_state_until_progress_resumes() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(StalledOutputPlayer { playing: false });
    app.integration = Box::new(RecordingIntegration {
        events: Rc::clone(&events),
    });
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });

    app.sync_integration_playback(true);
    app.player.play().unwrap();
    app.sync_integration_playback(true);

    assert_eq!(
        events.borrow().as_slice(),
        &[
            IntegrationEvent::Playback(crate::integration::PlaybackSnapshot {
                state: PlaybackState::Paused,
                position_ms: 50_000,
            }),
            IntegrationEvent::Playback(crate::integration::PlaybackSnapshot {
                state: PlaybackState::Playing,
                position_ms: 50_000,
            }),
        ]
    );
}

#[test]
fn unavailable_replacement_output_keeps_app_paused() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.player = Box::new(OutputFailedPlayer);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });
    app.suspended_position_ms = Some(50_000);

    app.resume_current().unwrap();

    assert_eq!(app.suspended_position_ms, Some(50_000));
    assert_eq!(app.logical_state(), PlaybackState::Paused);
    assert!(app.message.contains("could not resume"));
    assert!(app.message.contains("no audio output available"));
}

#[test]
fn play_entry_starts_player_backend() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.play_index(&conn, 0).unwrap();

    assert_eq!(app.player.state(), PlaybackState::Playing);
    assert_eq!(app.logical_state(), PlaybackState::Playing);
}

#[test]
fn relative_seek_while_paused_uses_suspended_position() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });
    app.suspended_position_ms = Some(50_000);

    app.seek_relative(5).unwrap();

    assert_eq!(app.suspended_position_ms, Some(55_000));
    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 55_000);
}

#[test]
fn repeated_integration_failures_do_not_keep_overwriting_messages() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.integration = Box::new(FailingIntegration);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.publish_track_changed();
    assert!(app.message.contains("track integration unavailable"));

    app.message = String::from("normal playback message");
    app.sync_integration_playback(true);

    assert_eq!(app.message, "normal playback message");
}

#[test]
fn track_changed_event_uses_owned_track_snapshot() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut track = test_track(1, "first track");
    track.cover_path = Some(String::from("/tmp/cover.jpg"));
    let mut app = test_app(vec![track]);
    app.integration = Box::new(RecordingIntegration {
        events: Rc::clone(&events),
    });
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });

    app.publish_track_changed();

    assert_eq!(
        events.borrow().as_slice(),
        &[IntegrationEvent::TrackChanged(TrackSnapshot {
            title: Some(String::from("first track")),
            artist: Some(String::from("Artist")),
            album: Some(String::from("Album")),
            duration_ms: Some(100_000),
            artwork_path: Some(PathBuf::from("/tmp/cover.jpg")),
        })]
    );
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
    assert!(app.expanded_artists.contains("Artist"));
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
    assert_eq!(app.selected_tree, 0);
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

    app.selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Artist { artist } if artist == "Other Artist"))
        .unwrap();
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

    let app = App::new(&conn, &test_paths()).unwrap();

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

    let app = App::new(&conn, &test_paths()).unwrap();

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

fn test_app(tracks: Vec<LibraryTrack>) -> App {
    let mut app = App {
        paths: test_paths(),
        tracks,
        playlists: Vec::new(),
        playlist_track_ids: HashMap::new(),
        playlist_track_entry_ids: HashMap::new(),
        playlist_track_indices: HashMap::new(),
        view: ViewCache::default(),
        tree_state: ListState::default(),
        track_state: ListState::default(),
        playlist_state: ListState::default(),
        keymap_state: ListState::default(),
        selected_tree: 0,
        selected_track_row: 0,
        selected_playlist_row: 0,
        selected_keymap_row: 0,
        expanded_artists: HashSet::new(),
        compilations_expanded: false,
        playlists_expanded: false,
        expanded_playlists: HashSet::new(),
        active_playlist_id: None,
        playlist_panel_open: false,
        keymap_panel_open: false,
        focus: FocusPane::Tree,
        filter: String::new(),
        restore_filter: true,
        restore_track: true,
        filter_mode: false,
        rate_input: String::new(),
        rate_mode: false,
        command: String::new(),
        command_mode: false,
        command_output: Vec::new(),
        command_output_kind: CommandOutputKind::Text,
        command_roots: Vec::new(),
        command_selected: 0,
        command_focus: false,
        key_bindings: HashMap::new(),
        keymap_capture_action: None,
        library_job: None,
        info_panel_visible: true,
        startup_info_visible: false,
        library_pane_percent_offset: 0,
        info_pane_height_offset: 0,
        play_target: PlayTarget::Library,
        continuous: true,
        repeat: false,
        shuffle: false,
        shuffle_seed: 0x476d_7573_2026_0528,
        shuffle_scope: Vec::new(),
        shuffle_order: Vec::new(),
        player: Box::new(NullPlayer::default()),
        integration: Box::new(NoopIntegration),
        current: None,
        suspended_position_ms: None,
        last_integration_state: None,
        last_integration_position_s: None,
        integration_error_reported: false,
        track_notifications_visible: true,
        transient_status: None,
        message: String::new(),
    };
    app.rebuild_search_cache();
    app.sync_selection();
    app
}

fn test_paths() -> AppPaths {
    AppPaths {
        data_dir: PathBuf::from("/tmp/gmus-test"),
        db_path: PathBuf::from("/tmp/gmus-test/gmus.sqlite3"),
        art_dir: PathBuf::from("/tmp/gmus-test/art"),
    }
}

fn wait_for_library_job(app: &mut App, conn: &Connection) -> bool {
    for _ in 0..50 {
        if app.poll_library_job(conn).unwrap() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn test_track(id: i64, title: &str) -> LibraryTrack {
    LibraryTrack {
        media_item_id: id,
        location_id: id,
        path: format!("/tmp/{title}.flac"),
        library_root: None,
        title: Some(title.to_string()),
        artist: Some("Artist".to_string()),
        album: Some("Album".to_string()),
        album_artist: None,
        album_year: Some(2018),
        release_date: Some("2018-05-11".to_string()),
        composer: None,
        genre: None,
        cover_path: None,
        track_number: Some(id),
        track_total: Some(10),
        disc_number: None,
        disc_total: None,
        duration_ms: Some(100_000),
        compilation: false,
        play_count: 0,
    }
}

fn test_track_metadata(path: &str, title: &str, track_number: i64) -> crate::media::TrackMetadata {
    crate::media::TrackMetadata {
        path: path.into(),
        file_size: 10,
        modified_at: Some(1),
        title: Some(title.to_string()),
        artist: Some("Artist".to_string()),
        album: Some("Album".to_string()),
        album_artist: None,
        album_year: Some(2018),
        release_date: Some("2018-05-11".to_string()),
        composer: None,
        genre: None,
        track_number: Some(track_number),
        track_total: Some(10),
        disc_number: None,
        disc_total: None,
        duration_ms: Some(100_000),
        compilation: false,
    }
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn lines_text(lines: &[Line<'_>]) -> String {
    lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
}

fn buffer_row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
    (0..width)
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

fn playlist_text(app: &App) -> String {
    app.view
        .playlist_entries
        .iter()
        .map(|entry| playlist_entry_text(app, entry))
        .collect::<Vec<_>>()
        .join("\n")
}

fn keymap_text(app: &App) -> String {
    lines_text(&keymap_lines(app, 80))
}

fn playback_bar_width(text: &str) -> usize {
    let start = text.find('[').unwrap();
    let end = text[start..].find(']').unwrap() + start;
    display_width(&text[start + 1..end])
}

fn test_conn() -> Connection {
    db::open_in_memory_for_tests().unwrap()
}

fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

struct FailingSeekPlayer;

impl PlayerBackend for FailingSeekPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn play(&mut self) -> Result<()> {
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn seek(&mut self, _position: Duration) -> Result<()> {
        anyhow::bail!("decoder refused seek")
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::from_millis(197_500)
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn state(&self) -> PlaybackState {
        PlaybackState::Playing
    }
}

struct OutputFailedPlayer;

impl PlayerBackend for OutputFailedPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        anyhow::bail!("no audio output available")
    }

    fn play(&mut self) -> Result<()> {
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn seek(&mut self, _position: Duration) -> Result<()> {
        Ok(())
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::from_millis(50_000)
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn output_failed(&self) -> bool {
        true
    }

    fn state(&self) -> PlaybackState {
        PlaybackState::Stopped
    }
}

struct StalledOutputPlayer {
    playing: bool,
}

impl PlayerBackend for StalledOutputPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn play(&mut self) -> Result<()> {
        self.playing = true;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn seek(&mut self, _position: Duration) -> Result<()> {
        Ok(())
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::from_millis(50_000)
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn state(&self) -> PlaybackState {
        if self.playing {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }
}

struct FailingIntegration;

impl Integration for FailingIntegration {
    fn next_command(&mut self) -> Option<IntegrationCommand> {
        None
    }

    fn publish_event(&mut self, _event: &IntegrationEvent) -> Result<()> {
        anyhow::bail!("integration unavailable")
    }
}

struct RecordingIntegration {
    events: Rc<RefCell<Vec<IntegrationEvent>>>,
}

impl Integration for RecordingIntegration {
    fn next_command(&mut self) -> Option<IntegrationCommand> {
        None
    }

    fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
        self.events.borrow_mut().push(event.clone());
        Ok(())
    }
}
