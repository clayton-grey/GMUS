use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, Paragraph};
use ratatui::Frame;

use super::layout::{
    info_panel_height, BOTTOM_STATUS_ROWS, LIST_SCROLL_PADDING, NARROW_TREE_PERCENT,
    STACKED_PANE_WIDTH, WIDE_TREE_PERCENT,
};
use super::lines::{
    command_border_style, command_info_lines, command_info_title, command_pane_style,
    filter_info_lines, input_bar_style, input_line, metadata_lines, now_playing_line,
    now_playing_row_style, pane_active, pane_border_style, pane_highlight_style, playback_line,
    playlist_items, selected_scope_title, track_items, tree_items,
};
use super::{App, FocusPane};

pub(super) fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let info_visible = app.info_area_visible();
    let input_visible = app.input_bar_visible();
    let info_height = if info_visible {
        info_panel_height(
            area.height.saturating_sub(BOTTOM_STATUS_ROWS),
            input_visible,
        )
    } else {
        0
    };
    let mut constraints = vec![Constraint::Min(6)];
    if info_visible {
        constraints.push(Constraint::Length(info_height));
    }
    if input_visible {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    render_browser_panes(frame, app, vertical[0]);

    let mut row = 1;
    if info_visible {
        render_info_pane(frame, app, vertical[row]);
        row += 1;
    }
    if input_visible {
        render_input_bar(frame, app, vertical[row]);
        row += 1;
    }

    let now = Paragraph::new(now_playing_line(app, usize::from(vertical[row].width)))
        .style(now_playing_row_style())
        .alignment(Alignment::Left);
    frame.render_widget(now, vertical[row]);
    row += 1;

    let status = Paragraph::new(playback_line(app, usize::from(vertical[row].width)))
        .alignment(Alignment::Left);
    frame.render_widget(status, vertical[row]);
}

fn render_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < STACKED_PANE_WIDTH {
        render_stacked_browser_panes(frame, app, area);
    } else {
        render_wide_browser_panes(frame, app, area);
    }
}

fn render_wide_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(WIDE_TREE_PERCENT),
            Constraint::Percentage(100 - WIDE_TREE_PERCENT),
        ])
        .split(area);

    render_tree_pane(frame, app, columns[0]);
    render_tracks_pane(frame, app, columns[1]);
}

fn render_stacked_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let stack = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(NARROW_TREE_PERCENT),
            Constraint::Percentage(100 - NARROW_TREE_PERCENT),
        ])
        .split(area);

    render_tree_pane(frame, app, stack[0]);
    render_tracks_pane(frame, app, stack[1]);
}

fn render_tree_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let tree_active = pane_active(app, FocusPane::Tree);
    let tree = List::new(tree_items(app))
        .block(
            Block::default()
                .title("Library")
                .borders(Borders::ALL)
                .border_style(pane_border_style(tree_active)),
        )
        .scroll_padding(LIST_SCROLL_PADDING)
        .highlight_style(pane_highlight_style(tree_active));
    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

fn render_tracks_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let tracks_active = pane_active(app, FocusPane::Tracks);
    let tracks_title = selected_scope_title(app);
    let track_width = usize::from(area.width.saturating_sub(2));
    let tracks = List::new(track_items(app, track_width))
        .block(
            Block::default()
                .title(tracks_title)
                .borders(Borders::ALL)
                .border_style(pane_border_style(tracks_active)),
        )
        .scroll_padding(LIST_SCROLL_PADDING)
        .highlight_style(pane_highlight_style(tracks_active));
    frame.render_stateful_widget(tracks, area, &mut app.track_state);
}

fn render_info_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let command_info = app.command_mode || app.command_output_visible();
    let filter_info = !command_info && app.filter_mode;
    let playlist_info = !command_info && !filter_info && app.playlist_panel_open;
    let info_inner_width = usize::from(area.width.saturating_sub(2));
    let info_inner_height = area.height.saturating_sub(2);
    if playlist_info {
        render_playlist_info_pane(frame, app, area, info_inner_width);
        return;
    }

    let info_lines = if command_info {
        command_info_lines(app, info_inner_width, info_inner_height)
    } else if filter_info {
        filter_info_lines(app, info_inner_width, info_inner_height)
    } else {
        metadata_lines(app, info_inner_width)
    };
    let command_style = command_pane_style(app);
    let mut info_block = Block::default()
        .title(command_info_title(app))
        .borders(Borders::ALL)
        .border_style(if command_info {
            command_border_style(app)
        } else if filter_info {
            pane_border_style(true)
        } else {
            pane_border_style(false)
        });
    if command_info {
        info_block = info_block.style(command_style);
    }
    let mut info = Paragraph::new(info_lines)
        .block(info_block)
        .alignment(Alignment::Left);
    if command_info {
        info = info.style(command_style);
    }
    frame.render_widget(info, area);
}

pub(super) fn render_playlist_info_pane(
    frame: &mut Frame<'_>,
    app: &mut App,
    area: Rect,
    info_inner_width: usize,
) {
    let playlist_active = pane_active(app, FocusPane::Playlist);
    let playlist = List::new(playlist_items(app, info_inner_width))
        .block(
            Block::default()
                .title(command_info_title(app))
                .borders(Borders::ALL)
                .border_style(pane_border_style(playlist_active)),
        )
        .scroll_padding(LIST_SCROLL_PADDING)
        .highlight_style(pane_highlight_style(playlist_active));
    frame.render_stateful_widget(playlist, area, &mut app.playlist_state);
}

fn render_input_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let input = Paragraph::new(input_line(app, usize::from(area.width)))
        .style(input_bar_style(app))
        .alignment(Alignment::Left);
    frame.render_widget(input, area);
}
