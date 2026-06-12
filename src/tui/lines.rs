use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;

use crate::db::{self, LibraryTrack};
use crate::player::PlaybackState;

use super::command::{parse_playback_rate, COMMAND_NAMES};
use super::filter::FilterQuery;
use super::formatting::{
    album_divider, display_width, fit_to_width, push_limited_span, right_aligned_line, spans_width,
    truncate_to_width,
};
use super::layout::COMMAND_OUTPUT_MAX_ROWS;
use super::{
    App, CommandOutputKind, FocusPane, PlaybackSource, PlaylistPanelEntry, TrackRow, TreeEntry,
};

pub(super) fn command_info_title(app: &App) -> &'static str {
    if app.command_output.kind() == CommandOutputKind::LibraryRoots {
        "Library"
    } else if app.filter_mode && !app.command_output_visible() {
        "Filter"
    } else if app.rate_mode && !app.command_output_visible() {
        "Rate"
    } else if app.playlist_panel_open && !app.command_mode && !app.command_output_visible() {
        "Playlists"
    } else if app.keymap_panel_open && !app.command_mode && !app.command_output_visible() {
        "Keymap"
    } else if app.command_mode || app.command_output_visible() {
        "Command"
    } else {
        "Info"
    }
}

pub(super) fn pane_highlight_style(active: bool) -> Style {
    if active {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::White).fg(Color::Black)
    }
}

pub(super) fn pane_active(app: &App, pane: FocusPane) -> bool {
    !app.command_mode
        && !app.filter_mode
        && !app.rate_mode
        && !app.command_output.is_focused()
        && app.focus == pane
        && (pane != FocusPane::Playlist || app.playlist_panel_open)
        && (pane != FocusPane::Keymap || app.keymap_panel_open)
}

pub(super) fn pane_border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn input_bar_focused(app: &App) -> bool {
    app.command_mode || app.filter_mode || app.rate_mode
}

pub(super) fn input_bar_style(app: &App) -> Style {
    if input_bar_focused(app) {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

pub(super) fn command_pane_style(app: &App) -> Style {
    if app.command_mode {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

pub(super) fn command_border_style(app: &App) -> Style {
    if app.command_mode {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

fn placeholder_input_style(app: &App) -> Style {
    if input_bar_focused(app) {
        Style::default().fg(Color::Gray).bg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray).bg(Color::White)
    }
}

pub(super) fn now_playing_row_style() -> Style {
    Style::default().fg(Color::Black).bg(Color::White)
}

pub(super) fn tree_items(app: &App) -> Vec<ListItem<'static>> {
    let entries = app.tree_entries();
    if entries.is_empty() {
        return vec![ListItem::new(Line::from("no scanned tracks"))];
    }

    entries
        .iter()
        .map(|entry| ListItem::new(tree_item_line(app, entry)))
        .collect()
}

pub(super) fn tree_item_line(app: &App, entry: &TreeEntry) -> Line<'static> {
    match entry {
        TreeEntry::Playlists => {
            let marker = if app.playlists_expanded { "[-]" } else { "[+]" };
            let current_prefix = if app.tree_entry_is_current(entry) {
                "> "
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(current_prefix, Style::default().fg(Color::LightGreen)),
                Span::styled("Playlists", Style::default().add_modifier(Modifier::BOLD)),
            ])
        }
        TreeEntry::Playlist { name, .. } => Line::from(vec![
            Span::raw("    "),
            Span::styled(
                if app.tree_entry_is_current(entry) {
                    "> "
                } else {
                    ""
                },
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(name.clone(), Style::default().fg(Color::Cyan)),
        ]),
        TreeEntry::Compilation => {
            let marker = if app.compilations_expanded {
                "[-]"
            } else {
                "[+]"
            };
            let current_prefix = if app.tree_entry_is_current(entry) {
                "> "
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(current_prefix, Style::default().fg(Color::LightGreen)),
                Span::styled(
                    "Compilations",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])
        }
        TreeEntry::CompilationAlbum { album } => Line::from(vec![
            Span::raw("    "),
            Span::styled(
                if app.tree_entry_is_current(entry) {
                    "> "
                } else {
                    ""
                },
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(album.clone(), Style::default().fg(Color::Cyan)),
        ]),
        TreeEntry::Artist { artist } => {
            let expanded = app.expanded_artists.contains(artist);
            let marker = if expanded { "[-]" } else { "[+]" };
            let current_prefix = if app.tree_entry_is_current(entry) {
                "> "
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(current_prefix, Style::default().fg(Color::LightGreen)),
                Span::styled(
                    artist.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])
        }
        TreeEntry::Album { album, .. } => Line::from(vec![
            Span::raw("    "),
            Span::styled(
                if app.tree_entry_is_current(entry) {
                    "> "
                } else {
                    ""
                },
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(album.clone(), Style::default().fg(Color::Cyan)),
        ]),
    }
}

pub(super) fn track_items(app: &App, width: usize) -> Vec<ListItem<'static>> {
    let rows = app.track_rows();
    if rows.is_empty() {
        return vec![ListItem::new(Line::from("no tracks in this view"))];
    }

    rows.iter()
        .map(|row| match row {
            TrackRow::AlbumHeader {
                album,
                album_year,
                duration_ms,
            } => ListItem::new(album_header_line(album, *album_year, *duration_ms, width)),
            TrackRow::DiscDivider { disc_number } => {
                ListItem::new(disc_divider_line(*disc_number, width))
            }
            TrackRow::Track {
                track_index,
                show_disc_number,
            } => ListItem::new(track_line(app, *track_index, *show_disc_number, width)),
            TrackRow::PlaylistHeader { name, duration_ms } => {
                ListItem::new(playlist_header_line(name, *duration_ms, width))
            }
            TrackRow::PlaylistTrack {
                playlist_id,
                playlist_track_id,
                position,
                track_index,
            } => ListItem::new(playlist_track_line(
                app,
                *track_index,
                *playlist_id,
                *playlist_track_id,
                *position,
                width,
            )),
        })
        .collect()
}

pub(super) fn album_header_line(
    album: &str,
    album_year: Option<i64>,
    duration_ms: i64,
    width: usize,
) -> Line<'static> {
    let duration = db::format_duration(Some(duration_ms));
    let right = match album_year {
        Some(year) => format!("{year} {duration}"),
        None => duration,
    };
    let title_width = width.saturating_sub(display_width(&right) + 1);
    let title = truncate_to_width(album, title_width);
    let divider_width = width.saturating_sub(display_width(&title) + display_width(&right));
    Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            album_divider(divider_width),
            Style::default().fg(Color::LightMagenta),
        ),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ])
}

pub(super) fn playlist_header_line(name: &str, duration_ms: i64, width: usize) -> Line<'static> {
    album_header_line(name, None, duration_ms, width)
}

pub(super) fn disc_divider_line(disc_number: Option<i64>, width: usize) -> Line<'static> {
    let label = disc_number
        .map(|disc| format!(" disc {disc} "))
        .unwrap_or_else(|| " disc ".to_string());
    let divider_width = width.saturating_sub(display_width(&label));
    let left = divider_width / 2;
    let right = divider_width.saturating_sub(left);
    Line::from(Span::styled(
        format!("{}{}{}", "-".repeat(left), label, "-".repeat(right)),
        Style::default().fg(Color::DarkGray),
    ))
}

pub(super) fn track_line(
    app: &App,
    track_index: usize,
    show_disc_number: bool,
    width: usize,
) -> Line<'static> {
    let track = &app.tracks[track_index];
    let is_current = app
        .current
        .as_ref()
        .map(|current| {
            current.source.is_none() && current.track.media_item_id == track.media_item_id
        })
        .unwrap_or(false);
    let marker = if is_current { ">" } else { " " };
    let number = match (show_disc_number, track.disc_number, track.track_number) {
        (true, Some(disc), Some(track)) => format!("{disc}.{track:02}."),
        (_, _, Some(track)) => format!("{track:02}."),
        _ => "   ".to_string(),
    };
    let title_style = if is_current {
        Style::default().fg(Color::LightYellow)
    } else {
        Style::default()
    };
    let duration = db::format_duration(track.duration_ms);
    let play_count = format!("  x{}", track.play_count);
    let fixed_left = format!("{marker} {number} {play_count}");
    let title_width =
        width.saturating_sub(display_width(&fixed_left) + display_width(&duration) + 1);

    right_aligned_line(
        vec![
            Span::styled(marker, Style::default().fg(Color::LightGreen)),
            Span::raw(" "),
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                truncate_to_width(track.display_title(), title_width),
                title_style,
            ),
            Span::styled(play_count, Style::default().fg(Color::DarkGray)),
        ],
        vec![Span::styled(duration, Style::default().fg(Color::DarkGray))],
        width,
    )
}

pub(super) fn playlist_track_line(
    app: &App,
    track_index: usize,
    playlist_id: i64,
    playlist_track_id: i64,
    position: usize,
    width: usize,
) -> Line<'static> {
    let track = &app.tracks[track_index];
    let is_current = playlist_track_is_current(app, playlist_id, playlist_track_id);
    let marker = if is_current { ">" } else { " " };
    let number = format!("{position:02}.");
    let title_style = if is_current {
        Style::default().fg(Color::LightYellow)
    } else {
        Style::default()
    };
    let duration = db::format_duration(track.duration_ms);
    let separator = " - ";
    let fixed_left_width = display_width(marker)
        + 1
        + display_width(&number)
        + 1
        + display_width(track.display_artist())
        + display_width(separator);
    let text_width = width.saturating_sub(fixed_left_width + display_width(&duration) + 1);

    right_aligned_line(
        vec![
            Span::styled(marker, Style::default().fg(Color::LightGreen)),
            Span::raw(" "),
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(track.display_artist().to_string(), title_style),
            Span::raw(separator),
            Span::styled(
                truncate_to_width(track.display_title(), text_width),
                title_style,
            ),
        ],
        vec![Span::styled(duration, Style::default().fg(Color::DarkGray))],
        width,
    )
}

fn playlist_track_is_current(app: &App, playlist_id: i64, playlist_track_id: i64) -> bool {
    let Some(current) = &app.current else {
        return false;
    };
    match current.source {
        Some(PlaybackSource::PlaylistTrack {
            playlist_id: current_playlist_id,
            playlist_track_id: current_playlist_track_id,
        }) => current_playlist_id == playlist_id && current_playlist_track_id == playlist_track_id,
        None => false,
    }
}

pub(super) fn selected_scope_title(app: &App) -> String {
    match app.selected_tree_entry() {
        Some(TreeEntry::Playlists) => "Playlists".to_string(),
        Some(TreeEntry::Playlist { name, .. }) => format!("Playlist - {name}"),
        Some(TreeEntry::Compilation) => "Compilations".to_string(),
        Some(TreeEntry::CompilationAlbum { album, .. }) => {
            format!("Compilations - {album}")
        }
        Some(TreeEntry::Artist { artist }) => artist.clone(),
        Some(TreeEntry::Album { artist, album, .. }) => format!("{artist} - {album}"),
        None => "Tracks".to_string(),
    }
}

pub(super) fn now_playing_line(app: &App, width: usize) -> Line<'static> {
    let Some(current) = &app.current else {
        return right_aligned_line(
            vec![Span::raw(" idle ")],
            vec![Span::styled(
                format!("{} tracks", app.tracks.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )],
            width,
        );
    };

    let left = format!(
        " {} - {}",
        current.track.display_artist(),
        current.track.display_title()
    );
    let right = match (current.track.display_album(), current.track.album_year) {
        ("", Some(year)) => format!("({year})"),
        ("", None) => String::new(),
        (album, Some(year)) => format!("{album} ({year})"),
        (album, None) => album.to_string(),
    };
    let left_width = width.saturating_sub(display_width(&right) + 1);

    right_aligned_line(
        vec![Span::styled(
            truncate_to_width(&left, left_width),
            Style::default().add_modifier(Modifier::BOLD),
        )],
        vec![Span::styled(right, Style::default().fg(Color::DarkGray))],
        width,
    )
}

pub(super) fn playback_line(app: &App, width: usize) -> Line<'static> {
    if app.active_transient_status().is_some() {
        return Line::from(playback_progress_spans(app, width, 0));
    }

    let mut right = vec![Span::styled(
        format!(
            "{} | {}% | ",
            app.play_target.label(),
            progress_percent(app)
        ),
        Style::default().fg(Color::DarkGray),
    )];
    right.extend(playback_flag_spans(app));

    right_aligned_line(
        playback_progress_spans(app, width, spans_width(&right)),
        right,
        width,
    )
}

pub(super) fn input_line(app: &App, width: usize) -> Line<'static> {
    if app.command_mode {
        command_line(app, width)
    } else if app.rate_mode {
        rate_line(app, width)
    } else {
        filter_line(app, width)
    }
}

pub(super) fn command_info_lines(app: &App, width: usize, height: u16) -> Vec<Line<'static>> {
    let style = command_pane_style(app);
    if app.command_output.kind() == CommandOutputKind::LibraryRoots {
        library_root_lines(app, width, height, style)
    } else if app.command_output_visible() {
        command_output_lines(app, width, height.min(app.command_output_height()), style)
    } else {
        command_help_lines(width, style)
    }
}

pub(super) fn playlist_items(app: &App, width: usize) -> Vec<ListItem<'static>> {
    if app.view.playlist_entries.is_empty() {
        return vec![
            ListItem::new(Line::from(Span::styled(
                truncate_to_width(" playlists", width),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from(Span::styled(
                truncate_to_width(" :playlist NAME creates one", width),
                Style::default().fg(Color::DarkGray),
            ))),
        ];
    }

    app.view
        .playlist_entries
        .iter()
        .map(|entry| ListItem::new(truncate_to_width(&playlist_entry_text(app, entry), width)))
        .collect()
}

pub(super) fn playlist_entry_text(app: &App, entry: &PlaylistPanelEntry) -> String {
    match entry {
        PlaylistPanelEntry::Playlist { playlist_id, name } => {
            let marker = if app.expanded_playlists.contains(playlist_id) {
                "[-]"
            } else {
                "[+]"
            };
            let active = if app.active_playlist_id == Some(*playlist_id) {
                "> "
            } else {
                "  "
            };
            let count = app.playlist_cache.len(*playlist_id);
            format!(" {marker} {active}{name} ({count})")
        }
        PlaylistPanelEntry::Track {
            position,
            track_index,
            ..
        } => {
            let Some(track) = app.tracks.get(*track_index) else {
                return format!("      {position:02}. <missing track>");
            };
            format!(
                "      {position:02}. {} - {}",
                track.display_artist(),
                track.display_title()
            )
        }
    }
}

fn library_root_lines(app: &App, width: usize, height: u16, style: Style) -> Vec<Line<'static>> {
    let height = usize::from(height.min(COMMAND_OUTPUT_MAX_ROWS));
    if height == 0 {
        return Vec::new();
    }

    let roots = app.command_output.roots();
    if roots.is_empty() {
        return command_output_lines(app, width, height as u16, style);
    }

    let active_count = roots.iter().filter(|root| root.active).count();
    let mut lines = vec![Line::from(Span::styled(
        truncate_to_width(
            &format!(
                " library roots ({active_count} active / {} total)",
                roots.len()
            ),
            width,
        ),
        style.add_modifier(Modifier::BOLD),
    ))];

    let root_slots = height.saturating_sub(1);
    if root_slots == 0 {
        return lines;
    }

    let selected = app.command_output.selected_index().min(roots.len() - 1);
    let offset = selected.saturating_add(1).saturating_sub(root_slots);
    for (index, root) in roots.iter().enumerate().skip(offset).take(root_slots) {
        let content = format!(" {} {}", if root.active { "[x]" } else { "[ ]" }, root.path);
        let selected_row = app.command_output.is_focused() && index == selected;
        let row_style = if selected_row {
            pane_highlight_style(true)
        } else {
            style
        };
        let content = if selected_row {
            fit_to_width(&content, width)
        } else {
            truncate_to_width(&content, width)
        };
        lines.push(Line::from(Span::styled(content, row_style)));
    }
    lines
}

fn command_output_lines(app: &App, width: usize, height: u16, style: Style) -> Vec<Line<'static>> {
    let height = usize::from(height.min(COMMAND_OUTPUT_MAX_ROWS));
    if height == 0 {
        return Vec::new();
    }
    let output = app.command_output.lines();
    let hidden = if output.len() > height {
        output.len() - (height - 1)
    } else {
        0
    };
    let mut lines = Vec::new();
    for (index, text) in output.iter().take(height).enumerate() {
        let content = if hidden > 0 && index + 1 == height {
            format!(" ... {hidden} more")
        } else {
            format!(" {text}")
        };
        let style = if index == 0 {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        };
        lines.push(Line::from(Span::styled(
            truncate_to_width(&content, width),
            style,
        )));
    }
    lines
}

pub(super) fn command_help_lines(width: usize, style: Style) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        " command mode",
        style.add_modifier(Modifier::BOLD),
    ))];
    lines.extend(command_list_lines(width, style));
    lines.push(Line::from(Span::styled(
        " Tab completes commands and paths",
        style,
    )));
    lines.push(Line::from(Span::styled(" Enter runs  Esc cancels", style)));
    lines
}

pub(super) fn filter_info_lines(app: &App, width: usize, height: u16) -> Vec<Line<'static>> {
    let height = usize::from(height);
    if height == 0 {
        return Vec::new();
    }

    let query = FilterQuery::parse(&app.filter);
    let mut lines = vec![Line::from(Span::styled(
        " filter syntax",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];

    if let Some(warning) = query.warning() {
        lines.push(Line::from(Span::styled(
            truncate_to_width(&format!(" {warning}"), width),
            Style::default().fg(Color::LightRed),
        )));
    }

    for text in [
        "bare text searches title, artist, album, genre, composer, root, date, and path",
        "field:value narrows a field; prefix - to exclude a term",
        "fields: title artist album albumartist year date genre composer root path compilation plays trackno disc",
        "examples: genre:ambient year:2010..2020",
        "          root:Instrumental -compilation:true plays:>5",
    ] {
        lines.push(Line::from(Span::styled(
            truncate_to_width(&format!(" {text}"), width),
            Style::default().fg(Color::Gray),
        )));
    }

    lines.truncate(height);
    lines
}

pub(super) fn rate_info_lines(app: &App, width: usize, height: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        " playback rate",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];
    if !app.rate_input.is_empty() && parse_playback_rate(&app.rate_input).is_none() {
        lines.push(Line::from(Span::styled(
            truncate_to_width(" invalid rate; use 0.25..4.0 or 25..400", width),
            Style::default().fg(Color::LightRed),
        )));
    }
    for (text, style) in [
        (
            " enter a multiplier from 0.25 to 4.0",
            Style::default().fg(Color::Gray),
        ),
        (
            " values above 4 are percentages: 75 means 75%",
            Style::default().fg(Color::Gray),
        ),
        (
            " Enter or Tab applies  Esc cancels",
            Style::default().fg(Color::Gray),
        ),
    ] {
        lines.push(Line::from(Span::styled(
            truncate_to_width(text, width),
            style,
        )));
    }
    lines.truncate(usize::from(height));
    lines
}

fn command_list_lines(width: usize, style: Style) -> Vec<Line<'static>> {
    let prefix = " commands: ";
    let indent = " ".repeat(display_width(prefix));
    let mut lines = Vec::new();
    let mut current = prefix.to_string();

    for command in COMMAND_NAMES {
        let separator_width = usize::from(!current.ends_with(' '));
        let next_width = display_width(&current) + separator_width + display_width(command);
        if next_width <= width || current == prefix {
            if !current.ends_with(' ') {
                current.push(' ');
            }
            current.push_str(command);
        } else {
            lines.push(Line::from(Span::styled(
                truncate_to_width(&current, width),
                style,
            )));
            current = format!("{indent}{command}");
        }
    }

    lines.push(Line::from(Span::styled(
        truncate_to_width(&current, width),
        style,
    )));
    lines
}

pub(super) fn metadata_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    if app.startup_info_visible {
        return startup_info_lines(width);
    }

    let Some(track) = app
        .selected_playable_track_index()
        .and_then(|index| app.tracks.get(index))
        .or_else(|| app.current.as_ref().map(|current| &current.track))
    else {
        return vec![Line::from(Span::styled(
            " no selected track",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    vec![
        Line::from(Span::styled(
            " selected track",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        metadata_pair("title", track.display_title(), width),
        metadata_pair("artist", fallback_text(track.display_artist()), width),
        metadata_pair("album", fallback_text(track.display_album()), width),
        metadata_pair(
            "composer",
            fallback_optional(track.composer.as_deref()),
            width,
        ),
        metadata_pair("genre", fallback_optional(track.genre.as_deref()), width),
        metadata_pair("released", release_date_text(track), width),
        metadata_track_position_pair(track, width),
        metadata_pair("length", db::format_duration(track.duration_ms), width),
        metadata_pair("plays", track.play_count.to_string(), width),
    ]
}

fn startup_info_lines(width: usize) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            truncate_to_width(" GMUS", width),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_to_width(" a CMUS inspired terminal music player", width),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::ITALIC),
        )),
        Line::from(Span::styled(
            truncate_to_width(" authors: Clayton Grey with Codex", width),
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

fn release_date_text(track: &LibraryTrack) -> String {
    track
        .release_date
        .clone()
        .or_else(|| track.album_year.map(|year| year.to_string()))
        .unwrap_or_else(|| "--".to_string())
}

fn metadata_track_position_pair(track: &LibraryTrack, width: usize) -> Line<'static> {
    let label = format!(" {:<9}", "track");
    let mut remaining = width.saturating_sub(display_width(&label));
    let mut spans = vec![Span::styled(label, Style::default().fg(Color::DarkGray))];
    let value_style = Style::default().fg(Color::White);
    let label_style = Style::default().fg(Color::DarkGray);

    if let Some(track_text) = track_number_text(track) {
        push_limited_span(&mut spans, &mut remaining, &track_text, value_style);
        if let Some(disc_text) = disc_number_text(track) {
            push_limited_span(&mut spans, &mut remaining, "  disc ", label_style);
            push_limited_span(&mut spans, &mut remaining, &disc_text, value_style);
        }
    } else if let Some(disc_text) = disc_number_text(track) {
        push_limited_span(&mut spans, &mut remaining, "disc ", label_style);
        push_limited_span(&mut spans, &mut remaining, &disc_text, value_style);
    } else {
        push_limited_span(&mut spans, &mut remaining, "--", value_style);
    }

    Line::from(spans)
}

fn track_number_text(track: &LibraryTrack) -> Option<String> {
    let track_number = track.track_number?;
    Some(match track.track_total {
        Some(track_total) => format!("{track_number}/{track_total}"),
        None => track_number.to_string(),
    })
}

fn disc_number_text(track: &LibraryTrack) -> Option<String> {
    let disc_number = track.disc_number?;
    Some(match track.disc_total {
        Some(disc_total) => format!("{disc_number}/{disc_total}"),
        None => disc_number.to_string(),
    })
}

fn metadata_pair(label: &str, value: impl AsRef<str>, width: usize) -> Line<'static> {
    let label = format!(" {label:<9}");
    let value_width = width.saturating_sub(display_width(&label));
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(
            truncate_to_width(value.as_ref(), value_width),
            Style::default().fg(Color::White),
        ),
    ])
}

fn fallback_text(value: &str) -> &str {
    if value.is_empty() {
        "--"
    } else {
        value
    }
}

fn fallback_optional(value: Option<&str>) -> &str {
    value.filter(|value| !value.is_empty()).unwrap_or("--")
}

pub(super) fn command_line(app: &App, width: usize) -> Line<'static> {
    let text_width = width.saturating_sub(1);
    let style = input_bar_style(app);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(":", style),
        Span::styled(
            truncate_to_width(&format!("{}_", app.command), text_width.saturating_sub(1)),
            style,
        ),
    ])
}

pub(super) fn filter_line(app: &App, width: usize) -> Line<'static> {
    let text_width = width.saturating_sub(1);
    let style = input_bar_style(app);
    let filter = if app.filter.is_empty() {
        Span::styled(
            truncate_to_width(
                "none_",
                text_width.saturating_sub(display_width("filter: ")),
            ),
            placeholder_input_style(app),
        )
    } else if app.filter_mode {
        Span::styled(
            truncate_to_width(
                &format!("{}_", app.filter),
                text_width.saturating_sub(display_width("filter: ")),
            ),
            style,
        )
    } else {
        Span::styled(
            truncate_to_width(
                &app.filter,
                text_width.saturating_sub(display_width("filter: ")),
            ),
            style,
        )
    };

    Line::from(vec![
        Span::raw(" "),
        Span::styled("filter: ", style),
        filter,
    ])
}

pub(super) fn rate_line(app: &App, width: usize) -> Line<'static> {
    let text_width = width.saturating_sub(1);
    let style = input_bar_style(app);
    let rate = if app.rate_input.is_empty() {
        Span::styled(
            truncate_to_width(
                "0.75 or 75_",
                text_width.saturating_sub(display_width("rate: ")),
            ),
            placeholder_input_style(app),
        )
    } else {
        Span::styled(
            truncate_to_width(
                &format!("{}_", app.rate_input),
                text_width.saturating_sub(display_width("rate: ")),
            ),
            style,
        )
    };

    Line::from(vec![Span::raw(" "), Span::styled("rate: ", style), rate])
}

fn playback_flag_spans(app: &App) -> Vec<Span<'static>> {
    vec![
        Span::styled("C", active_flag_style(app.continuous)),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled("R", active_flag_style(app.repeat)),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled("S", active_flag_style(app.shuffle)),
    ]
}

fn active_flag_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn playback_progress_spans(app: &App, width: usize, right_width: usize) -> Vec<Span<'static>> {
    if let Some(status) = app.active_transient_status() {
        let available_width = width.saturating_sub(right_width + 1);
        return vec![
            Span::raw(" "),
            Span::styled(
                truncate_to_width(status, available_width.saturating_sub(1)),
                Style::default().fg(Color::White),
            ),
        ];
    }

    let position = db::format_duration(Some(app.current_position_ms()));
    let duration = app
        .current
        .as_ref()
        .map(|current| db::format_duration(current.track.duration_ms))
        .unwrap_or_else(|| "--:--".to_string());
    let time = format!("{position} / {duration}");
    let rate = playback_rate_label(app);
    let fixed_width =
        display_width(" > ") + display_width(&time) + display_width(&rate) + display_width(" []");
    let available_bar_width = width.saturating_sub(fixed_width + right_width + 2);
    let bar_width = if available_bar_width >= 24 {
        available_bar_width.min(56)
    } else {
        available_bar_width
    };
    let playing = app.logical_state() == PlaybackState::Playing;
    let state_marker = if playing { ">" } else { "|" };
    let marker_style = if playing {
        Style::default().fg(Color::LightGreen)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut spans = vec![
        Span::styled(format!(" {state_marker} "), marker_style),
        Span::styled(time, Style::default().fg(Color::White)),
    ];
    if !rate.is_empty() {
        spans.push(Span::styled(rate, Style::default().fg(Color::DarkGray)));
    }
    if bar_width > 0 {
        spans.extend([
            Span::raw(" "),
            Span::styled(
                format!("[{}]", progress_bar(app, bar_width)),
                Style::default().fg(Color::LightMagenta),
            ),
        ]);
    }
    spans
}

fn playback_rate_label(app: &App) -> String {
    let percent = (app.player.rate() * 100.0).round() as i32;
    if percent == 100 {
        String::new()
    } else {
        format!(" ({percent}%)")
    }
}

fn progress_percent(app: &App) -> i64 {
    let Some(current) = &app.current else {
        return 0;
    };
    let Some(duration_ms) = current.track.duration_ms.filter(|duration| *duration > 0) else {
        return 0;
    };
    ((app.current_position_ms().clamp(0, duration_ms) * 100) / duration_ms).clamp(0, 100)
}

fn progress_bar(app: &App, width: usize) -> String {
    let Some(current) = &app.current else {
        return "-".repeat(width);
    };
    let Some(duration_ms) = current.track.duration_ms.filter(|duration| *duration > 0) else {
        return "-".repeat(width);
    };
    let position_ms = app.current_position_ms().clamp(0, duration_ms);
    let filled = ((position_ms as usize) * width) / (duration_ms as usize);
    format!("{}{}", "=".repeat(filled), "-".repeat(width - filled))
}
