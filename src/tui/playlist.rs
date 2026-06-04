use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::db;

use super::playback::PlaybackEntry;
use super::{App, FocusPane};

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

impl App {
    pub(super) fn refresh_playlist_tracks(&mut self, conn: &Connection) -> Result<()> {
        self.playlist_track_ids.clear();
        self.playlist_track_entry_ids.clear();
        self.playlist_track_indices.clear();
        let media_index: HashMap<i64, usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.media_item_id, index))
            .collect();

        for playlist in &self.playlists {
            let entries = db::playlist_tracks(conn, playlist.id)?;
            let ids = entries
                .iter()
                .map(|entry| entry.media_item_id)
                .collect::<Vec<_>>();
            let mut entry_ids = Vec::new();
            let mut indices = Vec::new();
            for entry in &entries {
                if let Some(index) = media_index.get(&entry.media_item_id).copied() {
                    entry_ids.push(entry.id);
                    indices.push(index);
                }
            }
            self.playlist_track_ids.insert(playlist.id, ids);
            self.playlist_track_entry_ids.insert(playlist.id, entry_ids);
            self.playlist_track_indices.insert(playlist.id, indices);
        }

        if self
            .active_playlist_id
            .is_none_or(|id| !self.playlists.iter().any(|playlist| playlist.id == id))
        {
            self.active_playlist_id = self.playlists.first().map(|playlist| playlist.id);
        }
        Ok(())
    }

    pub(super) fn clamp_playlist_selection(&mut self) {
        let row_len = self.view.playlist_entries.len();
        self.selected_playlist_row = if row_len == 0 {
            0
        } else {
            self.selected_playlist_row.min(row_len - 1)
        };
        if let Some(playlist_id) = self.selected_playlist_entry_playlist_id() {
            self.active_playlist_id = Some(playlist_id);
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
            if self.expanded_playlists.contains(&playlist.id) {
                if let (Some(entry_ids), Some(indices)) = (
                    self.playlist_track_entry_ids.get(&playlist.id),
                    self.playlist_track_indices.get(&playlist.id),
                ) {
                    self.view.playlist_entries.extend(
                        entry_ids
                            .iter()
                            .copied()
                            .zip(indices.iter().copied())
                            .enumerate()
                            .map(|(position, (playlist_track_id, track_index))| {
                                PlaylistPanelEntry::Track {
                                    playlist_id: playlist.id,
                                    playlist_track_id,
                                    position: position + 1,
                                    track_index,
                                }
                            }),
                    );
                }
            }
        }
    }

    pub(super) fn selected_playlist_entry_playlist_id(&self) -> Option<i64> {
        match self.view.playlist_entries.get(self.selected_playlist_row) {
            Some(PlaylistPanelEntry::Playlist { playlist_id, .. })
            | Some(PlaylistPanelEntry::Track { playlist_id, .. }) => Some(*playlist_id),
            None => self.active_playlist_id,
        }
    }

    pub(super) fn move_playlist_selection(&mut self, direction: i32, amount: usize) {
        if self.view.playlist_entries.is_empty() {
            self.selected_playlist_row = 0;
            return;
        }

        if direction >= 0 {
            self.selected_playlist_row =
                (self.selected_playlist_row + amount).min(self.view.playlist_entries.len() - 1);
        } else {
            self.selected_playlist_row = self.selected_playlist_row.saturating_sub(amount);
        }
        if let Some(playlist_id) = self.selected_playlist_entry_playlist_id() {
            self.active_playlist_id = Some(playlist_id);
        }
        self.apply_selection_state();
    }

    pub(super) fn activate_playlist_selection(&mut self, conn: &Connection) -> Result<()> {
        match self.view.playlist_entries.get(self.selected_playlist_row) {
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
            if let Some(playlist_id) = self.active_playlist_id {
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
        self.active_playlist_id = Some(playlist.id);
        self.expanded_playlists.insert(playlist.id);
        self.playlist_panel_open = true;
        self.keymap_panel_open = false;
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
        self.active_playlist_id = Some(playlist.id);
        self.expanded_playlists.insert(playlist.id);
        self.playlist_panel_open = true;
        self.keymap_panel_open = false;
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
            self.expanded_playlists.remove(&playlist.id);
            self.active_playlist_id = self.playlists.first().map(|playlist| playlist.id);
            self.sync_selection();
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
            .active_playlist_id
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
            self.active_playlist_id = Some(playlist.id);
            self.expanded_playlists.insert(playlist.id);
            self.playlists = db::playlists(conn)?;
            self.refresh_playlist_tracks(conn)?;
        }

        self.playlist_panel_open = true;
        self.keymap_panel_open = false;
        if self.active_playlist_id.is_none() {
            self.active_playlist_id = self.playlists.first().map(|playlist| playlist.id);
        }
        if let Some(playlist_id) = self.active_playlist_id {
            self.expanded_playlists.insert(playlist_id);
            self.sync_selection_preserving_browser_selection();
            self.select_playlist_row_for_id(playlist_id);
        }
        self.focus = FocusPane::Playlist;
        self.apply_selection_state();
        self.message = String::from("playlist panel");
        Ok(())
    }

    pub(super) fn show_track_info_panel(&mut self) {
        self.playlist_panel_open = false;
        self.keymap_panel_open = false;
        self.info_panel_visible = true;
        if matches!(self.focus, FocusPane::Playlist | FocusPane::Keymap) {
            self.focus = FocusPane::Tree;
        }
        self.apply_selection_state();
        self.message = String::from("track info panel");
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
            self.selected_playlist_row = position;
        }
        self.active_playlist_id = Some(playlist_id);
    }

    pub(super) fn toggle_selected_playlist_expansion(&mut self) {
        let Some(playlist_id) = self.selected_playlist_entry_playlist_id() else {
            self.message = String::from("no playlist selected");
            return;
        };
        self.active_playlist_id = Some(playlist_id);
        if self.expanded_playlists.remove(&playlist_id) {
            self.select_playlist_row_for_id(playlist_id);
            self.message = format!("collapsed {}", self.playlist_name(playlist_id));
        } else {
            self.expanded_playlists.insert(playlist_id);
            self.message = format!("expanded {}", self.playlist_name(playlist_id));
        }
    }

    pub(super) fn add_selected_tracks_to_active_playlist(
        &mut self,
        conn: &Connection,
    ) -> Result<()> {
        if !self.playlist_panel_open {
            self.message = String::from("open playlist panel with p before editing playlists");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }
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
        self.expanded_playlists.insert(playlist_id);
        self.active_playlist_id = Some(playlist_id);
        self.sync_selection_preserving_browser_selection();
        self.select_playlist_row_for_id(playlist_id);
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
        if !self.playlist_panel_open {
            self.message = String::from("open playlist panel with p before editing playlists");
            self.show_transient_status(self.message.clone());
            return Ok(());
        }
        let Some(playlist_id) = self.active_playlist_id else {
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
        self.active_playlist_id = Some(target_playlist_id);
        self.sync_selection_preserving_browser_selection();
        self.message = format!(
            "removed {removed} tracks from {}",
            self.playlist_name(target_playlist_id)
        );
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn ensure_active_playlist(&mut self, conn: &Connection) -> Result<i64> {
        if let Some(playlist_id) = self.active_playlist_id {
            return Ok(playlist_id);
        }
        let playlist = db::create_playlist(conn, "Default")?;
        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.active_playlist_id = Some(playlist.id);
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
        } = self.view.playlist_entries.get(self.selected_playlist_row)?
        else {
            return None;
        };
        self.tracks
            .get(*track_index)
            .map(|track| (*playlist_id, *playlist_track_id, track.media_item_id))
    }

    pub(super) fn playlist_name(&self, playlist_id: i64) -> String {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
            .unwrap_or_else(|| "playlist".to_string())
    }
}
