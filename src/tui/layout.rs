pub(super) const LIST_SCROLL_PADDING: usize = 3;
pub(super) const DEFAULT_COLUMN_LAYOUT_WIDTH: u16 = 75;
pub(super) const WIDE_TREE_PERCENT: u16 = 33;
pub(super) const NARROW_TREE_PERCENT: u16 = 34;
pub(super) const BOTTOM_STATUS_ROWS: u16 = 2;
pub(super) const COMMAND_OUTPUT_MAX_ROWS: u16 = 8;
pub(super) const LIBRARY_PANE_STEP_PERCENT: i16 = 2;
pub(super) const INFO_PANE_STEP_ROWS: i16 = 1;

use super::{App, InputKind};

const INFO_PANEL_HEIGHT: u16 = 12;
const INFO_PANEL_MIN_HEIGHT: i16 = 3;
const INFO_PANEL_MAX_HEIGHT: i16 = 24;
const TRACKS_MIN_HEIGHT: u16 = 4;
const LIBRARY_PANE_MIN_PERCENT: i16 = 15;
const LIBRARY_PANE_MAX_PERCENT: i16 = 70;

pub(super) struct LayoutState {
    info_panel_visible: bool,
    startup_info_visible: bool,
    library_pane_percent_offset: i16,
    info_pane_height_offset: i16,
    column_layout_width: u16,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self::new(0, 0, DEFAULT_COLUMN_LAYOUT_WIDTH, false)
    }
}

impl LayoutState {
    pub(super) fn new(
        library_pane_percent_offset: i16,
        info_pane_height_offset: i16,
        column_layout_width: u16,
        startup_info_visible: bool,
    ) -> Self {
        Self {
            info_panel_visible: true,
            startup_info_visible,
            library_pane_percent_offset: clamp_library_pane_offset(library_pane_percent_offset),
            info_pane_height_offset: clamp_info_panel_offset(info_pane_height_offset),
            column_layout_width,
        }
    }

    pub(super) fn info_panel_visible(&self) -> bool {
        self.info_panel_visible
    }

    pub(super) fn show_info_panel(&mut self) {
        self.info_panel_visible = true;
    }

    pub(super) fn toggle_info_panel(&mut self) -> bool {
        self.info_panel_visible = !self.info_panel_visible;
        self.info_panel_visible
    }

    pub(super) fn startup_info_visible(&self) -> bool {
        self.startup_info_visible
    }

    pub(super) fn dismiss_startup_info(&mut self) -> bool {
        if self.startup_info_visible {
            self.startup_info_visible = false;
            true
        } else {
            false
        }
    }

    pub(super) fn library_pane_percent_offset(&self) -> i16 {
        self.library_pane_percent_offset
    }

    pub(super) fn info_pane_height_offset(&self) -> i16 {
        self.info_pane_height_offset
    }

    pub(super) fn column_layout_width(&self) -> u16 {
        self.column_layout_width
    }

    pub(super) fn set_column_layout_width(&mut self, width: u16) {
        self.column_layout_width = width;
    }

    pub(super) fn reset_pane_offsets(&mut self) {
        self.library_pane_percent_offset = 0;
        self.info_pane_height_offset = 0;
    }

    pub(super) fn resize_library_pane(&mut self, delta: i16) -> (i16, i16) {
        let previous = self.library_pane_percent_offset;
        let next = clamp_library_pane_offset(previous.saturating_add(delta));
        self.library_pane_percent_offset = next;
        (previous, next)
    }

    pub(super) fn resize_info_pane(&mut self, delta: i16) -> (i16, i16) {
        let previous = self.info_pane_height_offset;
        let next = clamp_info_panel_offset(previous.saturating_add(delta));
        self.info_pane_height_offset = next;
        (previous, next)
    }
}

impl App {
    pub(super) fn input_bar_visible(&self) -> bool {
        matches!(self.input.kind(), InputKind::Command | InputKind::Rate)
            || self.filter_bar_visible()
    }

    pub(super) fn command_output_visible(&self) -> bool {
        self.command_output.is_visible()
    }

    pub(super) fn info_area_visible(&self) -> bool {
        self.layout.info_panel_visible()
            || self.input.kind() == InputKind::Command
            || self.command_output_visible()
            || matches!(self.input.kind(), InputKind::Filter | InputKind::Rate)
            || self.management_panel.playlist_open()
            || self.management_panel.keymap_open()
    }

    pub(super) fn command_output_height(&self) -> u16 {
        self.command_output.height(COMMAND_OUTPUT_MAX_ROWS)
    }

    pub(super) fn reserved_bottom_rows(&self) -> u16 {
        BOTTOM_STATUS_ROWS
    }
}

pub(super) fn percent_floor(value: u16, percent: u16) -> u16 {
    ((u32::from(value) * u32::from(percent)) / 100) as u16
}

pub(super) fn uses_stacked_browser_layout(width: u16, column_layout_width: u16) -> bool {
    width <= column_layout_width
}

pub(super) fn library_pane_percent(base_percent: u16, offset: i16) -> u16 {
    (base_percent as i16 + clamp_library_pane_offset(offset)) as u16
}

pub(super) fn clamp_library_pane_offset(offset: i16) -> i16 {
    let min_base = WIDE_TREE_PERCENT.min(NARROW_TREE_PERCENT) as i16;
    let max_base = WIDE_TREE_PERCENT.max(NARROW_TREE_PERCENT) as i16;
    offset.clamp(
        LIBRARY_PANE_MIN_PERCENT - min_base,
        LIBRARY_PANE_MAX_PERCENT - max_base,
    )
}

pub(super) fn info_panel_target_height(offset: i16) -> u16 {
    (INFO_PANEL_HEIGHT as i16 + clamp_info_panel_offset(offset)) as u16
}

pub(super) fn clamp_info_panel_offset(offset: i16) -> i16 {
    offset.clamp(
        INFO_PANEL_MIN_HEIGHT - INFO_PANEL_HEIGHT as i16,
        INFO_PANEL_MAX_HEIGHT - INFO_PANEL_HEIGHT as i16,
    )
}

pub(super) fn info_panel_height_with_offset(
    available_height: u16,
    input_visible: bool,
    offset: i16,
) -> u16 {
    if available_height == 0 {
        return 0;
    }
    let reserved = TRACKS_MIN_HEIGHT + u16::from(input_visible);
    let height = available_height.saturating_sub(reserved);
    if height == 0 {
        1
    } else {
        height.min(info_panel_target_height(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_state_clamps_loaded_and_resized_offsets() {
        let mut layout = LayoutState::new(i16::MAX, i16::MIN, 90, true);

        assert_eq!(
            layout.library_pane_percent_offset(),
            clamp_library_pane_offset(i16::MAX)
        );
        assert_eq!(
            layout.info_pane_height_offset(),
            clamp_info_panel_offset(i16::MIN)
        );
        assert_eq!(layout.resize_library_pane(1), (36, 36));
        assert_eq!(layout.resize_info_pane(-1), (-9, -9));
        assert_eq!(layout.column_layout_width(), 90);
        assert!(layout.startup_info_visible());
    }

    #[test]
    fn layout_visibility_transitions_are_owned_together() {
        let mut layout = LayoutState::new(0, 0, DEFAULT_COLUMN_LAYOUT_WIDTH, true);

        assert!(!layout.toggle_info_panel());
        assert!(layout.dismiss_startup_info());
        assert!(!layout.dismiss_startup_info());

        layout.show_info_panel();

        assert!(layout.info_panel_visible());
        assert!(!layout.startup_info_visible());
    }
}
