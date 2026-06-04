use std::collections::{HashMap, HashSet};

use anyhow::Result;
use ratatui::widgets::ListState;
use rusqlite::Connection;

use crate::config::AppPaths;
use crate::db::{self, LibraryTrack};
use crate::integration::{self, Integration};
use crate::player::{self, PlaybackState, PlayerBackend};

mod browser;
mod command;
mod control;
mod filter;
mod formatting;
mod jobs;
mod layout;
mod lines;
mod media_sync;
mod mouse;
mod playback;
mod playlist;
mod renderer;
mod runtime;
mod selection;
mod status;
#[cfg(test)]
mod tests;

pub use runtime::run;

use browser::{TrackRow, TreeEntry};
use jobs::LibraryJobRunner;
use playback::{PlayTarget, PlaybackEntry, PlaybackSource, PlayingTrack};
use playlist::PlaylistPanelEntry;
use status::TransientStatus;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum FocusPane {
    Tree,
    Tracks,
    Playlist,
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
    playlist_track_ids: HashMap<i64, Vec<i64>>,
    playlist_track_entry_ids: HashMap<i64, Vec<i64>>,
    playlist_track_indices: HashMap<i64, Vec<usize>>,
    view: ViewCache,
    tree_state: ListState,
    track_state: ListState,
    playlist_state: ListState,
    selected_tree: usize,
    selected_track_row: usize,
    selected_playlist_row: usize,
    expanded_artists: HashSet<String>,
    compilations_expanded: bool,
    playlists_expanded: bool,
    expanded_playlists: HashSet<i64>,
    active_playlist_id: Option<i64>,
    playlist_panel_open: bool,
    focus: FocusPane,
    filter: String,
    filter_mode: bool,
    command: String,
    command_mode: bool,
    command_output: Vec<String>,
    command_output_kind: CommandOutputKind,
    command_roots: Vec<db::LibraryRoot>,
    command_selected: usize,
    command_focus: bool,
    library_job: Option<LibraryJobRunner>,
    info_panel_visible: bool,
    play_target: PlayTarget,
    continuous: bool,
    repeat: bool,
    shuffle: bool,
    shuffle_seed: u64,
    shuffle_scope: Vec<PlaybackEntry>,
    shuffle_order: Vec<PlaybackEntry>,
    player: Box<dyn PlayerBackend>,
    integration: Box<dyn Integration>,
    current: Option<PlayingTrack>,
    suspended_position_ms: Option<i64>,
    last_integration_state: Option<PlaybackState>,
    last_integration_position_s: Option<i64>,
    integration_error_reported: bool,
    #[cfg_attr(
        not(all(target_os = "macos", feature = "macos-media-session")),
        allow(dead_code)
    )]
    track_notifications_visible: bool,
    transient_status: Option<TransientStatus>,
    message: String,
}

impl App {
    fn new(conn: &Connection, paths: &AppPaths) -> Result<Self> {
        let mut app = Self {
            paths: paths.clone(),
            tracks: db::library_tracks(conn)?,
            playlists: db::playlists(conn)?,
            playlist_track_ids: HashMap::new(),
            playlist_track_entry_ids: HashMap::new(),
            playlist_track_indices: HashMap::new(),
            view: ViewCache::default(),
            tree_state: ListState::default(),
            track_state: ListState::default(),
            playlist_state: ListState::default(),
            selected_tree: 0,
            selected_track_row: 0,
            selected_playlist_row: 0,
            expanded_artists: HashSet::new(),
            compilations_expanded: false,
            playlists_expanded: false,
            expanded_playlists: HashSet::new(),
            active_playlist_id: None,
            playlist_panel_open: false,
            focus: FocusPane::Tree,
            filter: String::new(),
            filter_mode: false,
            command: String::new(),
            command_mode: false,
            command_output: Vec::new(),
            command_output_kind: CommandOutputKind::Text,
            command_roots: Vec::new(),
            command_selected: 0,
            command_focus: false,
            library_job: None,
            info_panel_visible: true,
            play_target: PlayTarget::Library,
            continuous: true,
            repeat: false,
            shuffle: false,
            shuffle_seed: 0x476d_7573_2026_0528,
            shuffle_scope: Vec::new(),
            shuffle_order: Vec::new(),
            player: player::default_player_backend()?,
            integration: integration::default_integration(),
            current: None,
            suspended_position_ms: None,
            last_integration_state: None,
            last_integration_position_s: None,
            integration_error_reported: false,
            track_notifications_visible: true,
            transient_status: None,
            message: String::from(
                "Tab pane  Enter select/play  x play  c play/pause  p playlists  v stop  b/z next/prev",
            ),
        };
        app.refresh_playlist_tracks(conn)?;
        app.rebuild_search_cache();
        app.sync_selection();
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
        self.sync_current_track_index();
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        self.message = format!("loaded {} tracks", self.tracks.len());
        Ok(())
    }
}
