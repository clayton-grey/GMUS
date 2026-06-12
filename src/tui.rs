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
mod keymap;
mod layout;
mod lines;
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

use browser::{TrackRow, TreeEntry};
use jobs::LibraryJobRunner;
use keymap::{KeyAction, KeySpec};
use playback::{PlayTarget, PlaybackEntry, PlaybackSource, PlayingTrack};
use playlist::PlaylistPanelEntry;
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
    playlist_track_ids: HashMap<i64, Vec<i64>>,
    playlist_track_entry_ids: HashMap<i64, Vec<i64>>,
    playlist_track_indices: HashMap<i64, Vec<usize>>,
    view: ViewCache,
    tree_state: ListState,
    track_state: ListState,
    playlist_state: ListState,
    keymap_state: ListState,
    selected_tree: usize,
    selected_track_row: usize,
    selected_playlist_row: usize,
    selected_keymap_row: usize,
    expanded_artists: HashSet<String>,
    compilations_expanded: bool,
    playlists_expanded: bool,
    expanded_playlists: HashSet<i64>,
    active_playlist_id: Option<i64>,
    playlist_panel_open: bool,
    keymap_panel_open: bool,
    focus: FocusPane,
    filter: String,
    restore_filter: bool,
    restore_track: bool,
    filter_mode: bool,
    rate_input: String,
    rate_mode: bool,
    command: String,
    command_mode: bool,
    command_output: Vec<String>,
    command_output_kind: CommandOutputKind,
    command_roots: Vec<db::LibraryRoot>,
    command_selected: usize,
    command_focus: bool,
    key_bindings: HashMap<KeyAction, Vec<KeySpec>>,
    keymap_capture_action: Option<KeyAction>,
    library_job: Option<LibraryJobRunner>,
    info_panel_visible: bool,
    startup_info_visible: bool,
    library_pane_percent_offset: i16,
    info_pane_height_offset: i16,
    column_layout_width: u16,
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
            playlist_track_ids: HashMap::new(),
            playlist_track_entry_ids: HashMap::new(),
            playlist_track_indices: HashMap::new(),
            view: ViewCache::default(),
            tree_state: ListState::default(),
            track_state: ListState::default(),
            playlist_state: ListState::default(),
            keymap_state: ListState::default(),
            selected_tree: 0,
            selected_track_row: 0,
            selected_playlist_row: 0,
            selected_keymap_row: 0,
            expanded_artists: HashSet::new(),
            compilations_expanded: false,
            playlists_expanded: false,
            expanded_playlists: HashSet::new(),
            active_playlist_id: None,
            playlist_panel_open: false,
            keymap_panel_open: false,
            focus: FocusPane::Tree,
            filter: String::new(),
            restore_filter: db::restore_filter_enabled(conn)?,
            restore_track: db::restore_track_enabled(conn)?,
            filter_mode: false,
            rate_input: String::new(),
            rate_mode: false,
            command: String::new(),
            command_mode: false,
            command_output: Vec::new(),
            command_output_kind: CommandOutputKind::Text,
            command_roots: Vec::new(),
            command_selected: 0,
            command_focus: false,
            key_bindings: HashMap::new(),
            keymap_capture_action: None,
            library_job: None,
            info_panel_visible: true,
            startup_info_visible: true,
            library_pane_percent_offset: pane_layout.library_percent_offset,
            info_pane_height_offset: pane_layout.info_height_offset,
            column_layout_width: db::column_layout_width(
                conn,
                layout::DEFAULT_COLUMN_LAYOUT_WIDTH,
            )?,
            play_target: PlayTarget::Library,
            continuous: true,
            repeat: false,
            shuffle: false,
            shuffle_seed: 0x476d_7573_2026_0528,
            shuffle_scope: Vec::new(),
            shuffle_order: Vec::new(),
            player,
            integration: integration::default_integration(),
            current: None,
            suspended_position_ms: None,
            last_integration_state: None,
            last_integration_position_s: None,
            integration_error_reported: false,
            track_notifications_visible: true,
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
                app.filter = filter;
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
        self.sync_current_track_index();
        self.sync_selection_preserving_browser_anchors(
            selected_tree_entry.as_ref(),
            selected_media_item_id,
        );
        self.message = format!("loaded {} tracks", self.tracks.len());
        Ok(())
    }
}
