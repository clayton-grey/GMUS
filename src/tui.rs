use std::collections::HashMap;

use anyhow::Result;
use ratatui::widgets::ListState;
use rusqlite::Connection;

use crate::config::AppPaths;
use crate::db::{self, LibraryTrack};
use crate::player::{self, PlayerBackend};

mod browser;
mod command;
mod control;
mod filter;
mod formatting;
mod input;
mod jobs;
mod keymap;
mod layout;
mod lines;
mod management;
mod media_sync;
mod mouse;
mod persistence;
mod playback;
mod playlist;
mod renderer;
mod runtime;
mod selection;
mod status;
#[cfg(test)]
mod tests;

pub use runtime::run;

use browser::{BrowserState, TrackRow, TreeEntry};
use command::CommandOutputState;
use input::{InputKind, InputState};
use jobs::LibraryJobRunner;
use keymap::{KeyAction, KeySpec};
use layout::LayoutState;
use management::ManagementPanelState;
use media_sync::IntegrationState;
#[cfg(test)]
use playback::PlayTarget;
use playback::{PlaybackModeState, PlaybackSource, PlayingTrack};
use playlist::{PlaylistCache, PlaylistPanelEntry};
use status::TransientStatus;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum FocusPane {
    Tree,
    Tracks,
    Playlist,
    Keymap,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CommandOutputKind {
    Text,
    LibraryRoots,
}

#[derive(Debug, Default)]
struct ViewCache {
    search_texts: Vec<String>,
    filtered_indices: Vec<usize>,
    tree_entries: Vec<TreeEntry>,
    track_rows: Vec<TrackRow>,
    playlist_entries: Vec<PlaylistPanelEntry>,
}

struct App {
    paths: AppPaths,
    tracks: Vec<LibraryTrack>,
    playlists: Vec<db::Playlist>,
    playlist_cache: PlaylistCache,
    view: ViewCache,
    tree_state: ListState,
    track_state: ListState,
    playlist_state: ListState,
    keymap_state: ListState,
    browser: BrowserState,
    management_panel: ManagementPanelState,
    focus: FocusPane,
    restore_filter: bool,
    restore_track: bool,
    input: InputState,
    command_output: CommandOutputState,
    key_bindings: HashMap<KeyAction, Vec<KeySpec>>,
    library_job: Option<LibraryJobRunner>,
    layout: LayoutState,
    playback_mode: PlaybackModeState,
    player: Box<dyn PlayerBackend>,
    integration: IntegrationState,
    current: Option<PlayingTrack>,
    suspended_position_ms: Option<i64>,
    explicit_seek_to_end: bool,
    transient_status: Option<TransientStatus>,
    message: String,
}

impl App {
    fn new(conn: &Connection, paths: &AppPaths) -> Result<Self> {
        Self::new_with_player(conn, paths, player::default_player_backend()?)
    }

    fn new_with_player(
        conn: &Connection,
        paths: &AppPaths,
        player: Box<dyn PlayerBackend>,
    ) -> Result<Self> {
        let pane_layout = db::pane_layout(conn)?;
        let mut app = Self {
            paths: paths.clone(),
            tracks: db::library_tracks(conn)?,
            playlists: db::playlists(conn)?,
            playlist_cache: PlaylistCache::default(),
            view: ViewCache::default(),
            tree_state: ListState::default(),
            track_state: ListState::default(),
            playlist_state: ListState::default(),
            keymap_state: ListState::default(),
            browser: BrowserState::default(),
            management_panel: ManagementPanelState::default(),
            focus: FocusPane::Tree,
            restore_filter: db::restore_filter_enabled(conn)?,
            restore_track: db::restore_track_enabled(conn)?,
            input: InputState::default(),
            command_output: CommandOutputState::default(),
            key_bindings: HashMap::new(),
            library_job: None,
            layout: LayoutState::new(
                pane_layout.library_percent_offset,
                pane_layout.info_height_offset,
                db::column_layout_width(conn, layout::DEFAULT_COLUMN_LAYOUT_WIDTH)?,
                true,
            ),
            playback_mode: PlaybackModeState::default(),
            player,
            integration: IntegrationState::default(),
            current: None,
            suspended_position_ms: None,
            explicit_seek_to_end: false,
            transient_status: None,
            message: String::from(
                "Tab pane  Enter select/play  k keymap  x play  c play/pause  p playlists  v stop",
            ),
        };
        app.load_key_bindings(conn)?;
        app.refresh_playlist_tracks(conn)?;
        app.rebuild_search_cache();
        if app.restore_filter {
            if let Some(filter) = db::saved_filter(conn)? {
                app.input.set_filter(filter);
            }
        }
        if app.restore_track {
            match db::browser_selection(conn)? {
                Some(selection) => {
                    if app.restore_saved_browser_selection(&selection) {
                        app.focus = FocusPane::Tracks;
                        app.apply_selection_state();
                    }
                }
                None => app.sync_selection(),
            }
        } else {
            app.sync_selection();
        }
        Ok(app)
    }

    fn refresh(&mut self, conn: &Connection) -> Result<()> {
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_media_item_id = self.selected_playable_media_item_id();
        self.tracks = db::library_tracks(conn)?;
        self.playlists = db::playlists(conn)?;
        self.refresh_playlist_tracks(conn)?;
        self.rebuild_search_cache();
        self.reset_shuffle_order();
        self.sync_current_track_index(conn)?;
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        self.message = format!("loaded {} tracks", self.tracks.len());
        Ok(())
    }
}
