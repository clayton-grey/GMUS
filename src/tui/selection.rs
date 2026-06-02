use super::App;

impl App {
    pub(super) fn sync_selection(&mut self) {
        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();
        self.clamp_tree_selection();
        self.rebuild_track_rows();
        self.clamp_track_selection();
        self.rebuild_playlist_entries();
        self.clamp_playlist_selection();
        self.apply_selection_state();
    }

    pub(super) fn sync_selection_preserving_browser_selection(&mut self) {
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_track_index = self.selected_playable_track_index();

        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();
        if let Some(position) = selected_tree_entry.as_ref().and_then(|entry| {
            self.tree_entries()
                .iter()
                .position(|candidate| candidate == entry)
        }) {
            self.selected_tree = position;
        } else {
            self.clamp_tree_selection();
        }

        self.rebuild_track_rows();
        if let Some(position) = selected_track_index.and_then(|index| {
            self.track_rows()
                .iter()
                .position(|row| row.track_index() == Some(index))
        }) {
            self.selected_track_row = position;
        } else {
            self.clamp_track_selection();
        }

        self.rebuild_playlist_entries();
        self.clamp_playlist_selection();
        self.apply_selection_state();
    }

    pub(super) fn apply_selection_state(&mut self) {
        let tree_len = self.view.tree_entries.len();
        if tree_len == 0 {
            self.tree_state.select(None);
        } else {
            self.tree_state.select(Some(self.selected_tree));
        }

        let row_len = self.view.track_rows.len();
        if row_len == 0 {
            self.track_state.select(None);
        } else {
            self.track_state.select(Some(self.selected_track_row));
        }

        let playlist_len = self.view.playlist_entries.len();
        if playlist_len == 0 || !self.playlist_panel_open {
            self.playlist_state.select(None);
        } else {
            self.playlist_state.select(Some(self.selected_playlist_row));
        }
    }

    pub(super) fn clamp_tree_selection(&mut self) {
        let tree_len = self.view.tree_entries.len();
        self.selected_tree = if tree_len == 0 {
            0
        } else {
            self.selected_tree.min(tree_len - 1)
        };
    }

    pub(super) fn clamp_track_selection(&mut self) {
        let row_len = self.view.track_rows.len();
        self.selected_track_row = if row_len == 0 {
            0
        } else {
            self.selected_track_row.min(row_len - 1)
        };
        if row_len > 0 {
            self.selected_track_row = self
                .nearest_track_row(self.selected_track_row)
                .unwrap_or(self.selected_track_row);
        }
    }
}
