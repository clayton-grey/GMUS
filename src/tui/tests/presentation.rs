use super::*;

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
    app.toggle_play_target();
    app.toggle_play_target();
    app.toggle_repeat();
    app.toggle_shuffle();
    app.transient_status = None;

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
fn pane_resize_keys_adjust_library_boundary_for_tree_and_tracks() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);

    app.focus = FocusPane::Tree;
    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.layout.library_pane_percent_offset(), 2);
    assert!(app.message.contains("library pane larger"));
    assert_eq!(
        db::pane_layout(&conn).unwrap().library_percent_offset,
        app.layout.library_pane_percent_offset()
    );

    app.focus = FocusPane::Tracks;
    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.layout.library_pane_percent_offset(), 4);
    assert!(app.message.contains("tracks pane smaller"));

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.layout.library_pane_percent_offset(), 2);
    assert!(app.message.contains("tracks pane larger"));
}

#[test]
fn pane_resize_keys_adjust_bottom_management_pane_height() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.management_panel.show_keymap();
    app.focus = FocusPane::Keymap;

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.layout.info_pane_height_offset(), -1);
    assert!(app.message.contains("info pane smaller"));
    assert_eq!(
        db::pane_layout(&conn).unwrap().info_height_offset,
        app.layout.info_pane_height_offset()
    );

    app.handle_key(
        &conn,
        KeyEvent::new(KeyCode::Char('{'), KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.layout.info_pane_height_offset(), 0);
    assert!(app.message.contains("info pane larger"));
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
    app.layout = LayoutState::new(0, 0, layout::DEFAULT_COLUMN_LAYOUT_WIDTH, true);

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
    app.layout = LayoutState::new(0, 0, layout::DEFAULT_COLUMN_LAYOUT_WIDTH, true);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();

    assert!(!app.layout.startup_info_visible());
    assert!(lines_text(&metadata_lines(&app, 80)).contains("selected track"));
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
fn album_metadata_reflects_filtered_visible_tracks() {
    let mut hidden = test_track(1, "hidden track");
    hidden.album_year = Some(2018);
    hidden.duration_ms = Some(120_000);
    hidden.disc_number = Some(2);
    let mut visible = test_track(2, "visible track");
    visible.album_year = Some(2024);
    visible.duration_ms = Some(60_000);
    visible.disc_number = Some(1);
    let mut app = test_app(vec![hidden, visible]);

    app.input.set_filter("visible".to_string());
    app.sync_selection();

    assert!(matches!(
        app.track_rows().first(),
        Some(TrackRow::AlbumHeader {
            album_year: Some(2024),
            duration_ms: 60_000,
            ..
        })
    ));
    assert!(matches!(
        app.track_rows().get(1),
        Some(TrackRow::Track {
            show_disc_number: false,
            ..
        })
    ));
    assert!(!app
        .track_rows()
        .iter()
        .any(|row| matches!(row, TrackRow::DiscDivider { .. })));
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

    assert_eq!(app.browser.selected_track_row(), 1);
    app.focus = FocusPane::Tracks;
    app.move_down();
    assert_eq!(app.browser.selected_track_row(), 2);
    app.move_down();
    assert_eq!(app.browser.selected_track_row(), 4);
    app.move_up();
    assert_eq!(app.browser.selected_track_row(), 2);
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
    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
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
    assert_eq!(app.browser.selected_track_row(), 2);
}

#[test]
fn mouse_scroll_ignores_bottom_status_area_and_filter_mode() {
    let mut app = test_app(vec![test_track(1, "first track")]);

    assert!(!app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 28), 100, 30,));
    app.input.enter_filter();
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
    assert_eq!(
        mouse_pane(10, 20, MouseLayout::new(75, 30, 2)),
        Some(FocusPane::Tracks)
    );
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
fn mouse_hit_testing_uses_configured_column_layout_width() {
    assert_eq!(
        mouse_pane(
            10,
            20,
            MouseLayout::new(100, 30, 2).with_column_layout_width(120)
        ),
        Some(FocusPane::Tracks)
    );
    assert_eq!(
        mouse_pane(
            10,
            1,
            MouseLayout::new(75, 30, 2).with_column_layout_width(75)
        ),
        Some(FocusPane::Tree)
    );
    assert_eq!(
        mouse_pane(
            10,
            20,
            MouseLayout::new(76, 30, 2).with_column_layout_width(75)
        ),
        Some(FocusPane::Tree)
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
fn render_keeps_tree_selection_padded_from_bottom_when_possible() {
    let mut tracks = Vec::new();
    for id in 1..=20 {
        let mut track = test_track(id, &format!("track {id}"));
        track.artist = Some(format!("Artist {id:02}"));
        track.album_artist = track.artist.clone();
        tracks.push(track);
    }
    let mut app = test_app(tracks);
    app.browser.select_tree(10);
    app.sync_selection();
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &mut app)).unwrap();

    assert!(app.tree_state.offset() > 0);
    assert!(app.browser.selected_tree() - app.tree_state.offset() <= 4);
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
    let conn = test_conn();

    assert!(pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.input.enter_command();
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.input.cancel_command();
    app.focus = FocusPane::Tracks;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.management_panel.show_playlist();
    app.focus = FocusPane::Playlist;
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(pane_active(&app, FocusPane::Playlist));

    app.input.enter_filter();
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.input.finish_filter();
    app.input.enter_rate();
    assert!(!pane_active(&app, FocusPane::Tree));
    assert!(!pane_active(&app, FocusPane::Tracks));
    assert!(!pane_active(&app, FocusPane::Playlist));

    app.input.finish_rate();
    db::upsert_library_root(&conn, Path::new("/tmp/music")).unwrap();
    set_command_input(&mut app, String::from("library"));
    app.execute_command(&conn);
    assert!(app.command_output.is_focused());
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
    app.browser.select_tree(0);
    app.browser.select_track_row(2);
    app.apply_selection_state();

    app.toggle_focus();

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.browser.selected_tree(), 0);
    assert_eq!(app.browser.selected_track_row(), 2);
    assert_eq!(app.tree_state.selected(), Some(0));
    assert_eq!(app.track_state.selected(), Some(2));
}
