pub(super) const LIST_SCROLL_PADDING: usize = 3;
pub(super) const STACKED_PANE_WIDTH: u16 = 75;
pub(super) const WIDE_TREE_PERCENT: u16 = 33;
pub(super) const NARROW_TREE_PERCENT: u16 = 34;
pub(super) const BOTTOM_STATUS_ROWS: u16 = 2;
pub(super) const COMMAND_OUTPUT_MAX_ROWS: u16 = 8;

const INFO_PANEL_HEIGHT: u16 = 12;
const TRACKS_MIN_HEIGHT: u16 = 4;

pub(super) fn percent_floor(value: u16, percent: u16) -> u16 {
    ((u32::from(value) * u32::from(percent)) / 100) as u16
}

pub(super) fn info_panel_height(available_height: u16, input_visible: bool) -> u16 {
    if available_height == 0 {
        return 0;
    }
    let reserved = TRACKS_MIN_HEIGHT + u16::from(input_visible);
    let height = available_height.saturating_sub(reserved);
    if height == 0 {
        1
    } else {
        height.min(INFO_PANEL_HEIGHT)
    }
}
