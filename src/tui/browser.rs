use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::db::LibraryTrack;

use super::filter::{track_search_text, FilterQuery};
use super::playback::PlaybackSource;
use super::App;

#[derive(Debug, Clone)]
pub(super) struct BrowserExpansionState {
    expanded_artists: HashSet<String>,
    compilations_expanded: bool,
    playlists_expanded: bool,
}

#[derive(Debug, Default)]
pub(super) struct BrowserState {
    selected_tree: usize,
    selected_track_row: usize,
    expanded_artists: HashSet<String>,
    compilations_expanded: bool,
    playlists_expanded: bool,
}

impl BrowserState {
    pub(super) fn selected_tree(&self) -> usize {
        self.selected_tree
    }

    pub(super) fn select_tree(&mut self, position: usize) {
        self.selected_tree = position;
    }

    pub(super) fn selected_track_row(&self) -> usize {
        self.selected_track_row
    }

    pub(super) fn select_track_row(&mut self, position: usize) {
        self.selected_track_row = position;
    }

    pub(super) fn reset_track_selection(&mut self) {
        self.selected_track_row = 0;
    }

    pub(super) fn reset_selection(&mut self) {
        self.selected_tree = 0;
        self.selected_track_row = 0;
    }

    pub(super) fn move_tree_selection(&mut self, direction: i32, amount: usize, tree_len: usize) {
        if tree_len == 0 {
            return;
        }
        self.selected_tree = if direction >= 0 {
            self.selected_tree.saturating_add(amount).min(tree_len - 1)
        } else {
            self.selected_tree.saturating_sub(amount)
        };
        self.reset_track_selection();
    }

    pub(super) fn clamp_tree_selection(&mut self, tree_len: usize) {
        self.selected_tree = if tree_len == 0 {
            0
        } else {
            self.selected_tree.min(tree_len - 1)
        };
    }

    pub(super) fn clamp_track_selection(&mut self, track_len: usize) {
        self.selected_track_row = if track_len == 0 {
            0
        } else {
            self.selected_track_row.min(track_len - 1)
        };
    }

    pub(super) fn artist_expanded(&self, artist: &str) -> bool {
        self.expanded_artists.contains(artist)
    }

    pub(super) fn expand_artist(&mut self, artist: String) {
        self.expanded_artists.insert(artist);
    }

    pub(super) fn collapse_artist(&mut self, artist: &str) -> bool {
        self.expanded_artists.remove(artist)
    }

    pub(super) fn compilations_expanded(&self) -> bool {
        self.compilations_expanded
    }

    pub(super) fn set_compilations_expanded(&mut self, expanded: bool) {
        self.compilations_expanded = expanded;
    }

    pub(super) fn toggle_compilations_expanded(&mut self) -> bool {
        self.compilations_expanded = !self.compilations_expanded;
        self.compilations_expanded
    }

    pub(super) fn playlists_expanded(&self) -> bool {
        self.playlists_expanded
    }

    pub(super) fn set_playlists_expanded(&mut self, expanded: bool) {
        self.playlists_expanded = expanded;
    }

    pub(super) fn toggle_playlists_expanded(&mut self) -> bool {
        self.playlists_expanded = !self.playlists_expanded;
        self.playlists_expanded
    }

    pub(super) fn expansion_state(&self) -> BrowserExpansionState {
        BrowserExpansionState {
            expanded_artists: self.expanded_artists.clone(),
            compilations_expanded: self.compilations_expanded,
            playlists_expanded: self.playlists_expanded,
        }
    }

    pub(super) fn restore_expansion_state(&mut self, state: BrowserExpansionState) {
        self.expanded_artists = state.expanded_artists;
        self.compilations_expanded = state.compilations_expanded;
        self.playlists_expanded = state.playlists_expanded;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TreeEntry {
    Playlists,
    Playlist { playlist_id: i64, name: String },
    Compilation,
    CompilationAlbum { album: String },
    Artist { artist: String },
    Album { artist: String, album: String },
}

impl TreeEntry {
    pub(super) fn artist(&self) -> &str {
        match self {
            Self::Playlists | Self::Playlist { .. } => "Playlists",
            Self::Compilation | Self::CompilationAlbum { .. } => "Compilations",
            Self::Artist { artist } | Self::Album { artist, .. } => artist,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum TrackRow {
    AlbumHeader {
        album: String,
        album_year: Option<i64>,
        duration_ms: i64,
    },
    DiscDivider {
        disc_number: Option<i64>,
    },
    Track {
        track_index: usize,
        show_disc_number: bool,
    },
    PlaylistHeader {
        name: String,
        duration_ms: i64,
    },
    PlaylistTrack {
        playlist_id: i64,
        playlist_track_id: i64,
        position: usize,
        track_index: usize,
    },
}

impl TrackRow {
    pub(super) fn track_index(&self) -> Option<usize> {
        match self {
            Self::Track { track_index, .. } | Self::PlaylistTrack { track_index, .. } => {
                Some(*track_index)
            }
            Self::AlbumHeader { .. } | Self::DiscDivider { .. } | Self::PlaylistHeader { .. } => {
                None
            }
        }
    }
}

impl App {
    pub(super) fn rebuild_search_cache(&mut self) {
        self.view.search_texts = self.tracks.iter().map(track_search_text).collect();
    }

    pub(super) fn rebuild_filtered_indices(&mut self) {
        self.view.filtered_indices.clear();
        let query = FilterQuery::parse(self.input.filter());
        if query.is_empty() {
            self.view.filtered_indices.extend(0..self.tracks.len());
            return;
        }

        self.view
            .filtered_indices
            .extend(
                self.view
                    .search_texts
                    .iter()
                    .enumerate()
                    .filter_map(|(index, haystack)| {
                        query
                            .matches(&self.tracks[index], haystack)
                            .then_some(index)
                    }),
            );
    }

    pub(super) fn rebuild_tree_entries(&mut self) {
        self.view.tree_entries.clear();
        if !self.playlists.is_empty() {
            self.view.tree_entries.push(TreeEntry::Playlists);
            if self.browser.playlists_expanded() {
                self.view
                    .tree_entries
                    .extend(self.playlists.iter().map(|playlist| TreeEntry::Playlist {
                        playlist_id: playlist.id,
                        name: playlist.name.clone(),
                    }));
            }
        }
        if self
            .view
            .filtered_indices
            .iter()
            .any(|index| self.tracks[*index].compilation)
        {
            self.view.tree_entries.push(TreeEntry::Compilation);
            if self.browser.compilations_expanded() {
                let mut seen_compilation_albums = HashSet::new();
                let mut compilation_indices: Vec<usize> = self
                    .view
                    .filtered_indices
                    .iter()
                    .copied()
                    .filter(|index| self.tracks[*index].compilation)
                    .collect();
                compilation_indices.sort_by(|left, right| {
                    compare_compilation_tracks(&self.tracks[*left], &self.tracks[*right])
                });
                for index in compilation_indices {
                    let track = &self.tracks[index];
                    let album = track.tree_album().to_string();
                    if seen_compilation_albums.insert(album.clone()) {
                        self.view
                            .tree_entries
                            .push(TreeEntry::CompilationAlbum { album });
                    }
                }
            }
        }

        let mut seen_artists = HashSet::new();
        let mut seen_albums = HashSet::new();
        for &index in &self.view.filtered_indices {
            let track = &self.tracks[index];
            let artist = track.tree_artist().to_string();
            if seen_artists.insert(artist.clone()) {
                self.view.tree_entries.push(TreeEntry::Artist {
                    artist: artist.clone(),
                });
            }
            if self.browser.artist_expanded(&artist) {
                let album = track.tree_album().to_string();
                if seen_albums.insert((artist.clone(), album.clone())) {
                    self.view
                        .tree_entries
                        .push(TreeEntry::Album { artist, album });
                }
            }
        }
    }

    pub(super) fn rebuild_track_rows(&mut self) {
        self.view.track_rows.clear();
        let Some(entry) = self.selected_tree_entry().cloned() else {
            return;
        };
        if matches!(entry, TreeEntry::Playlists) {
            self.rebuild_playlist_group_track_rows();
            return;
        }
        if let TreeEntry::Playlist { playlist_id, .. } = entry {
            for (position, playback) in self
                .playlist_playback_entries(playlist_id)
                .into_iter()
                .enumerate()
            {
                let Some(PlaybackSource::PlaylistTrack {
                    playlist_id,
                    playlist_track_id,
                }) = playback.source
                else {
                    continue;
                };
                self.view.track_rows.push(TrackRow::PlaylistTrack {
                    playlist_id,
                    playlist_track_id,
                    position: position + 1,
                    track_index: playback.track_index,
                });
            }
            return;
        }

        let mut album_durations = HashMap::new();
        let mut album_years = HashMap::new();
        let mut album_discs = HashMap::new();
        let track_indices = self.track_indices_for_entry(&entry);
        for &index in &track_indices {
            let track = &self.tracks[index];
            let album_key = track_album_key(track);
            *album_durations.entry(album_key.clone()).or_insert(0) +=
                track.duration_ms.unwrap_or(0);
            let album_year = album_years.entry(album_key.clone()).or_insert(None);
            if album_year.is_none() {
                *album_year = track.album_year;
            }
            if let Some(disc_number) = track.disc_number {
                album_discs
                    .entry(album_key)
                    .or_insert_with(HashSet::new)
                    .insert(disc_number);
            }
        }

        let mut current_album: Option<String> = None;
        let mut current_disc: Option<i64> = None;
        for index in track_indices {
            let track = &self.tracks[index];
            let album_key = track_album_key(track);
            let album = track.tree_album().to_string();
            if current_album.as_deref() != Some(album_key.as_str()) {
                current_album = Some(album_key.clone());
                current_disc = None;
                self.view.track_rows.push(TrackRow::AlbumHeader {
                    album_year: album_years.get(&album_key).copied().flatten(),
                    duration_ms: album_durations.get(&album_key).copied().unwrap_or_default(),
                    album,
                });
            }
            let show_disc_number = album_discs
                .get(&album_key)
                .map(|discs| discs.len() > 1)
                .unwrap_or(false);
            if show_disc_number && current_disc.is_some() && current_disc != track.disc_number {
                self.view.track_rows.push(TrackRow::DiscDivider {
                    disc_number: track.disc_number,
                });
            }
            current_disc = track.disc_number;
            self.view.track_rows.push(TrackRow::Track {
                track_index: index,
                show_disc_number,
            });
        }
    }

    fn rebuild_playlist_group_track_rows(&mut self) {
        for playlist in &self.playlists {
            let entries = self.playlist_playback_entries(playlist.id);
            if entries.is_empty() {
                continue;
            }

            let duration_ms = entries
                .iter()
                .filter_map(|entry| self.tracks.get(entry.track_index)?.duration_ms)
                .sum();
            self.view.track_rows.push(TrackRow::PlaylistHeader {
                name: playlist.name.clone(),
                duration_ms,
            });
            for (position, playback) in entries.into_iter().enumerate() {
                let Some(PlaybackSource::PlaylistTrack {
                    playlist_id,
                    playlist_track_id,
                }) = playback.source
                else {
                    continue;
                };
                self.view.track_rows.push(TrackRow::PlaylistTrack {
                    playlist_id,
                    playlist_track_id,
                    position: position + 1,
                    track_index: playback.track_index,
                });
            }
        }
    }

    pub(super) fn tree_entries(&self) -> &[TreeEntry] {
        &self.view.tree_entries
    }

    pub(super) fn track_rows(&self) -> &[TrackRow] {
        &self.view.track_rows
    }

    pub(super) fn track_index_for_media_item_id(&self, media_item_id: i64) -> Option<usize> {
        self.tracks
            .iter()
            .position(|track| track.media_item_id == media_item_id)
    }

    pub(super) fn track_row_media_item_id(&self, row: &TrackRow) -> Option<i64> {
        let index = row.track_index()?;
        self.tracks.get(index).map(|track| track.media_item_id)
    }

    pub(super) fn tree_entry_is_current(&self, entry: &TreeEntry) -> bool {
        let Some(current) = &self.current else {
            return false;
        };

        if let Some(PlaybackSource::PlaylistTrack { playlist_id, .. }) = current.source {
            return match entry {
                TreeEntry::Playlists => !self.browser.playlists_expanded(),
                TreeEntry::Playlist {
                    playlist_id: entry_playlist_id,
                    ..
                } => self.browser.playlists_expanded() && *entry_playlist_id == playlist_id,
                TreeEntry::Compilation
                | TreeEntry::CompilationAlbum { .. }
                | TreeEntry::Artist { .. }
                | TreeEntry::Album { .. } => false,
            };
        }

        let current_artist = current.track.tree_artist();
        let current_album = current.track.tree_album();
        match entry {
            TreeEntry::Playlists | TreeEntry::Playlist { .. } => false,
            TreeEntry::Compilation => {
                current.track.compilation && !self.browser.compilations_expanded()
            }
            TreeEntry::CompilationAlbum { album } => {
                current.track.compilation
                    && current_album == album
                    && self.browser.compilations_expanded()
            }
            TreeEntry::Artist { artist } => {
                artist == current_artist && !self.browser.artist_expanded(artist)
            }
            TreeEntry::Album { artist, album } => {
                artist == current_artist
                    && album == current_album
                    && self.browser.artist_expanded(artist)
            }
        }
    }

    pub(super) fn selected_tree_entry(&self) -> Option<&TreeEntry> {
        self.view.tree_entries.get(self.browser.selected_tree())
    }

    pub(super) fn selected_scope_tracks(&self) -> Vec<(usize, &LibraryTrack)> {
        let Some(entry) = self.selected_tree_entry() else {
            return Vec::new();
        };
        self.track_indices_for_entry(entry)
            .into_iter()
            .map(|index| (index, &self.tracks[index]))
            .collect()
    }

    pub(super) fn track_indices_for_entry(&self, entry: &TreeEntry) -> Vec<usize> {
        if matches!(entry, TreeEntry::Playlists) {
            let filtered: HashSet<usize> = self.view.filtered_indices.iter().copied().collect();
            let mut seen = HashSet::new();
            return self
                .playlists
                .iter()
                .flat_map(|playlist| self.playlist_cache.track_indices(playlist.id))
                .filter(|index| filtered.contains(index) && seen.insert(*index))
                .collect();
        }
        if let TreeEntry::Playlist { playlist_id, .. } = entry {
            let filtered: HashSet<usize> = self.view.filtered_indices.iter().copied().collect();
            return self
                .playlist_cache
                .track_indices(*playlist_id)
                .filter(|index| filtered.contains(index))
                .collect();
        }

        let mut indices: Vec<usize> = self
            .view
            .filtered_indices
            .iter()
            .copied()
            .filter(|index| tree_entry_matches_track(entry, &self.tracks[*index]))
            .collect();
        if matches!(
            entry,
            TreeEntry::Compilation | TreeEntry::CompilationAlbum { .. }
        ) {
            indices.sort_by(|left, right| {
                compare_compilation_tracks(&self.tracks[*left], &self.tracks[*right])
            });
        }
        indices
    }

    fn track_in_any_playlist(&self, track_index: usize) -> bool {
        self.playlist_cache
            .contains_track_in_any_playlist(track_index)
    }

    fn track_in_playlist(&self, track_index: usize, playlist_id: i64) -> bool {
        self.playlist_cache.contains_track(playlist_id, track_index)
    }

    pub(super) fn nearest_track_row(&self, from: usize) -> Option<usize> {
        let rows = self.track_rows();
        if rows.get(from).and_then(TrackRow::track_index).is_some() {
            return Some(from);
        }

        rows.iter()
            .enumerate()
            .skip(from)
            .find_map(|(row, entry)| entry.track_index().is_some().then_some(row))
            .or_else(|| {
                rows.iter()
                    .enumerate()
                    .take(from)
                    .rev()
                    .find_map(|(row, entry)| entry.track_index().is_some().then_some(row))
            })
    }

    pub(super) fn next_track_row(&self, direction: i32) -> Option<usize> {
        let rows = self.track_rows();
        if rows.is_empty() {
            return None;
        }

        let current = self.browser.selected_track_row().min(rows.len() - 1);
        if direction >= 0 {
            rows.iter()
                .enumerate()
                .skip(current + 1)
                .find_map(|(row, entry)| entry.track_index().is_some().then_some(row))
                .or_else(|| {
                    rows.get(current)
                        .and_then(TrackRow::track_index)
                        .is_some()
                        .then_some(current)
                })
        } else {
            rows.iter()
                .enumerate()
                .take(current)
                .rev()
                .find_map(|(row, entry)| entry.track_index().is_some().then_some(row))
                .or_else(|| {
                    rows.get(current)
                        .and_then(TrackRow::track_index)
                        .is_some()
                        .then_some(current)
                })
        }
    }

    pub(super) fn select_current_track(&mut self) {
        let index = self
            .current
            .as_ref()
            .and_then(|current| self.track_index_for_media_item_id(current.track.media_item_id));
        if let Some(index) = index {
            self.select_track_index(index);
            self.message = String::from("selected current track");
        } else {
            self.message = String::from("nothing playing");
        }
    }

    pub(super) fn select_current_track_for_restore(&mut self) {
        let Some(index) = self
            .current
            .as_ref()
            .and_then(|current| self.track_index_for_media_item_id(current.track.media_item_id))
        else {
            return;
        };
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let artist = track.tree_artist().to_string();
        if let Some(position) = self.tree_entries().iter().position(|entry| {
            matches!(entry, TreeEntry::Artist { artist: entry_artist } if entry_artist == &artist)
        }) {
            self.browser.select_tree(position);
            self.rebuild_track_rows();
        } else {
            self.select_track_index(index);
        }

        if let Some(position) = self
            .track_rows()
            .iter()
            .position(|row| row.track_index() == Some(index))
        {
            self.browser.select_track_row(position);
        }
        self.focus = super::FocusPane::Tracks;
        self.apply_selection_state();
    }

    pub(super) fn select_track_index(&mut self, index: usize) {
        if let Some(track) = self.tracks.get(index) {
            let artist = track.tree_artist().to_string();
            let album = track.tree_album().to_string();
            let is_compilation = track.compilation;
            let current_entry_matches = self
                .selected_tree_entry()
                .map(|entry| match entry {
                    TreeEntry::Playlists => self.track_in_any_playlist(index),
                    TreeEntry::Playlist { playlist_id, .. } => {
                        self.track_in_playlist(index, *playlist_id)
                    }
                    TreeEntry::Compilation => is_compilation,
                    TreeEntry::CompilationAlbum { album: entry_album } => {
                        is_compilation && entry_album == &album
                    }
                    TreeEntry::Artist {
                        artist: entry_artist,
                    } => entry_artist == &artist,
                    TreeEntry::Album {
                        artist: entry_artist,
                        album: entry_album,
                    } => entry_artist == &artist && entry_album == &album,
                })
                .unwrap_or(false);

            if !current_entry_matches {
                let mut tree_changed = false;
                if is_compilation {
                    self.browser.set_compilations_expanded(true);
                    self.sync_selection();
                }
                if is_compilation {
                    if let Some(position) = self.tree_entries().iter().position(|entry| {
                        matches!(
                            entry,
                            TreeEntry::CompilationAlbum {
                                album: entry_album,
                            } if entry_album == &album
                        )
                    }) {
                        self.browser.select_tree(position);
                        tree_changed = true;
                    }
                } else if let Some(position) = self.tree_entries().iter().position(|entry| {
                    matches!(
                        entry,
                        TreeEntry::Album {
                            artist: entry_artist,
                            album: entry_album,
                        } if entry_artist == &artist
                            && entry_album == &album
                    )
                }) {
                    self.browser.select_tree(position);
                    tree_changed = true;
                } else if let Some(position) = self.tree_entries().iter().position(|entry| {
                    matches!(
                        entry,
                        TreeEntry::Artist {
                            artist: entry_artist
                        } if entry_artist == &artist
                    )
                }) {
                    self.browser.select_tree(position);
                    tree_changed = true;
                }
                if tree_changed {
                    self.sync_selection();
                }
            }
        }

        if let Some(position) = self
            .track_rows()
            .iter()
            .position(|row| row.track_index() == Some(index))
        {
            self.browser.select_track_row(position);
        }
        self.apply_selection_state();
    }
}

fn tree_entry_matches_track(entry: &TreeEntry, track: &LibraryTrack) -> bool {
    match entry {
        TreeEntry::Playlists | TreeEntry::Playlist { .. } => false,
        TreeEntry::Compilation => track.compilation,
        TreeEntry::CompilationAlbum { album } => track.compilation && track.tree_album() == album,
        TreeEntry::Artist { artist } => track.tree_artist() == artist,
        TreeEntry::Album { artist, album } => {
            track.tree_artist() == artist && track.tree_album() == album
        }
    }
}

fn track_album_key(track: &LibraryTrack) -> String {
    track.tree_album().to_string()
}

fn compare_compilation_tracks(left: &LibraryTrack, right: &LibraryTrack) -> Ordering {
    compare_optional_i64(left.album_year, right.album_year)
        .then_with(|| compare_text(left.tree_album(), right.tree_album()))
        .then_with(|| compare_optional_i64(left.disc_number, right.disc_number))
        .then_with(|| compare_optional_i64(left.track_number, right.track_number))
        .then_with(|| compare_text(left.display_title(), right.display_title()))
        .then_with(|| left.path.cmp(&right.path))
}

fn compare_text(left: &str, right: &str) -> Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn compare_optional_i64(left: Option<i64>, right: Option<i64>) -> Ordering {
    left.unwrap_or(i64::MAX).cmp(&right.unwrap_or(i64::MAX))
}

pub(super) fn track_root_label(track: &LibraryTrack) -> Option<String> {
    track.library_root.as_deref().map(root_label)
}

fn root_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_movement_clamps_and_resets_track_selection() {
        let mut browser = BrowserState::default();
        browser.select_tree(2);
        browser.select_track_row(7);

        browser.move_tree_selection(1, usize::MAX, 5);

        assert_eq!(browser.selected_tree(), 4);
        assert_eq!(browser.selected_track_row(), 0);

        browser.move_tree_selection(-1, usize::MAX, 5);

        assert_eq!(browser.selected_tree(), 0);
    }

    #[test]
    fn expansion_snapshot_restores_tree_policy_without_cursor_state() {
        let mut browser = BrowserState::default();
        browser.expand_artist(String::from("Artist"));
        browser.set_compilations_expanded(true);
        let snapshot = browser.expansion_state();

        browser.select_tree(4);
        browser.select_track_row(8);
        browser.collapse_artist("Artist");
        browser.set_compilations_expanded(false);
        browser.set_playlists_expanded(true);
        browser.restore_expansion_state(snapshot);

        assert_eq!(browser.selected_tree(), 4);
        assert_eq!(browser.selected_track_row(), 8);
        assert!(browser.artist_expanded("Artist"));
        assert!(browser.compilations_expanded());
        assert!(!browser.playlists_expanded());
    }
}
