use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::db;

use super::playback::PlaybackEntry;
use super::{App, FocusPane};

#[derive(Debug, Default)]
pub(super) struct PlaylistPanelState {
    selected_row: usize,
    expanded_playlists: HashSet<i64>,
    active_playlist_id: Option<i64>,
}

impl PlaylistPanelState {
    pub(super) fn selected_row(&self) -> usize {
        self.selected_row
    }

    pub(super) fn select_row(&mut self, position: usize) {
        self.selected_row = position;
    }

    pub(super) fn clamp_selection(&mut self, row_len: usize) {
        self.selected_row = if row_len == 0 {
            0
        } else {
            self.selected_row.min(row_len - 1)
        };
    }

    pub(super) fn move_selection(&mut self, direction: i32, amount: usize, row_len: usize) {
        if row_len == 0 {
            self.selected_row = 0;
            return;
        }
        self.selected_row = if direction >= 0 {
            self.selected_row.saturating_add(amount).min(row_len - 1)
        } else {
            self.selected_row.saturating_sub(amount)
        };
    }

    pub(super) fn active_playlist_id(&self) -> Option<i64> {
        self.active_playlist_id
    }

    pub(super) fn set_active_playlist_id(&mut self, playlist_id: Option<i64>) {
        self.active_playlist_id = playlist_id;
    }

    pub(super) fn playlist_expanded(&self, playlist_id: i64) -> bool {
        self.expanded_playlists.contains(&playlist_id)
    }

    pub(super) fn expand_playlist(&mut self, playlist_id: i64) {
        self.expanded_playlists.insert(playlist_id);
    }

    pub(super) fn collapse_playlist(&mut self, playlist_id: i64) -> bool {
        self.expanded_playlists.remove(&playlist_id)
    }

    pub(super) fn activate_and_expand(&mut self, playlist_id: i64) {
        self.active_playlist_id = Some(playlist_id);
        self.expand_playlist(playlist_id);
    }
}

#[derive(Debug, Clone)]
pub(super) enum PlaylistPanelEntry {
    Playlist {
        playlist_id: i64,
        name: String,
    },
    Track {
        playlist_id: i64,
        playlist_track_id: i64,
        position: usize,
        track_index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistSelectionAnchor {
    Playlist {
        playlist_id: i64,
    },
    Track {
        playlist_id: i64,
        playlist_track_id: i64,
    },
}

impl PlaylistSelectionAnchor {
    fn playlist_id(self) -> i64 {
        match self {
            Self::Playlist { playlist_id } | Self::Track { playlist_id, .. } => playlist_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlaylistCacheEntry {
    pub(super) playlist_track_id: i64,
    pub(super) media_item_id: i64,
    pub(super) track_index: Option<usize>,
}

#[derive(Debug, Default)]
pub(super) struct PlaylistCache {
    entries: HashMap<i64, Vec<PlaylistCacheEntry>>,
}

impl PlaylistCache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn insert(&mut self, playlist_id: i64, entries: Vec<PlaylistCacheEntry>) {
        self.entries.insert(playlist_id, entries);
    }

    pub(super) fn len(&self, playlist_id: i64) -> usize {
        self.entries.get(&playlist_id).map_or(0, Vec::len)
    }

    pub(super) fn playable_entries(
        &self,
        playlist_id: i64,
    ) -> impl Iterator<Item = &PlaylistCacheEntry> {
        self.entries
            .get(&playlist_id)
            .into_iter()
            .flatten()
            .filter(|entry| entry.track_index.is_some())
    }

    pub(super) fn track_indices(&self, playlist_id: i64) -> impl Iterator<Item = usize> + '_ {
        self.playable_entries(playlist_id)
            .filter_map(|entry| entry.track_index)
    }

    pub(super) fn contains_track(&self, playlist_id: i64, track_index: usize) -> bool {
        self.track_indices(playlist_id)
            .any(|index| index == track_index)
    }

    pub(super) fn contains_track_in_any_playlist(&self, track_index: usize) -> bool {
        self.entries
            .keys()
            .copied()
            .any(|playlist_id| self.contains_track(playlist_id, track_index))
    }

    pub(super) fn track_index_for_entry(
        &self,
        playlist_id: i64,
        playlist_track_id: i64,
    ) -> Option<usize> {
        self.entries
            .get(&playlist_id)?
            .iter()
            .find(|entry| entry.playlist_track_id == playlist_track_id)?
            .track_index
    }

    pub(super) fn media_item_id_for_entry(
        &self,
        playlist_id: i64,
        playlist_track_id: i64,
    ) -> Option<i64> {
        self.entries
            .get(&playlist_id)?
            .iter()
            .find(|entry| entry.playlist_track_id == playlist_track_id)
            .map(|entry| entry.media_item_id)
    }
}

impl App {
    pub(super) fn refresh_playlist_tracks(&mut self, conn: &Connection) -> Result<()> {
        self.playlist_cache.clear();
        let media_index: HashMap<i64, usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.media_item_id, index))
            .collect();

        for playlist in &self.playlists {
            let entries = db::playlist_tracks(conn, playlist.id)?;
            let cached_entries = entries
                .iter()
                .map(|entry| PlaylistCacheEntry {
                    playlist_track_id: entry.id,
                    media_item_id: entry.media_item_id,
                    track_index: media_index.get(&entry.media_item_id).copied(),
                })
                .collect::<Vec<_>>();
            self.playlist_cache.insert(playlist.id, cached_entries);
        }

        if self
            .management_panel
            .playlist
            .active_playlist_id()
            .is_none_or(|id| !self.playlists.iter().any(|playlist| playlist.id == id))
        {
            self.management_panel
                .playlist
                .set_active_playlist_id(self.playlists.first().map(|playlist| playlist.id));
        }
        Ok(())
    }

    pub(super) fn clamp_playlist_selection(&mut self) {
        let row_len = self.view.playlist_entries.len();
        self.management_panel.playlist.clamp_selection(row_len);
        if let Some(playlist_id) = self.selected_playlist_entry_playlist_id() {
            self.management_panel
                .playlist
                .set_active_playlist_id(Some(playlist_id));
        }
    }

    pub(super) fn rebuild_playlist_entries(&mut self) {
        self.view.playlist_entries.clear();
        for playlist in &self.playlists {
            self.view
                .playlist_entries
                .push(PlaylistPanelEntry::Playlist {
                    playlist_id: playlist.id,
                    name: playlist.name.clone(),
                });
            if self
                .management_panel
                .playlist
                .playlist_expanded(playlist.id)
            {
                self.view.playlist_entries.extend(
                    self.playlist_cache
                        .playable_entries(playlist.id)
                        .enumerate()
                        .map(|(position, entry)| PlaylistPanelEntry::Track {
                            playlist_id: playlist.id,
                            playlist_track_id: entry.playlist_track_id,
                            position: position + 1,
                            track_index: entry
                                .track_index
                                .expect("playable playlist entries have a track index"),
                        }),
                );
            }
        }
    }

    pub(super) fn selected_playlist_entry_playlist_id(&self) -> Option<i64> {
        match self
            .view
            .playlist_entries
            .get(self.management_panel.playlist.selected_row())
        {
            Some(PlaylistPanelEntry::Playlist { playlist_id, .. })
            | Some(PlaylistPanelEntry::Track { playlist_id, .. }) => Some(*playlist_id),
            None => self.management_panel.playlist.active_playlist_id(),
        }
    }

    pub(super) fn move_playlist_selection(&mut self, direction: i32, amount: usize) {
        if self.view.playlist_entries.is_empty() {
            self.management_panel.playlist.clamp_selection(0);
            return;
        }
        self.management_panel.playlist.move_selection(
            direction,
            amount,
            self.view.playlist_entries.len(),
        );
        if let Some(playlist_id) = self.selected_playlist_entry_playlist_id() {
            self.management_panel
                .playlist
                .set_active_playlist_id(Some(playlist_id));
        }
        self.apply_selection_state();
    }

    pub(super) fn activate_playlist_selection(&mut self, conn: &Connection) -> Result<()> {
        match self
            .view
            .playlist_entries
            .get(self.management_panel.playlist.selected_row())
        {
            Some(PlaylistPanelEntry::Playlist { playlist_id, .. }) => {
                if let Some(entry) = self.first_playlist_panel_playback_entry(*playlist_id) {
                    self.play_entry(conn, entry)?;
                } else {
                    self.message = String::from("no tracks in this playlist");
                }
            }
            Some(PlaylistPanelEntry::Track {
                playlist_id,
                playlist_track_id,
                track_index,
                ..
            }) => {
                self.play_entry(
                    conn,
                    PlaybackEntry::playlist_track(*playlist_id, *playlist_track_id, *track_index),
                )?;
            }
            None => self.message = String::from("no playlist selection"),
        }
        Ok(())
    }

    pub(super) fn first_playlist_panel_playback_entry(
        &self,
        playlist_id: i64,
    ) -> Option<PlaybackEntry> {
        self.playlist_playback_entries(playlist_id)
            .into_iter()
            .next()
    }

    pub(super) fn command_playlist(&mut self, conn: &Connection, name: &str) -> Result<String> {
        self.clear_command_output();
        let playlist = if name.trim().is_empty() {
            if let Some(playlist_id) = self.management_panel.playlist.active_playlist_id() {
                self.playlists
                    .iter()
                    .find(|playlist| playlist.id == playlist_id)
                    .cloned()
                    .or_else(|| self.playlists.first().cloned())
            } else {
                self.playlists.first().cloned()
            }
            .map(Ok)
            .unwrap_or_else(|| db::create_playlist(conn, "Default"))?
        } else {
            db::create_playlist(conn, name)?
        };

        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.management_panel
            .playlist
            .activate_and_expand(playlist.id);
        self.management_panel.show_playlist();
        if self.focus == FocusPane::Keymap {
            self.focus = FocusPane::Playlist;
        }
        self.sync_selection();
        self.select_playlist_row_for_id(playlist.id);
        Ok(format!("playlist: {}", playlist.name))
    }

    pub(super) fn command_playlist_clear(
        &mut self,
        conn: &Connection,
        name: &str,
    ) -> Result<String> {
        let Some(playlist) = self.command_playlist_target(conn, name)? else {
            return Ok(String::from("no playlist selected"));
        };
        let removed = db::clear_playlist(conn, playlist.id)?;
        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.management_panel
            .playlist
            .activate_and_expand(playlist.id);
        self.management_panel.show_playlist();
        if self.focus == FocusPane::Keymap {
            self.focus = FocusPane::Playlist;
        }
        self.sync_selection();
        self.select_playlist_row_for_id(playlist.id);
        Ok(format!("cleared {removed} tracks from {}", playlist.name))
    }

    pub(super) fn command_playlist_delete(
        &mut self,
        conn: &Connection,
        name: &str,
    ) -> Result<String> {
        let Some(playlist) = self.command_playlist_target(conn, name)? else {
            return Ok(String::from("no playlist selected"));
        };
        if db::delete_playlist(conn, &playlist.name)? {
            self.playlists = db::playlists(conn)?;
            self.refresh_playlist_tracks(conn)?;
            self.management_panel
                .playlist
                .collapse_playlist(playlist.id);
            let fallback_playlist_id = self.playlists.first().map(|playlist| playlist.id);
            self.management_panel
                .playlist
                .set_active_playlist_id(fallback_playlist_id);
            self.sync_selection();
            if let Some(playlist_id) = fallback_playlist_id {
                self.select_playlist_row_for_id(playlist_id);
                self.apply_selection_state();
            }
            Ok(format!("deleted playlist {}", playlist.name))
        } else {
            Ok(format!("no playlist: {}", playlist.name))
        }
    }

    pub(super) fn command_playlist_target(
        &self,
        conn: &Connection,
        name: &str,
    ) -> Result<Option<db::Playlist>> {
        if !name.trim().is_empty() {
            return db::playlist_by_name(conn, name);
        }
        Ok(self
            .management_panel
            .playlist
            .active_playlist_id()
            .and_then(|playlist_id| {
                self.playlists
                    .iter()
                    .find(|playlist| playlist.id == playlist_id)
                    .cloned()
            })
            .or_else(|| self.playlists.first().cloned()))
    }

    pub(super) fn open_playlist_panel(&mut self, conn: &Connection) -> Result<()> {
        if self.playlists.is_empty() {
            let playlist = db::create_playlist(conn, "Default")?;
            self.management_panel
                .playlist
                .activate_and_expand(playlist.id);
            self.playlists = db::playlists(conn)?;
            self.refresh_playlist_tracks(conn)?;
        }

        self.management_panel.show_playlist();
        if self
            .management_panel
            .playlist
            .active_playlist_id()
            .is_none()
        {
            self.management_panel
                .playlist
                .set_active_playlist_id(self.playlists.first().map(|playlist| playlist.id));
        }
        if let Some(playlist_id) = self.management_panel.playlist.active_playlist_id() {
            self.management_panel.playlist.expand_playlist(playlist_id);
            self.sync_selection_preserving_browser_selection();
            self.select_playlist_row_for_id(playlist_id);
        }
        self.focus = FocusPane::Playlist;
        self.apply_selection_state();
        self.message = String::from("playlist panel");
        Ok(())
    }

    pub(super) fn select_playlist_row_for_id(&mut self, playlist_id: i64) {
        if let Some(position) = self.view.playlist_entries.iter().position(|entry| {
            matches!(
                entry,
                PlaylistPanelEntry::Playlist {
                    playlist_id: entry_id,
                    ..
                } if *entry_id == playlist_id
            )
        }) {
            self.management_panel.playlist.select_row(position);
        }
        self.management_panel
            .playlist
            .set_active_playlist_id(Some(playlist_id));
    }

    fn selected_playlist_entry_anchor(&self) -> Option<PlaylistSelectionAnchor> {
        self.view
            .playlist_entries
            .get(self.management_panel.playlist.selected_row())
            .map(|entry| match entry {
                PlaylistPanelEntry::Playlist { playlist_id, .. } => {
                    PlaylistSelectionAnchor::Playlist {
                        playlist_id: *playlist_id,
                    }
                }
                PlaylistPanelEntry::Track {
                    playlist_id,
                    playlist_track_id,
                    ..
                } => PlaylistSelectionAnchor::Track {
                    playlist_id: *playlist_id,
                    playlist_track_id: *playlist_track_id,
                },
            })
    }

    fn select_playlist_entry_anchor(&mut self, anchor: PlaylistSelectionAnchor) -> bool {
        let Some(position) =
            self.view
                .playlist_entries
                .iter()
                .position(|entry| match (entry, anchor) {
                    (
                        PlaylistPanelEntry::Playlist { playlist_id, .. },
                        PlaylistSelectionAnchor::Playlist {
                            playlist_id: anchor_id,
                        },
                    ) => *playlist_id == anchor_id,
                    (
                        PlaylistPanelEntry::Track {
                            playlist_id,
                            playlist_track_id,
                            ..
                        },
                        PlaylistSelectionAnchor::Track {
                            playlist_id: anchor_playlist_id,
                            playlist_track_id: anchor_track_id,
                        },
                    ) => {
                        *playlist_id == anchor_playlist_id && *playlist_track_id == anchor_track_id
                    }
                    _ => false,
                })
        else {
            return false;
        };

        self.management_panel.playlist.select_row(position);
        self.management_panel
            .playlist
            .set_active_playlist_id(Some(anchor.playlist_id()));
        true
    }

    pub(super) fn toggle_selected_playlist_expansion(&mut self) {
        let Some(playlist_id) = self.selected_playlist_entry_playlist_id() else {
            self.message = String::from("no playlist selected");
            return;
        };
        self.management_panel
            .playlist
            .set_active_playlist_id(Some(playlist_id));
        if self
            .management_panel
            .playlist
            .collapse_playlist(playlist_id)
        {
            self.select_playlist_row_for_id(playlist_id);
            self.message = format!("collapsed {}", self.playlist_name(playlist_id));
        } else {
            self.management_panel.playlist.expand_playlist(playlist_id);
            self.message = format!("expanded {}", self.playlist_name(playlist_id));
        }
    }

    pub(super) fn add_selected_tracks_to_active_playlist(
        &mut self,
        conn: &Connection,
    ) -> Result<()> {
        if !self.management_panel.playlist_open() {
            self.message = String::from("open playlist panel with p before editing playlists");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }
        let playlist_selection_anchor = (self.focus == FocusPane::Playlist)
            .then(|| self.selected_playlist_entry_anchor())
            .flatten();
        let playlist_id = self.ensure_active_playlist(conn)?;
        let media_item_ids = if self.focus == FocusPane::Playlist {
            self.selected_playlist_track_media_item_id()
                .map(|id| vec![id])
                .unwrap_or_else(|| self.selected_source_media_item_ids())
        } else {
            self.selected_source_media_item_ids()
        };
        if media_item_ids.is_empty() {
            self.message = String::from("no selected tracks to add");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }

        let added = db::add_tracks_to_playlist(conn, playlist_id, &media_item_ids)?;
        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.management_panel
            .playlist
            .activate_and_expand(playlist_id);
        self.sync_selection_preserving_browser_selection();
        let restored_selection = playlist_selection_anchor
            .map(|anchor| self.select_playlist_entry_anchor(anchor))
            .unwrap_or(false);
        if !restored_selection {
            self.select_playlist_row_for_id(playlist_id);
        }
        self.apply_selection_state();
        self.message = format!(
            "added {added} tracks to {}",
            self.playlist_name(playlist_id)
        );
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn remove_selected_tracks_from_active_playlist(
        &mut self,
        conn: &Connection,
    ) -> Result<()> {
        if !self.management_panel.playlist_open() {
            self.message = String::from("open playlist panel with p before editing playlists");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }
        let Some(playlist_id) = self.management_panel.playlist.active_playlist_id() else {
            self.message = String::from("no active playlist");
            self.show_transient_status(self.message.clone());
            return Ok(());
        };

        let mut target_playlist_id = playlist_id;
        let removed = match self.focus {
            FocusPane::Playlist => {
                if let Some((entry_playlist_id, playlist_track_id, _media_item_id)) =
                    self.selected_playlist_track()
                {
                    target_playlist_id = entry_playlist_id;
                    db::remove_playlist_track_entries(
                        conn,
                        entry_playlist_id,
                        &[playlist_track_id],
                    )?
                } else {
                    0
                }
            }
            FocusPane::Tracks => {
                let media_item_ids = self.selected_source_media_item_ids();
                db::remove_latest_tracks_from_playlist(conn, playlist_id, &media_item_ids)?
            }
            FocusPane::Tree => {
                let media_item_ids = self.selected_source_media_item_ids();
                db::remove_tracks_from_playlist(conn, playlist_id, &media_item_ids)?
            }
            FocusPane::Keymap => 0,
        };
        if removed == 0 {
            self.message = String::from("no selected tracks to remove");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }

        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.management_panel
            .playlist
            .set_active_playlist_id(Some(target_playlist_id));
        self.sync_selection_preserving_browser_selection();
        self.message = format!(
            "removed {removed} tracks from {}",
            self.playlist_name(target_playlist_id)
        );
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn ensure_active_playlist(&mut self, conn: &Connection) -> Result<i64> {
        if let Some(playlist_id) = self.management_panel.playlist.active_playlist_id() {
            return Ok(playlist_id);
        }
        let playlist = db::create_playlist(conn, "Default")?;
        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.management_panel
            .playlist
            .set_active_playlist_id(Some(playlist.id));
        Ok(playlist.id)
    }

    pub(super) fn selected_source_media_item_ids(&self) -> Vec<i64> {
        let indices: Vec<usize> = match self.focus {
            FocusPane::Tree => self
                .selected_scope_tracks()
                .into_iter()
                .map(|(index, _track)| index)
                .collect(),
            FocusPane::Tracks => self.selected_playable_track_index().into_iter().collect(),
            FocusPane::Playlist => self
                .selected_playlist_track()
                .and_then(|(_playlist_id, _playlist_track_id, media_item_id)| {
                    self.tracks
                        .iter()
                        .position(|track| track.media_item_id == media_item_id)
                })
                .into_iter()
                .collect(),
            FocusPane::Keymap => Vec::new(),
        };
        indices
            .into_iter()
            .filter_map(|index| self.tracks.get(index).map(|track| track.media_item_id))
            .collect()
    }

    pub(super) fn selected_playlist_track_media_item_id(&self) -> Option<i64> {
        self.selected_playlist_track()
            .map(|(_playlist_id, _playlist_track_id, media_item_id)| media_item_id)
    }

    pub(super) fn selected_playlist_track(&self) -> Option<(i64, i64, i64)> {
        let PlaylistPanelEntry::Track {
            playlist_id,
            playlist_track_id,
            track_index,
            ..
        } = self
            .view
            .playlist_entries
            .get(self.management_panel.playlist.selected_row())?
        else {
            return None;
        };
        self.playlist_cache
            .media_item_id_for_entry(*playlist_id, *playlist_track_id)
            .or_else(|| {
                self.tracks
                    .get(*track_index)
                    .map(|track| track.media_item_id)
            })
            .map(|media_item_id| (*playlist_id, *playlist_track_id, media_item_id))
    }

    pub(super) fn playlist_name(&self, playlist_id: i64) -> String {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
            .unwrap_or_else(|| "playlist".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_selection_movement_and_clamping_are_owned_together() {
        let mut panel = PlaylistPanelState::default();
        panel.select_row(2);

        panel.move_selection(1, usize::MAX, 5);
        assert_eq!(panel.selected_row(), 4);

        panel.move_selection(-1, usize::MAX, 5);
        assert_eq!(panel.selected_row(), 0);

        panel.select_row(8);
        panel.clamp_selection(3);
        assert_eq!(panel.selected_row(), 2);

        panel.clamp_selection(0);
        assert_eq!(panel.selected_row(), 0);
    }

    #[test]
    fn activation_expansion_and_selection_have_independent_lifecycles() {
        let mut panel = PlaylistPanelState::default();

        panel.activate_and_expand(7);
        panel.select_row(3);

        assert_eq!(panel.active_playlist_id(), Some(7));
        assert!(panel.playlist_expanded(7));
        assert_eq!(panel.selected_row(), 3);

        assert!(panel.collapse_playlist(7));
        assert!(!panel.playlist_expanded(7));
    }
}
