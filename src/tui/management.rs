use super::keymap::{KeyAction, KeymapPanelState};
use super::playlist::PlaylistPanelState;
use super::{App, FocusPane};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ManagementPanel {
    Playlist,
    Keymap,
}

#[derive(Debug, Default)]
pub(super) struct ManagementPanelState {
    pub(super) playlist: PlaylistPanelState,
    pub(super) keymap: KeymapPanelState,
    visible: Option<ManagementPanel>,
}

impl ManagementPanelState {
    pub(super) fn playlist_open(&self) -> bool {
        self.visible == Some(ManagementPanel::Playlist)
    }

    pub(super) fn keymap_open(&self) -> bool {
        self.visible == Some(ManagementPanel::Keymap)
    }

    pub(super) fn show_playlist(&mut self) {
        self.keymap.cancel_capture();
        self.visible = Some(ManagementPanel::Playlist);
    }

    pub(super) fn show_keymap(&mut self) {
        self.visible = Some(ManagementPanel::Keymap);
    }

    pub(super) fn hide_keymap(&mut self) {
        self.keymap.cancel_capture();
        if self.keymap_open() {
            self.visible = None;
        }
    }

    pub(super) fn hide(&mut self) {
        self.keymap.cancel_capture();
        self.visible = None;
    }

    pub(super) fn begin_keymap_capture(&mut self, action: KeyAction) {
        self.show_keymap();
        self.keymap.begin_capture(action);
    }
}

impl App {
    pub(super) fn show_track_info_panel(&mut self) {
        self.management_panel.hide();
        self.layout.show_info_panel();
        if matches!(self.focus, FocusPane::Playlist | FocusPane::Keymap) {
            self.focus = FocusPane::Tree;
        }
        self.apply_selection_state();
        self.message = String::from("track info panel");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_a_management_panel_replaces_the_previous_panel() {
        let mut panels = ManagementPanelState::default();

        panels.show_playlist();
        assert!(panels.playlist_open());
        assert!(!panels.keymap_open());

        panels.show_keymap();
        assert!(!panels.playlist_open());
        assert!(panels.keymap_open());
    }

    #[test]
    fn every_keymap_exit_cancels_capture() {
        let mut panels = ManagementPanelState::default();
        panels.begin_keymap_capture(KeyAction::ToggleInfo);

        panels.hide_keymap();

        assert!(!panels.keymap_open());
        assert!(!panels.keymap.is_capturing());

        panels.begin_keymap_capture(KeyAction::ToggleInfo);
        panels.hide();

        assert!(!panels.keymap_open());
        assert!(!panels.keymap.is_capturing());

        panels.begin_keymap_capture(KeyAction::ToggleInfo);
        panels.show_playlist();

        assert!(panels.playlist_open());
        assert!(!panels.keymap.is_capturing());
    }
}
