use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, LibraryTrack, SavedBrowserSelection};

use super::browser::BrowserExpansionState;
use super::{App, TreeEntry};

const TREE_KIND_PLAYLISTS: &str = "playlists";
const TREE_KIND_PLAYLIST: &str = "playlist";
const TREE_KIND_COMPILATION: &str = "compilation";
const TREE_KIND_COMPILATION_ALBUM: &str = "compilation_album";
const TREE_KIND_ARTIST: &str = "artist";
const TREE_KIND_ALBUM: &str = "album";

impl App {
    pub(super) fn restore_saved_browser_selection(
        &mut self,
        selection: &SavedBrowserSelection,
    ) -> bool {
        let previous_expansion_state = self.browser.expansion_state();

        self.expand_for_saved_tree(selection);
        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();

        let Some(tree_position) = self.saved_tree_position(selection) else {
            self.reset_browser_selection(previous_expansion_state);
            return false;
        };

        self.browser.select_tree(tree_position);
        self.rebuild_track_rows();

        if let Some(media_item_id) = selection.media_item_id {
            let Some(track_position) = self
                .track_rows()
                .iter()
                .position(|row| self.track_row_media_item_id(row) == Some(media_item_id))
            else {
                self.reset_browser_selection(previous_expansion_state);
                return false;
            };
            self.browser.select_track_row(track_position);
        } else {
            self.browser.reset_track_selection();
            self.clamp_track_selection();
        }

        self.rebuild_playlist_entries();
        self.clamp_playlist_selection();
        self.apply_selection_state();
        true
    }

    #[cfg(test)]
    pub(super) fn save_browser_selection(&self, conn: &Connection) -> Result<()> {
        let Some(tree_entry) = self.selected_tree_entry() else {
            return Ok(());
        };
        let selection =
            saved_selection_from_tree_entry(tree_entry, self.selected_playable_media_item_id());
        db::save_browser_selection(conn, &selection)
    }

    pub(super) fn save_current_track_selection(&self, conn: &Connection) -> Result<()> {
        let Some(current) = &self.current else {
            return Ok(());
        };
        let selection = saved_selection_from_track(&current.track);
        db::save_browser_selection(conn, &selection)
    }

    fn expand_for_saved_tree(&mut self, selection: &SavedBrowserSelection) {
        match selection.tree_kind.as_str() {
            TREE_KIND_PLAYLIST => self.browser.set_playlists_expanded(true),
            TREE_KIND_COMPILATION_ALBUM => self.browser.set_compilations_expanded(true),
            TREE_KIND_ALBUM => {
                if let Some(artist) = selection.artist.as_ref() {
                    self.browser.expand_artist(artist.clone());
                }
            }
            TREE_KIND_PLAYLISTS | TREE_KIND_COMPILATION | TREE_KIND_ARTIST => {}
            _ => {}
        }
    }

    fn saved_tree_position(&self, selection: &SavedBrowserSelection) -> Option<usize> {
        self.tree_entries()
            .iter()
            .position(|entry| saved_tree_matches_entry(selection, entry))
    }

    fn reset_browser_selection(&mut self, expansion_state: BrowserExpansionState) {
        self.browser.restore_expansion_state(expansion_state);
        self.browser.reset_selection();
        self.sync_selection();
    }
}

#[cfg(test)]
fn saved_selection_from_tree_entry(
    entry: &TreeEntry,
    media_item_id: Option<i64>,
) -> SavedBrowserSelection {
    let mut selection = SavedBrowserSelection {
        tree_kind: String::new(),
        artist: None,
        album: None,
        playlist_id: None,
        media_item_id,
    };

    match entry {
        TreeEntry::Playlists => {
            selection.tree_kind = TREE_KIND_PLAYLISTS.to_string();
        }
        TreeEntry::Playlist { playlist_id, .. } => {
            selection.tree_kind = TREE_KIND_PLAYLIST.to_string();
            selection.playlist_id = Some(*playlist_id);
        }
        TreeEntry::Compilation => {
            selection.tree_kind = TREE_KIND_COMPILATION.to_string();
        }
        TreeEntry::CompilationAlbum { album } => {
            selection.tree_kind = TREE_KIND_COMPILATION_ALBUM.to_string();
            selection.album = Some(album.clone());
        }
        TreeEntry::Artist { artist } => {
            selection.tree_kind = TREE_KIND_ARTIST.to_string();
            selection.artist = Some(artist.clone());
        }
        TreeEntry::Album { artist, album } => {
            selection.tree_kind = TREE_KIND_ALBUM.to_string();
            selection.artist = Some(artist.clone());
            selection.album = Some(album.clone());
        }
    }

    selection
}

fn saved_selection_from_track(track: &LibraryTrack) -> SavedBrowserSelection {
    SavedBrowserSelection {
        tree_kind: TREE_KIND_ARTIST.to_string(),
        artist: Some(track.tree_artist().to_string()),
        album: None,
        playlist_id: None,
        media_item_id: Some(track.media_item_id),
    }
}

fn saved_tree_matches_entry(selection: &SavedBrowserSelection, entry: &TreeEntry) -> bool {
    match (selection.tree_kind.as_str(), entry) {
        (TREE_KIND_PLAYLISTS, TreeEntry::Playlists) => true,
        (TREE_KIND_PLAYLIST, TreeEntry::Playlist { playlist_id, .. }) => {
            selection.playlist_id == Some(*playlist_id)
        }
        (TREE_KIND_COMPILATION, TreeEntry::Compilation) => true,
        (TREE_KIND_COMPILATION_ALBUM, TreeEntry::CompilationAlbum { album }) => {
            selection.album.as_deref() == Some(album.as_str())
        }
        (TREE_KIND_ARTIST, TreeEntry::Artist { artist }) => {
            selection.artist.as_deref() == Some(artist.as_str())
        }
        (TREE_KIND_ALBUM, TreeEntry::Album { artist, album }) => {
            selection.artist.as_deref() == Some(artist.as_str())
                && selection.album.as_deref() == Some(album.as_str())
        }
        _ => false,
    }
}
