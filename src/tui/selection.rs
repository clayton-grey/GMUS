use super::{App, TrackRow, TreeEntry};

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
        let selected_media_item_id = self.selected_playable_media_item_id();
        let selected_playlist_track = self
            .track_rows()
            .get(self.browser.selected_track_row())
            .and_then(|row| match row {
                TrackRow::PlaylistTrack {
                    playlist_id,
                    playlist_track_id,
                    ..
                } => Some((*playlist_id, *playlist_track_id)),
                _ => None,
            });

        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        if let Some(position) =
            selected_playlist_track.and_then(|(playlist_id, playlist_track_id)| {
                self.track_rows().iter().position(|row| {
                    matches!(
                        row,
                        TrackRow::PlaylistTrack {
                            playlist_id: row_playlist_id,
                            playlist_track_id: row_playlist_track_id,
                            ..
                        } if *row_playlist_id == playlist_id
                            && *row_playlist_track_id == playlist_track_id
                    )
                })
            })
        {
            self.browser.select_track_row(position);
            self.apply_selection_state();
        }
    }

    pub(super) fn sync_selection_preserving_browser_anchors(
        &mut self,
        selected_tree_entry: Option<&TreeEntry>,
        selected_media_item_id: Option<i64>,
    ) {
        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();
        if let Some(position) = selected_tree_entry.and_then(|entry| {
            self.tree_entries()
                .iter()
                .position(|candidate| candidate == entry)
        }) {
            self.browser.select_tree(position);
        } else {
            self.clamp_tree_selection();
        }

        self.rebuild_track_rows();
        if let Some(position) = selected_media_item_id.and_then(|media_item_id| {
            self.track_rows()
                .iter()
                .position(|row| self.track_row_media_item_id(row) == Some(media_item_id))
        }) {
            self.browser.select_track_row(position);
        } else {
            self.clamp_track_selection();
        }

        self.rebuild_playlist_entries();
        self.clamp_playlist_selection();
        self.apply_selection_state();
    }

    pub(super) fn apply_selection_state(&mut self) {
        let tree_len = self.view.tree_entries.len();
        self.tree_state
            .select((tree_len > 0).then_some(self.browser.selected_tree()));
        let row_len = self.view.track_rows.len();
        self.track_state
            .select((row_len > 0).then_some(self.browser.selected_track_row()));

        let playlist_len = self.view.playlist_entries.len();
        if playlist_len == 0 || !self.management_panel.playlist_open() {
            self.playlist_state.select(None);
        } else {
            self.playlist_state
                .select(Some(self.management_panel.playlist.selected_row()));
        }

        let keymap_len = super::keymap::keymap_row_count();
        self.management_panel.keymap.clamp_selection(keymap_len);
        if keymap_len == 0 || !self.management_panel.keymap_open() {
            self.keymap_state.select(None);
        } else {
            self.keymap_state
                .select(Some(self.management_panel.keymap.selected_row()));
        }
    }

    pub(super) fn clamp_tree_selection(&mut self) {
        let tree_len = self.view.tree_entries.len();
        self.browser.clamp_tree_selection(tree_len);
    }

    pub(super) fn clamp_track_selection(&mut self) {
        let row_len = self.view.track_rows.len();
        self.browser.clamp_track_selection(row_len);
        if row_len > 0 {
            let selected = self.browser.selected_track_row();
            if let Some(nearest) = self.nearest_track_row(selected) {
                self.browser.select_track_row(nearest);
            }
        }
    }
}
