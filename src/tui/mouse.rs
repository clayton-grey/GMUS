use super::layout::{
    info_panel_height_with_offset, library_pane_percent, percent_floor,
    uses_stacked_browser_layout, NARROW_TREE_PERCENT, WIDE_TREE_PERCENT,
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
    pub(super) keymap_info_visible: bool,
    pub(super) library_pane_percent_offset: i16,
    pub(super) info_pane_height_offset: i16,
    pub(super) column_layout_width: u16,
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
            keymap_info_visible: false,
            library_pane_percent_offset: 0,
            info_pane_height_offset: 0,
            column_layout_width: super::layout::DEFAULT_COLUMN_LAYOUT_WIDTH,
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

    pub(super) fn with_keymap_info(mut self, keymap_info_visible: bool) -> Self {
        self.keymap_info_visible = keymap_info_visible;
        self
    }

    pub(super) fn with_pane_offsets(
        mut self,
        library_pane_percent_offset: i16,
        info_pane_height_offset: i16,
    ) -> Self {
        self.library_pane_percent_offset = library_pane_percent_offset;
        self.info_pane_height_offset = info_pane_height_offset;
        self
    }

    pub(super) fn with_column_layout_width(mut self, width: u16) -> Self {
        self.column_layout_width = width;
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
        info_panel_height_with_offset(
            main_height,
            layout.input_visible,
            layout.info_pane_height_offset,
        )
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
        if layout.keymap_info_visible && info_height > 0 && row < browser_height + info_height {
            return Some(FocusPane::Keymap);
        }
        return None;
    }

    if uses_stacked_browser_layout(layout.terminal_width, layout.column_layout_width) {
        let tree_percent =
            library_pane_percent(NARROW_TREE_PERCENT, layout.library_pane_percent_offset);
        let tree_height = percent_floor(browser_height, tree_percent).max(1);
        if row < tree_height {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    } else {
        let tree_percent =
            library_pane_percent(WIDE_TREE_PERCENT, layout.library_pane_percent_offset);
        let tree_width = percent_floor(layout.terminal_width, tree_percent).max(1);
        if column < tree_width {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    }
}
