use super::*;

#[test]
fn playback_sequence_respects_filter() {
    let mut app = test_app(vec![
        test_track(1, "keep one"),
        test_track(2, "skip this"),
        test_track(3, "keep two"),
    ]);
    app.input.set_filter("keep".to_string());
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

    assert!(!app.playback_mode.continuous());
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

    app.toggle_play_target();
    assert_eq!(app.playback_sequence_indices(), vec![0, 1]);

    app.toggle_play_target();
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

    app.toggle_repeat();
    assert_eq!(app.next_playback_index(1), Some(0));
}

#[test]
fn shuffled_playback_falls_back_to_selected_filtered_track() {
    let mut app = test_app(vec![
        test_track(1, "keep track"),
        test_track(2, "skip track"),
    ]);
    app.current = Some(PlayingTrack {
        index: 1,
        source: None,
        track: app.tracks[1].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.input.set_filter("keep".to_string());
    app.sync_selection();
    app.toggle_shuffle();

    assert_eq!(app.next_playback_index(1), Some(0));
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
fn long_event_loop_stall_counts_backend_playback_progress() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 1_000,
        listened_ms: 1_000,
    });

    app.current
        .as_mut()
        .unwrap()
        .tick_position(Duration::from_millis(31_000), PlaybackState::Playing);

    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 31_000);
    assert_eq!(app.current.as_ref().unwrap().listened_ms, 31_000);
}

#[test]
fn explicit_seek_realigns_progress_without_counting_seek_distance() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.player.play().unwrap();

    app.seek_to(30_000).unwrap();
    assert_eq!(app.current.as_ref().unwrap().listened_ms, 0);
    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 30_000);

    app.player.seek(Duration::from_millis(35_000)).unwrap();
    app.capture_current_progress();

    assert_eq!(app.current.as_ref().unwrap().listened_ms, 5_000);
    assert_eq!(app.current.as_ref().unwrap().last_position_ms, 35_000);
}

#[test]
fn backward_backend_jump_realigns_without_reducing_listened_time() {
    let mut app = test_app(vec![test_track(1, "first track")]);
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 30_000,
        listened_ms: 30_000,
    });

    let current = app.current.as_mut().unwrap();
    current.tick_position(Duration::from_millis(20_000), PlaybackState::Playing);
    current.tick_position(Duration::from_millis(25_000), PlaybackState::Playing);

    assert_eq!(current.last_position_ms, 25_000);
    assert_eq!(current.listened_ms, 35_000);
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

    app.toggle_continuous();
    app.transient_status = None;

    let line = playback_line(&app, 80);

    assert_eq!(
        line.spans[line.spans.len() - 5].style,
        Style::default().fg(Color::DarkGray)
    );
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
    app.integration.backend = Box::new(RecordingIntegration {
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
fn failed_record_play_during_stop_retains_current_for_retry() {
    let conn = test_conn();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let track = db::library_tracks(&conn)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut app = test_app(vec![track.clone()]);
    let mut invalid_track = track.clone();
    invalid_track.location_id += 1_000;
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: invalid_track,
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });
    app.suspended_position_ms = Some(50_000);

    assert!(app.stop_current(&conn).is_err());
    assert!(app.current.is_some());
    assert_eq!(app.suspended_position_ms, Some(50_000));

    app.current.as_mut().unwrap().track.location_id = track.location_id;
    app.stop_current(&conn).unwrap();

    assert!(app.current.is_none());
    assert_eq!(app.suspended_position_ms, None);
}

#[test]
fn failed_record_play_during_shutdown_retains_current_for_retry() {
    let conn = test_conn();
    db::upsert_track(
        &conn,
        &test_track_metadata("/tmp/first.flac", "first track", 1),
    )
    .unwrap();
    let track = db::library_tracks(&conn)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut app = test_app(vec![track.clone()]);
    let mut invalid_track = track.clone();
    invalid_track.location_id += 1_000;
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: invalid_track,
        last_position_ms: 50_000,
        listened_ms: 50_000,
    });
    app.suspended_position_ms = Some(50_000);

    assert!(app.shutdown(&conn).is_err());
    assert!(app.current.is_some());
    assert_eq!(app.suspended_position_ms, Some(50_000));

    app.current.as_mut().unwrap().track.location_id = track.location_id;
    app.shutdown(&conn).unwrap();

    assert!(app.current.is_none());
    assert_eq!(app.suspended_position_ms, None);
}

#[test]
fn quit_keys_signal_exit_without_immediate_shutdown() {
    let conn = test_conn();
    for key in [
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            source: None,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });
        app.player.play().unwrap();

        assert!(app.handle_key(&conn, key).unwrap());
        assert!(app.current.is_some());
        assert_eq!(app.player.state(), PlaybackState::Playing);
    }
}

#[test]
fn failed_replacement_track_load_publishes_stopped_integration_snapshot() {
    let conn = test_conn();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = test_app(vec![
        test_track(1, "first track"),
        test_track(2, "replacement track"),
    ]);
    app.player = Box::new(OutputFailedPlayer);
    app.integration.backend = Box::new(RecordingIntegration {
        events: Rc::clone(&events),
    });
    app.current = Some(PlayingTrack {
        index: 0,
        source: None,
        track: app.tracks[0].clone(),
        last_position_ms: 0,
        listened_ms: 0,
    });
    app.suspended_position_ms = Some(0);

    app.play_index(&conn, 1).unwrap();

    assert!(app.current.is_none());
    assert_eq!(
        events.borrow().as_slice(),
        &[IntegrationEvent::Playback(
            crate::integration::PlaybackSnapshot {
                state: PlaybackState::Stopped,
                position_ms: 0,
            }
        )]
    );
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
    app.integration.backend = Box::new(FailingIntegration);
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
    track.cover_path = Some(PathBuf::from("/tmp/cover.jpg"));
    let mut app = test_app(vec![track]);
    app.integration.backend = Box::new(RecordingIntegration {
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
