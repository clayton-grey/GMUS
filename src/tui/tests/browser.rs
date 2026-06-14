use super::*;

#[test]
fn changing_tree_selection_resets_track_selection() {
    let mut second_artist = test_track(2, "second artist track");
    second_artist.artist = Some("Other Artist".to_string());
    second_artist.album_artist = Some("Other Artist".to_string());
    let mut app = test_app(vec![test_track(1, "first track"), second_artist]);
    app.focus = FocusPane::Tracks;
    app.browser.select_track_row(1);
    app.toggle_focus();

    app.move_down();

    assert_eq!(app.focus, FocusPane::Tree);
    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
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
    app.browser.expand_artist("Artist".to_string());
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
    app.browser.expand_artist("Artist".to_string());
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(
            |entry| matches!(entry, TreeEntry::Album { album, .. } if album == "Another Album"),
        )
        .unwrap();
    app.browser.select_tree(selected_tree);

    app.space_action();

    assert!(!app.browser.artist_expanded("Artist"));
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
    app.browser.select_tree(artist_position);
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

    assert!(app.browser.compilations_expanded());
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
    set_playlist_cache(&mut app, 7, vec![2], vec![11], vec![1]);
    app.sync_selection();

    assert!(matches!(
        app.tree_entries().first(),
        Some(TreeEntry::Playlists)
    ));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[+] Playlists"));

    app.space_action();

    assert!(app.browser.playlists_expanded());
    assert!(matches!(
        app.tree_entries().get(1),
        Some(TreeEntry::Playlist { name, .. }) if name == "Road"
    ));
    assert!(line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[-] Playlists"));

    app.browser.select_tree(1);
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
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
    app.browser.set_playlists_expanded(true);
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.browser.select_tree(selected_tree);

    app.space_action();

    assert!(!app.browser.playlists_expanded());
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
    set_playlist_cache(&mut app, 7, vec![1, 2], vec![11, 12], vec![0, 1]);
    app.browser.set_playlists_expanded(true);
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.browser.select_tree(selected_tree);
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
    set_playlist_cache(&mut app, 7, vec![1, 1], vec![11, 12], vec![0, 0]);
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
    set_playlist_cache(&mut app, 7, vec![1, 1], vec![11, 12], vec![0, 0]);
    app.browser.set_playlists_expanded(true);
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.browser.select_tree(selected_tree);
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
fn shuffled_duplicate_playlist_playback_advances_by_entry_identity() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1, 1], vec![11, 12], vec![0, 0]);
    app.browser.set_playlists_expanded(true);
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { name, .. } if name == "Road"))
        .unwrap();
    app.browser.select_tree(selected_tree);
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
    app.toggle_shuffle();

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
fn preserving_browser_selection_keeps_duplicate_playlist_entry_identity() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1, 1], vec![11, 12], vec![0, 0]);
    app.browser.set_playlists_expanded(true);
    app.sync_selection();
    let selected_tree = app
        .tree_entries()
        .iter()
        .position(|entry| matches!(entry, TreeEntry::Playlist { playlist_id: 7, .. }))
        .unwrap();
    app.browser.select_tree(selected_tree);
    app.sync_selection();
    app.browser.select_track_row(1);

    app.sync_selection_preserving_browser_selection();

    assert!(matches!(
        app.track_rows().get(app.browser.selected_track_row()),
        Some(TrackRow::PlaylistTrack {
            playlist_track_id: 12,
            ..
        })
    ));
}

#[test]
fn playlist_playback_does_not_mark_library_track_row_current() {
    let mut app = test_app(vec![test_track(1, "looped track")]);
    app.playlists = vec![db::Playlist {
        id: 7,
        name: "Road".to_string(),
    }];
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
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
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
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
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
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
    set_playlist_cache(&mut app, 7, vec![1], vec![11], vec![0]);
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
    set_playlist_cache(&mut app, 7, vec![1, 2], vec![11, 12], vec![0, 1]);
    set_playlist_cache(&mut app, 8, vec![3, 1], vec![21, 22], vec![2, 0]);
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
    app.browser.expand_artist("Moby".to_string());
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

    assert_eq!(app.browser.selected_track_row(), 2);
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
    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
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
    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
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

    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
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
    assert_eq!(app.browser.selected_track_row(), 2);
}

#[test]
fn preserving_browser_selection_uses_media_item_id_after_reorder() {
    let first = test_track(1, "first track");
    let second = test_track(2, "second track");
    let mut app = test_app(vec![first.clone(), second.clone()]);
    app.focus = FocusPane::Tracks;
    app.browser.select_track_row(2);
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
    assert_eq!(app.browser.selected_track_row(), 1);
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

    assert!(!app.layout.info_panel_visible());
    assert_eq!(app.browser.selected_tree(), 0);

    app.handle_key(&conn, KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.browser.selected_tree(), 1);
    assert_eq!(app.browser.selected_track_row(), 1);
}
