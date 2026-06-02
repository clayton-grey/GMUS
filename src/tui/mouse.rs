use super::layout::{
    info_panel_height, percent_floor, NARROW_TREE_PERCENT, STACKED_PANE_WIDTH, WIDE_TREE_PERCENT,
};
use super::FocusPane;

#[derive(Debug, Clone, Copy)]
pub(super) struct MouseLayout {
    pub(super) terminal_width: u16,
    pub(super) terminal_height: u16,
    pub(super) reserved_bottom_rows: u16,
    pub(super) info_visible: bool,
    pub(super) input_visible: bool,
    pub(super) playlist_info_visible: bool,
}

#[cfg(test)]
impl MouseLayout {
    pub(super) fn new(
        terminal_width: u16,
        terminal_height: u16,
        reserved_bottom_rows: u16,
    ) -> Self {
        Self {
            terminal_width,
            terminal_height,
            reserved_bottom_rows,
            info_visible: false,
            input_visible: false,
            playlist_info_visible: false,
        }
    }

    pub(super) fn with_info(mut self, info_visible: bool, input_visible: bool) -> Self {
        self.info_visible = info_visible;
        self.input_visible = input_visible;
        self
    }

    pub(super) fn with_playlist_info(mut self, playlist_info_visible: bool) -> Self {
        self.playlist_info_visible = playlist_info_visible;
        self
    }
}

pub(super) fn mouse_pane(column: u16, row: u16, layout: MouseLayout) -> Option<FocusPane> {
    let main_height = layout
        .terminal_height
        .saturating_sub(layout.reserved_bottom_rows);
    if layout.terminal_width == 0 || main_height == 0 || row >= main_height {
        return None;
    }

    let info_height = if layout.info_visible {
        info_panel_height(main_height, layout.input_visible)
    } else {
        0
    };
    let browser_height = main_height
        .saturating_sub(info_height)
        .saturating_sub(u16::from(layout.input_visible));
    if browser_height == 0 || row >= browser_height {
        if layout.playlist_info_visible && info_height > 0 && row < browser_height + info_height {
            return Some(FocusPane::Playlist);
        }
        return None;
    }

    if layout.terminal_width < STACKED_PANE_WIDTH {
        let tree_height = percent_floor(browser_height, NARROW_TREE_PERCENT).max(1);
        if row < tree_height {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    } else {
        let tree_width = percent_floor(layout.terminal_width, WIDE_TREE_PERCENT).max(1);
        if column < tree_width {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    }
}
