pub(super) const LIST_SCROLL_PADDING: usize = 3;
pub(super) const DEFAULT_COLUMN_LAYOUT_WIDTH: u16 = 75;
pub(super) const WIDE_TREE_PERCENT: u16 = 33;
pub(super) const NARROW_TREE_PERCENT: u16 = 34;
pub(super) const BOTTOM_STATUS_ROWS: u16 = 2;
pub(super) const COMMAND_OUTPUT_MAX_ROWS: u16 = 8;
pub(super) const LIBRARY_PANE_STEP_PERCENT: i16 = 2;
pub(super) const INFO_PANE_STEP_ROWS: i16 = 1;

const INFO_PANEL_HEIGHT: u16 = 12;
const INFO_PANEL_MIN_HEIGHT: i16 = 3;
const INFO_PANEL_MAX_HEIGHT: i16 = 24;
const TRACKS_MIN_HEIGHT: u16 = 4;
const LIBRARY_PANE_MIN_PERCENT: i16 = 15;
const LIBRARY_PANE_MAX_PERCENT: i16 = 70;

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
