use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, LibraryTrack};
use crate::player::{self, PlaybackState};

use super::{App, TrackRow, TreeEntry};

const MAX_LISTENED_DELTA_MS: i64 = 10_000;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum PlayTarget {
    Library,
    Artist,
    Album,
}

impl PlayTarget {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Library => Self::Artist,
            Self::Artist => Self::Album,
            Self::Album => Self::Library,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Artist => "artist from library",
            Self::Album => "album from library",
        }
    }
}

pub(super) struct PlaybackModeState {
    target: PlayTarget,
    continuous: bool,
    repeat: bool,
    shuffle: bool,
    shuffle_seed: u64,
    shuffle_scope: Vec<PlaybackEntry>,
    shuffle_order: Vec<PlaybackEntry>,
}

impl Default for PlaybackModeState {
    fn default() -> Self {
        Self {
            target: PlayTarget::Library,
            continuous: true,
            repeat: false,
            shuffle: false,
            shuffle_seed: 0x476d_7573_2026_0528,
            shuffle_scope: Vec::new(),
            shuffle_order: Vec::new(),
        }
    }
}

impl PlaybackModeState {
    pub(super) fn target(&self) -> PlayTarget {
        self.target
    }

    pub(super) fn continuous(&self) -> bool {
        self.continuous
    }

    pub(super) fn repeat(&self) -> bool {
        self.repeat
    }

    pub(super) fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub(super) fn advance_target(&mut self) -> PlayTarget {
        self.target = self.target.next();
        self.reset_sequence();
        self.target
    }

    pub(super) fn toggle_continuous(&mut self) -> bool {
        self.continuous = !self.continuous;
        self.continuous
    }

    pub(super) fn toggle_repeat(&mut self) -> bool {
        self.repeat = !self.repeat;
        self.repeat
    }

    pub(super) fn toggle_shuffle(&mut self) -> bool {
        self.shuffle = !self.shuffle;
        self.reset_sequence();
        self.shuffle
    }

    pub(super) fn reset_sequence(&mut self) {
        self.shuffle_scope.clear();
        self.shuffle_order.clear();
    }

    fn next_entry(
        &mut self,
        sequence: &[PlaybackEntry],
        anchor: Option<PlaybackEntry>,
        selected: Option<PlaybackEntry>,
        direction: i32,
    ) -> Option<PlaybackEntry> {
        if self.shuffle {
            self.next_shuffle_entry(sequence, anchor, selected, direction)
        } else {
            self.next_ordered_entry(sequence, anchor, selected, direction)
        }
    }

    fn next_ordered_entry(
        &self,
        sequence: &[PlaybackEntry],
        anchor: Option<PlaybackEntry>,
        selected: Option<PlaybackEntry>,
        direction: i32,
    ) -> Option<PlaybackEntry> {
        if let Some(anchor) = anchor {
            if let Some(position) = sequence
                .iter()
                .position(|entry| playback_entries_match(*entry, anchor))
            {
                return if direction >= 0 {
                    sequence
                        .get(position + 1)
                        .copied()
                        .or_else(|| self.repeat.then(|| sequence[0]))
                } else {
                    position
                        .checked_sub(1)
                        .and_then(|position| sequence.get(position).copied())
                        .or_else(|| self.repeat.then(|| sequence[sequence.len() - 1]))
                };
            }

            if let Some(selected) = selected.filter(|selected| sequence.contains(selected)) {
                return Some(selected);
            }
        }

        if direction >= 0 {
            sequence.first().copied()
        } else {
            sequence.last().copied()
        }
    }

    fn next_shuffle_entry(
        &mut self,
        sequence: &[PlaybackEntry],
        anchor: Option<PlaybackEntry>,
        selected: Option<PlaybackEntry>,
        direction: i32,
    ) -> Option<PlaybackEntry> {
        self.ensure_shuffle_order(sequence);
        if self.shuffle_order.is_empty() {
            return None;
        }

        if let Some(anchor) = anchor {
            if let Some(position) = self
                .shuffle_order
                .iter()
                .position(|entry| playback_entries_match(*entry, anchor))
            {
                return if direction >= 0 {
                    self.shuffle_order.get(position + 1).copied().or_else(|| {
                        if self.repeat {
                            self.rebuild_shuffle_order(sequence);
                            self.shuffle_order.first().copied()
                        } else {
                            None
                        }
                    })
                } else {
                    position
                        .checked_sub(1)
                        .and_then(|position| self.shuffle_order.get(position).copied())
                        .or_else(|| {
                            if self.repeat {
                                self.shuffle_order.last().copied()
                            } else {
                                None
                            }
                        })
                };
            }

            if let Some(selected) = selected.filter(|selected| sequence.contains(selected)) {
                return Some(selected);
            }
        }

        if direction >= 0 {
            self.shuffle_order.first().copied()
        } else {
            self.shuffle_order.last().copied()
        }
    }

    fn ensure_shuffle_order(&mut self, sequence: &[PlaybackEntry]) {
        if self.shuffle_scope != sequence {
            self.rebuild_shuffle_order(sequence);
        }
    }

    fn rebuild_shuffle_order(&mut self, sequence: &[PlaybackEntry]) {
        self.shuffle_scope = sequence.to_vec();
        self.shuffle_order = sequence.to_vec();
        for index in (1..self.shuffle_order.len()).rev() {
            let swap_with = (self.next_shuffle_u64() as usize) % (index + 1);
            self.shuffle_order.swap(index, swap_with);
        }
    }

    fn next_shuffle_u64(&mut self) -> u64 {
        self.shuffle_seed = self
            .shuffle_seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.shuffle_seed
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlayingTrack {
    pub(super) index: usize,
    pub(super) source: Option<PlaybackSource>,
    pub(super) track: LibraryTrack,
    pub(super) last_position_ms: i64,
    pub(super) listened_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaybackSource {
    PlaylistTrack {
        playlist_id: i64,
        playlist_track_id: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlaybackEntry {
    pub(super) track_index: usize,
    pub(super) source: Option<PlaybackSource>,
}

impl PlaybackEntry {
    pub(super) fn library(track_index: usize) -> Self {
        Self {
            track_index,
            source: None,
        }
    }

    pub(super) fn playlist_track(
        playlist_id: i64,
        playlist_track_id: i64,
        track_index: usize,
    ) -> Self {
        Self {
            track_index,
            source: Some(PlaybackSource::PlaylistTrack {
                playlist_id,
                playlist_track_id,
            }),
        }
    }
}

impl PlayingTrack {
    pub(super) fn tick_position(&mut self, position: Duration, state: PlaybackState) {
        let position_ms = position.as_millis() as i64;
        if state == PlaybackState::Playing {
            let delta = position_ms - self.last_position_ms;
            if delta > 0 && delta <= MAX_LISTENED_DELTA_MS {
                self.listened_ms += delta;
            }
        }
        self.align_position(position_ms);
    }

    pub(super) fn align_position(&mut self, position_ms: i64) {
        self.last_position_ms = position_ms.max(0);
    }
}

impl App {
    pub(super) fn seek_relative(&mut self, delta_seconds: i64) -> Result<()> {
        if self.current.is_none() {
            self.message = String::from("nothing playing");
            return Ok(());
        }
        let current_position_ms = self.current_position_ms();
        let mut next_position_ms = (current_position_ms + delta_seconds * 1000).max(0);
        if let Some(duration_ms) = self
            .current
            .as_ref()
            .and_then(|current| current.track.duration_ms)
        {
            next_position_ms = next_position_ms.min(duration_ms);
        }
        if self.seek_to(next_position_ms)? {
            self.message = format!(
                "seek {}{}s to {}",
                if delta_seconds >= 0 { "+" } else { "" },
                delta_seconds,
                db::format_duration(Some(next_position_ms))
            );
        }
        Ok(())
    }

    pub(super) fn seek_to(&mut self, position_ms: i64) -> Result<bool> {
        let position_ms = position_ms.max(0);
        if self.suspended_position_ms.is_some() {
            self.suspended_position_ms = Some(position_ms);
            if let Some(current) = &mut self.current {
                current.align_position(position_ms);
            }
            self.sync_integration_playback(true);
            return Ok(true);
        }

        let position = Duration::from_millis(position_ms as u64);
        self.capture_current_progress();
        if let Err(error) = self.player.seek(position) {
            self.message = format!("seek failed: {error:#}");
            return Ok(false);
        }
        if let Some(current) = &mut self.current {
            current.align_position(position_ms);
        }
        self.sync_integration_playback(true);
        Ok(true)
    }

    pub(super) fn capture_current_progress(&mut self) {
        if self.current.is_none() || self.suspended_position_ms.is_some() {
            return;
        }
        let state = self.player.state();
        let position = self.player.position();
        if let Some(current) = &mut self.current {
            current.tick_position(position, state);
        }
    }

    pub(super) fn suspend_current(&mut self) -> Result<()> {
        if self.current.is_none() {
            self.message = String::from("nothing playing");
            return Ok(());
        }
        if self.suspended_position_ms.is_some() {
            self.message = String::from("paused");
            return Ok(());
        }

        self.capture_current_progress();
        let position_ms = self.player.position().as_millis() as i64;
        self.player.stop()?;
        if let Some(current) = &mut self.current {
            current.align_position(position_ms);
        }
        self.suspended_position_ms = Some(position_ms);
        self.message = String::from("paused");
        self.sync_integration_playback(true);
        Ok(())
    }

    pub(super) fn resume_current(&mut self) -> Result<()> {
        let Some(current) = self.current.as_ref() else {
            self.message = String::from("nothing playing");
            return Ok(());
        };
        let Some(position_ms) = self.suspended_position_ms else {
            self.player.play()?;
            self.message = String::from("playing");
            self.sync_integration_playback(true);
            return Ok(());
        };

        let path = current.track.path.clone();
        if let Err(error) = self.player.load_and_play(Path::new(&path)) {
            self.message = format!("could not resume {path}: {error:#}");
            self.sync_integration_playback(true);
            return Ok(());
        }
        if position_ms > 0 {
            if let Err(error) = self
                .player
                .seek(Duration::from_millis(position_ms.max(0) as u64))
            {
                let _ = self.player.stop();
                if let Some(current) = &mut self.current {
                    current.align_position(position_ms);
                }
                self.message = format!("seek failed: {error:#}");
                self.sync_integration_playback(true);
                return Ok(());
            }
        }
        self.suspended_position_ms = None;
        if let Some(current) = &mut self.current {
            current.align_position(position_ms);
        }
        self.message = String::from("playing");
        self.sync_integration_playback(true);
        Ok(())
    }

    pub(super) fn current_position_ms(&self) -> i64 {
        if let Some(position_ms) = self.suspended_position_ms {
            position_ms
        } else if self.current.is_some() {
            self.player.position().as_millis() as i64
        } else {
            0
        }
    }

    pub(super) fn logical_state(&self) -> PlaybackState {
        if self.current.is_some() && self.suspended_position_ms.is_some() {
            PlaybackState::Paused
        } else {
            self.player.state()
        }
    }

    pub(super) fn play_selected_row(&mut self, conn: &Connection) -> Result<()> {
        if let Some(entry) = self.selected_playback_entry() {
            self.play_entry(conn, entry)?;
        }
        Ok(())
    }

    pub(super) fn play_next(&mut self, conn: &Connection) -> Result<()> {
        if let Some(entry) = self.next_playback_entry(1) {
            self.play_entry(conn, entry)?;
        } else {
            self.message = String::from("end of filtered playback view");
        }
        Ok(())
    }

    pub(super) fn play_previous(&mut self, conn: &Connection) -> Result<()> {
        if let Some(entry) = self.next_playback_entry(-1) {
            self.play_entry(conn, entry)?;
        } else {
            self.message = String::from("start of filtered playback view");
        }
        Ok(())
    }

    pub(super) fn play_from_controls(&mut self, conn: &Connection) -> Result<()> {
        match self.logical_state() {
            PlaybackState::Paused => self.resume_current()?,
            PlaybackState::Playing => {
                self.message = String::from("already playing");
            }
            PlaybackState::Stopped => self.play_selected_row(conn)?,
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn play_index(&mut self, conn: &Connection, index: usize) -> Result<()> {
        self.play_entry(conn, PlaybackEntry::library(index))
    }

    pub(super) fn play_entry(&mut self, conn: &Connection, entry: PlaybackEntry) -> Result<()> {
        if entry.track_index >= self.tracks.len() {
            return Ok(());
        }

        self.finish_current(conn, false)?;
        let track = self.tracks[entry.track_index].clone();
        match self.player.load_and_play(Path::new(&track.path)) {
            Ok(()) => {
                self.suspended_position_ms = None;
                self.message = format!("playing {}", track.display_title());
                self.current = Some(PlayingTrack {
                    index: entry.track_index,
                    source: entry.source,
                    track,
                    last_position_ms: 0,
                    listened_ms: 0,
                });
                if self.restore_track {
                    self.save_current_track_selection(conn)?;
                    self.select_current_track_for_restore();
                }
                self.publish_track_changed();
                self.sync_integration_playback(true);
            }
            Err(error) => {
                let _ = self.player.stop();
                self.message = format!("could not play {}: {error:#}", track.path);
                self.sync_integration_playback(true);
            }
        }
        Ok(())
    }

    pub(super) fn toggle_pause(&mut self, conn: &Connection) -> Result<()> {
        match self.logical_state() {
            PlaybackState::Playing => self.suspend_current()?,
            PlaybackState::Paused => self.resume_current()?,
            PlaybackState::Stopped => self.play_selected_row(conn)?,
        }
        Ok(())
    }

    pub(super) fn stop_current(&mut self, conn: &Connection) -> Result<()> {
        self.finish_current(conn, false)?;
        self.player.stop()?;
        self.message = String::from("stopped");
        self.sync_integration_playback(true);
        Ok(())
    }

    fn finish_current(&mut self, conn: &Connection, natural_end: bool) -> Result<()> {
        let Some(mut current) = self.current.clone() else {
            return Ok(());
        };
        if let Some(position_ms) = self.suspended_position_ms {
            current.align_position(position_ms);
        } else {
            current.tick_position(self.player.position(), self.player.state());
        }
        let mut played_ms = current.listened_ms;
        if natural_end {
            if let Some(duration_ms) = current.track.duration_ms {
                played_ms = played_ms.max(duration_ms);
            }
        }
        let completed = natural_end
            || player::play_count_threshold_met(current.track.duration_ms, current.listened_ms);

        if played_ms > 0 || natural_end {
            db::record_play(
                conn,
                current.track.media_item_id,
                current.track.location_id,
                played_ms,
                completed,
            )?;
            self.message = if completed {
                format!("counted play for {}", current.track.display_title())
            } else {
                format!(
                    "recorded partial play for {}",
                    current.track.display_title()
                )
            };
            if completed {
                self.increment_cached_play_count(current.track.media_item_id);
            }
        }
        self.current = None;
        self.suspended_position_ms = None;
        Ok(())
    }

    pub(super) fn update_playback(&mut self, conn: &Connection) -> Result<bool> {
        if self.current.is_none() {
            return Ok(false);
        }
        if self.suspended_position_ms.is_some() {
            self.sync_integration_playback(false);
            return Ok(false);
        }

        self.capture_current_progress();
        if self.player.output_failed() {
            let position_ms = self.player.position().as_millis() as i64;
            self.player.stop()?;
            if let Some(current) = &mut self.current {
                current.align_position(position_ms);
            }
            self.suspended_position_ms = Some(position_ms);
            self.message = String::from("audio output disconnected; paused");
            self.sync_integration_playback(true);
            return Ok(true);
        }

        let mut changed = false;

        if self.current.is_some() && self.player.is_finished() {
            let next_entry = self.next_auto_advance_entry();
            self.finish_current(conn, true)?;
            if let Some(entry) = next_entry {
                self.play_entry(conn, entry)?;
            } else {
                self.player.stop()?;
            }
            changed = true;
        }
        self.sync_integration_playback(false);
        Ok(changed)
    }

    pub(super) fn shutdown(&mut self, conn: &Connection) -> Result<()> {
        let playback_result = self.finish_current(conn, false).and_then(|()| {
            self.player.stop()?;
            self.sync_integration_playback(true);
            Ok(())
        });
        let library_job_result = self.finish_library_job();
        playback_result.and(library_job_result)
    }

    fn increment_cached_play_count(&mut self, media_item_id: i64) {
        for track in &mut self.tracks {
            if track.media_item_id == media_item_id {
                track.play_count += 1;
            }
        }
    }

    pub(super) fn playlist_playback_entries(&self, playlist_id: i64) -> Vec<PlaybackEntry> {
        let filtered: HashSet<usize> = self.view.filtered_indices.iter().copied().collect();
        self.playlist_cache
            .playable_entries(playlist_id)
            .filter_map(|entry| {
                let track_index = entry.track_index?;
                filtered.contains(&track_index).then(|| {
                    PlaybackEntry::playlist_track(playlist_id, entry.playlist_track_id, track_index)
                })
            })
            .collect()
    }

    pub(super) fn selected_playable_track_index(&self) -> Option<usize> {
        self.selected_playback_entry()
            .map(|entry| entry.track_index)
    }

    pub(super) fn selected_playable_media_item_id(&self) -> Option<i64> {
        let index = self.selected_playable_track_index()?;
        self.tracks.get(index).map(|track| track.media_item_id)
    }

    pub(super) fn selected_playback_entry(&self) -> Option<PlaybackEntry> {
        let rows = self.track_rows();
        if let Some(entry) = rows
            .get(self.browser.selected_track_row())
            .and_then(track_row_playback_entry)
        {
            return Some(entry);
        }

        rows.iter()
            .skip(self.browser.selected_track_row())
            .find_map(track_row_playback_entry)
            .or_else(|| rows.iter().rev().find_map(track_row_playback_entry))
    }

    pub(super) fn first_selected_tree_playback_entry(&self) -> Option<PlaybackEntry> {
        let entry = self.selected_tree_entry()?;
        if matches!(entry, TreeEntry::Playlists | TreeEntry::Playlist { .. }) {
            return self
                .playback_entries_for_tree_entry(entry)
                .into_iter()
                .next();
        }
        self.selected_scope_tracks()
            .first()
            .map(|(index, _track)| PlaybackEntry::library(*index))
    }

    #[cfg(test)]
    pub(super) fn next_playback_index(&mut self, direction: i32) -> Option<usize> {
        self.next_playback_entry(direction)
            .map(|entry| entry.track_index)
    }

    pub(super) fn next_playback_entry(&mut self, direction: i32) -> Option<PlaybackEntry> {
        let sequence = self.playback_sequence_entries();
        if sequence.is_empty() {
            return None;
        }

        let anchor = self.playback_anchor_entry();
        let selected = anchor
            .filter(|anchor| !playback_sequence_contains_anchor(&sequence, *anchor))
            .and_then(|_| self.selected_playback_entry());
        self.playback_mode
            .next_entry(&sequence, anchor, selected, direction)
    }

    pub(super) fn next_auto_advance_entry(&mut self) -> Option<PlaybackEntry> {
        if self.playback_mode.continuous() {
            self.next_playback_entry(1)
        } else {
            None
        }
    }

    #[cfg(test)]
    pub(super) fn next_auto_advance_index(&mut self) -> Option<usize> {
        self.next_auto_advance_entry()
            .map(|entry| entry.track_index)
    }

    #[cfg(test)]
    pub(super) fn playback_sequence_indices(&self) -> Vec<usize> {
        self.playback_sequence_entries()
            .into_iter()
            .map(|entry| entry.track_index)
            .collect()
    }

    fn playback_sequence_entries(&self) -> Vec<PlaybackEntry> {
        let Some(anchor) = self.playback_anchor_entry() else {
            return self.library_playback_entries();
        };
        let Some(anchor_track) = self.tracks.get(anchor.track_index) else {
            return self.library_playback_entries();
        };

        if let Some(entry) = self.selected_tree_entry() {
            if matches!(entry, TreeEntry::Playlists | TreeEntry::Playlist { .. }) {
                let scope = self.playback_entries_for_tree_entry(entry);
                if playback_sequence_contains_anchor(&scope, anchor) {
                    return scope;
                }
            }
        }

        match self.playback_mode.target() {
            PlayTarget::Library => self.library_playback_entries(),
            PlayTarget::Artist => self
                .view
                .filtered_indices
                .iter()
                .copied()
                .filter(|index| {
                    self.tracks
                        .get(*index)
                        .map(|track| track.tree_artist() == anchor_track.tree_artist())
                        .unwrap_or(false)
                })
                .map(PlaybackEntry::library)
                .collect(),
            PlayTarget::Album => self
                .view
                .filtered_indices
                .iter()
                .copied()
                .filter(|index| {
                    self.tracks
                        .get(*index)
                        .map(|track| {
                            track.tree_artist() == anchor_track.tree_artist()
                                && track.tree_album() == anchor_track.tree_album()
                        })
                        .unwrap_or(false)
                })
                .map(PlaybackEntry::library)
                .collect(),
        }
    }

    fn playback_anchor_entry(&self) -> Option<PlaybackEntry> {
        self.current
            .as_ref()
            .and_then(|current| self.playback_entry_for_current(current))
            .or_else(|| self.selected_playback_entry())
    }

    fn playback_entry_for_current(&self, current: &PlayingTrack) -> Option<PlaybackEntry> {
        let track_index = self.track_index_for_media_item_id(current.track.media_item_id)?;
        match current.source {
            Some(PlaybackSource::PlaylistTrack {
                playlist_id,
                playlist_track_id,
            }) => self
                .playlist_cache
                .track_index_for_entry(playlist_id, playlist_track_id)
                .map(|track_index| {
                    PlaybackEntry::playlist_track(playlist_id, playlist_track_id, track_index)
                })
                .or_else(|| Some(PlaybackEntry::library(track_index))),
            None => Some(PlaybackEntry::library(track_index)),
        }
    }

    pub(super) fn sync_current_track_index(&mut self) {
        let Some(index) = self
            .current
            .as_ref()
            .and_then(|current| self.track_index_for_media_item_id(current.track.media_item_id))
        else {
            return;
        };
        let track = self.tracks[index].clone();
        if let Some(current) = &mut self.current {
            current.index = index;
            current.track = track;
        }
    }

    fn library_playback_entries(&self) -> Vec<PlaybackEntry> {
        self.view
            .filtered_indices
            .iter()
            .copied()
            .map(PlaybackEntry::library)
            .collect()
    }

    fn playback_entries_for_tree_entry(&self, entry: &TreeEntry) -> Vec<PlaybackEntry> {
        match entry {
            TreeEntry::Playlists => self
                .playlists
                .iter()
                .flat_map(|playlist| self.playlist_playback_entries(playlist.id))
                .collect(),
            TreeEntry::Playlist { playlist_id, .. } => self.playlist_playback_entries(*playlist_id),
            _ => self
                .track_indices_for_entry(entry)
                .into_iter()
                .map(PlaybackEntry::library)
                .collect(),
        }
    }

    pub(super) fn reset_shuffle_order(&mut self) {
        self.playback_mode.reset_sequence();
    }
}

fn track_row_playback_entry(row: &TrackRow) -> Option<PlaybackEntry> {
    match row {
        TrackRow::Track { track_index, .. } => Some(PlaybackEntry::library(*track_index)),
        TrackRow::PlaylistTrack {
            playlist_id,
            playlist_track_id,
            track_index,
            ..
        } => Some(PlaybackEntry::playlist_track(
            *playlist_id,
            *playlist_track_id,
            *track_index,
        )),
        TrackRow::AlbumHeader { .. }
        | TrackRow::DiscDivider { .. }
        | TrackRow::PlaylistHeader { .. } => None,
    }
}

fn playback_entries_match(entry: PlaybackEntry, anchor: PlaybackEntry) -> bool {
    if anchor.source.is_some() {
        entry == anchor
    } else {
        entry.track_index == anchor.track_index
    }
}

fn playback_sequence_contains_anchor(sequence: &[PlaybackEntry], anchor: PlaybackEntry) -> bool {
    sequence
        .iter()
        .any(|entry| playback_entries_match(*entry, anchor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_uses_a_permuted_playback_order() {
        let sequence = [
            PlaybackEntry::library(0),
            PlaybackEntry::library(1),
            PlaybackEntry::library(2),
        ];
        let mut mode = PlaybackModeState {
            shuffle: true,
            shuffle_seed: 1,
            ..PlaybackModeState::default()
        };

        let next = mode.next_entry(&sequence, None, None, 1);

        assert!(next.is_some());
        assert_eq!(mode.shuffle_scope, sequence);
        assert_eq!(mode.shuffle_order.len(), 3);
        assert_ne!(mode.shuffle_order, sequence);
    }

    #[test]
    fn shuffle_repeat_rebuilds_forward_and_reuses_order_backward() {
        let sequence = [
            PlaybackEntry::library(0),
            PlaybackEntry::library(1),
            PlaybackEntry::library(2),
        ];
        let mut mode = PlaybackModeState {
            repeat: true,
            shuffle: true,
            shuffle_seed: 1,
            shuffle_scope: sequence.to_vec(),
            shuffle_order: sequence.to_vec(),
            ..PlaybackModeState::default()
        };

        let backward = mode.next_entry(&sequence, Some(sequence[0]), None, -1);
        let seed_before_forward = mode.shuffle_seed;
        let forward = mode.next_entry(&sequence, Some(sequence[2]), None, 1);

        assert_eq!(backward, Some(sequence[2]));
        assert_ne!(mode.shuffle_seed, seed_before_forward);
        assert_eq!(forward, mode.shuffle_order.first().copied());
    }

    #[test]
    fn target_and_shuffle_changes_invalidate_only_cached_order() {
        let scope = vec![PlaybackEntry::library(0), PlaybackEntry::library(1)];
        let seed = 7;
        let mut mode = PlaybackModeState {
            continuous: false,
            repeat: true,
            shuffle: true,
            shuffle_seed: seed,
            shuffle_scope: scope.clone(),
            shuffle_order: scope.clone(),
            ..PlaybackModeState::default()
        };

        assert_eq!(mode.advance_target(), PlayTarget::Artist);
        assert!(mode.shuffle_scope.is_empty());
        assert!(mode.shuffle_order.is_empty());
        assert!(!mode.continuous);
        assert!(mode.repeat);
        assert!(mode.shuffle);
        assert_eq!(mode.shuffle_seed, seed);

        mode.shuffle_scope = scope.clone();
        mode.shuffle_order = scope;

        assert!(!mode.toggle_shuffle());
        assert!(mode.shuffle_scope.is_empty());
        assert!(mode.shuffle_order.is_empty());
        assert_eq!(mode.target, PlayTarget::Artist);
        assert_eq!(mode.shuffle_seed, seed);
    }
}
