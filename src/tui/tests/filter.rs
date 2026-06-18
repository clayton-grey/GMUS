use super::*;

#[test]
fn filter_line_has_its_own_prompt() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.enter_filter();
    app.input.set_filter("beat".to_string());
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
    app.input.enter_filter();
    let placeholder = filter_line(&app, 40);

    assert_eq!(line_text(&placeholder), " filter: none_");
    assert_eq!(
        placeholder.spans[2].style,
        Style::default().fg(Color::Gray).bg(Color::Blue)
    );

    app.input.finish_filter();
    app.input.set_filter("beat".to_string());

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
    app.input
        .set_filter("genre:ambient year:2010..2020 root:instrumental".to_string());
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
    app.input.set_filter(
        "artist:\"Other Artist\" -genre:podcast compilation:false plays:>5".to_string(),
    );
    app.sync_selection();

    assert_eq!(app.playback_sequence_indices(), vec![0]);
}

#[test]
fn unknown_filter_field_shows_hint_and_matches_nothing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.set_filter("mood:blue".to_string());
    app.sync_selection();

    let query = FilterQuery::parse(app.input.filter());

    assert_eq!(app.playback_sequence_indices(), Vec::<usize>::new());
    assert_eq!(query.warning(), Some("unknown filter field: mood"));
}

#[test]
fn malformed_numeric_range_shows_hint_and_matches_nothing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.set_filter("year:abc..2020".to_string());
    app.sync_selection();

    let query = FilterQuery::parse(app.input.filter());

    assert_eq!(app.playback_sequence_indices(), Vec::<usize>::new());
    assert_eq!(query.warning(), Some("expected a number for year"));
}

#[test]
fn filter_info_pane_hints_fields_while_typing() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.input.enter_filter();
    app.input.set_filter("genre:ambient".to_string());

    let text = lines_text(&filter_info_lines(&app, 80, 8));

    assert!(app.info_area_visible());
    assert_eq!(command_info_title(&app), "Filter");
    assert!(text.contains("fields: title artist album"));
    assert!(text.contains("examples: genre:ambient year:2010..2020"));
}

#[test]
fn typed_filter_updates_visible_tracks_before_confirmation() {
    let mut app = test_app(vec![
        test_track(1, "alpha song"),
        test_track(2, "beta song"),
    ]);
    let conn = test_conn();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .unwrap();
    for key in "title:a".chars() {
        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            .unwrap();
    }

    assert_eq!(app.input.kind(), InputKind::Filter);
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.filter(), "title:al");
    assert_eq!(app.playback_sequence_indices(), &[0]);
}

#[test]
fn slash_reopens_filter_with_the_active_filter_cleared() {
    let conn = test_conn();
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    app.input.set_filter("keep".to_string());
    app.sync_selection();
    db::save_filter(&conn, "keep").unwrap();
    assert_eq!(app.playback_sequence_indices(), &[0]);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::Filter);
    assert!(app.input.filter().is_empty());
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    assert_eq!(db::saved_filter(&conn).unwrap().as_deref(), Some(""));
    assert_eq!(app.message, "typing filter");
}

#[test]
fn tab_confirms_filter_and_focuses_library() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.input.enter_filter();
    app.input.set_filter("keep".to_string());

    let should_quit = app
        .handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert!(!should_quit);
    assert_eq!(app.input.kind(), InputKind::None);
    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.browser.selected_tree(), 0);
    assert_eq!(app.browser.selected_track_row(), 1);
    assert_eq!(app.playback_sequence_indices(), &[0]);
}

#[test]
fn enter_confirms_filter_and_focuses_library() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.focus = FocusPane::Tracks;
    app.input.enter_filter();
    app.input.set_filter("keep".to_string());

    app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
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

    let app = test_app_from_db(&conn);

    assert_eq!(app.input.filter(), "keep");
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

    let app = test_app_from_db(&conn);

    assert!(app.input.filter().is_empty());
    assert_eq!(app.playback_sequence_indices(), vec![0, 1]);
}

#[test]
fn escape_clears_filter_entry() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.input.enter_filter();
    app.input.set_filter("keep".to_string());
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.input.filter().is_empty());
    assert_eq!(app.message, "filter cleared");
    assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    assert_eq!(db::saved_filter(&conn).unwrap().as_deref(), Some(""));
}

#[test]
fn escape_clears_active_filter_outside_filter_entry() {
    let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
    let conn = test_conn();
    app.input.set_filter("keep".to_string());
    app.sync_selection();

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.input.kind(), InputKind::None);
    assert!(app.input.filter().is_empty());
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
    app.input.set_filter("other".to_string());
    app.sync_selection();
    assert_eq!(
        app.selected_tree_entry().map(TreeEntry::artist),
        Some("Other Artist")
    );
    assert_eq!(app.selected_playable_track_index(), Some(1));

    app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert!(app.input.filter().is_empty());
    assert_eq!(
        app.selected_tree_entry().map(TreeEntry::artist),
        Some("Other Artist")
    );
    assert_eq!(app.selected_playable_track_index(), Some(1));
}
