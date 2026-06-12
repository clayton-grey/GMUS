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
        let selected_playlist_track =
            self.track_rows()
                .get(self.selected_track_row)
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
            self.selected_track_row = position;
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
            self.selected_tree = position;
        } else {
            self.clamp_tree_selection();
        }

        self.rebuild_track_rows();
        if let Some(position) = selected_media_item_id.and_then(|media_item_id| {
            self.track_rows()
                .iter()
                .position(|row| self.track_row_media_item_id(row) == Some(media_item_id))
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

        let keymap_len = super::keymap::keymap_row_count();
        self.selected_keymap_row = if keymap_len == 0 {
            0
        } else {
            self.selected_keymap_row.min(keymap_len - 1)
        };
        if keymap_len == 0 || !self.keymap_panel_open {
            self.keymap_state.select(None);
        } else {
            self.keymap_state.select(Some(self.selected_keymap_row));
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
