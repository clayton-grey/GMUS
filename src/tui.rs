use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};
use rusqlite::Connection;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::AppPaths;
use crate::db::{self, LibraryTrack};
use crate::media_session::{self, MediaCommand, MediaSession, NowPlaying};
use crate::player::{self, PlaybackState, PlayerBackend};
use crate::scanner;

const SCRUB_SECONDS: i64 = 5;
const MAX_LISTENED_DELTA_MS: i64 = 10_000;
const ACTIVE_TICK: Duration = Duration::from_millis(1_000);
const MEDIA_IDLE_TICK: Duration = Duration::from_millis(1_000);
const STOPPED_TICK: Duration = Duration::from_secs(60);
const LIST_SCROLL_PADDING: usize = 3;
const MOUSE_SCROLL_LINES: usize = 1;
const STACKED_PANE_WIDTH: u16 = 75;
const WIDE_TREE_PERCENT: u16 = 33;
const NARROW_TREE_PERCENT: u16 = 34;
const INFO_PANEL_HEIGHT: u16 = 12;
const TRACKS_MIN_HEIGHT: u16 = 4;
const BOTTOM_STATUS_ROWS: u16 = 2;
const TRANSIENT_STATUS_DURATION: Duration = Duration::from_secs(1);
const COMMAND_OUTPUT_MAX_ROWS: u16 = 8;
const COMMAND_NAMES: &[&str] = &[
    "add",
    "remove",
    "update",
    "library",
    "filter",
    "clear",
    "clear-output",
];

pub fn run(conn: &Connection, paths: &AppPaths) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = App::new(conn, paths)?;
    let result = run_loop(&mut terminal, conn, paths, &mut app);
    let restore_result = restore_terminal(&mut terminal);
    result.and(restore_result)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum FocusPane {
    Tree,
    Tracks,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CommandOutputKind {
    Text,
    LibraryRoots,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PlayTarget {
    Library,
    Artist,
    Album,
}

impl PlayTarget {
    fn next(self) -> Self {
        match self {
            Self::Library => Self::Artist,
            Self::Artist => Self::Album,
            Self::Album => Self::Library,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Artist => "artist from library",
            Self::Album => "album from library",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeEntry {
    Compilation,
    CompilationAlbum { album: String },
    Artist { artist: String },
    Album { artist: String, album: String },
}

impl TreeEntry {
    fn artist(&self) -> &str {
        match self {
            Self::Compilation | Self::CompilationAlbum { .. } => "Compilations",
            Self::Artist { artist } | Self::Album { artist, .. } => artist,
        }
    }
}

#[derive(Debug, Clone)]
enum TrackRow {
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
}

#[derive(Debug, Default)]
struct ViewCache {
    search_texts: Vec<String>,
    filtered_indices: Vec<usize>,
    tree_entries: Vec<TreeEntry>,
    track_rows: Vec<TrackRow>,
}

struct App {
    paths: AppPaths,
    tracks: Vec<LibraryTrack>,
    view: ViewCache,
    tree_state: ListState,
    track_state: ListState,
    selected_tree: usize,
    selected_track_row: usize,
    expanded_artists: HashSet<String>,
    compilations_expanded: bool,
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
    pending_command: Option<String>,
    info_panel_visible: bool,
    play_target: PlayTarget,
    continuous: bool,
    repeat: bool,
    shuffle: bool,
    shuffle_seed: u64,
    shuffle_scope: Vec<usize>,
    shuffle_order: Vec<usize>,
    player: Box<dyn PlayerBackend>,
    media_session: Box<dyn MediaSession>,
    current: Option<PlayingTrack>,
    suspended_position_ms: Option<i64>,
    last_media_state: Option<PlaybackState>,
    last_media_position_s: Option<i64>,
    transient_status: Option<TransientStatus>,
    message: String,
}

struct TransientStatus {
    text: String,
    until: Instant,
}

#[derive(Debug, Clone)]
struct PlayingTrack {
    index: usize,
    track: LibraryTrack,
    last_position_ms: i64,
    listened_ms: i64,
}

impl PlayingTrack {
    fn tick_position(&mut self, position: Duration, state: PlaybackState) {
        let position_ms = position.as_millis() as i64;
        if state == PlaybackState::Playing {
            let delta = position_ms - self.last_position_ms;
            if delta > 0 && delta <= MAX_LISTENED_DELTA_MS {
                self.listened_ms += delta;
            }
        }
        self.align_position(position_ms);
    }

    fn align_position(&mut self, position_ms: i64) {
        self.last_position_ms = position_ms.max(0);
    }
}

impl App {
    fn new(conn: &Connection, paths: &AppPaths) -> Result<Self> {
        let mut app = Self {
            paths: paths.clone(),
            tracks: db::library_tracks(conn)?,
            view: ViewCache::default(),
            tree_state: ListState::default(),
            track_state: ListState::default(),
            selected_tree: 0,
            selected_track_row: 0,
            expanded_artists: HashSet::new(),
            compilations_expanded: false,
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
            pending_command: None,
            info_panel_visible: true,
            play_target: PlayTarget::Library,
            continuous: true,
            repeat: false,
            shuffle: false,
            shuffle_seed: 0x476d_7573_2026_0528,
            shuffle_scope: Vec::new(),
            shuffle_order: Vec::new(),
            player: player::default_player_backend()?,
            media_session: media_session::default_media_session(),
            current: None,
            suspended_position_ms: None,
            last_media_state: None,
            last_media_position_s: None,
            transient_status: None,
            message: String::from(
                "Tab pane  Enter select/play  x play  p pause  v stop  b/z next/prev",
            ),
        };
        app.rebuild_search_cache();
        app.sync_selection();
        Ok(app)
    }

    fn refresh(&mut self, conn: &Connection) -> Result<()> {
        self.tracks = db::library_tracks(conn)?;
        self.rebuild_search_cache();
        self.reset_shuffle_order();
        self.sync_selection();
        self.message = format!("loaded {} tracks", self.tracks.len());
        Ok(())
    }

    fn sync_selection(&mut self) {
        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();
        self.clamp_tree_selection();
        self.rebuild_track_rows();
        self.clamp_track_selection();
        self.apply_selection_state();
    }

    fn apply_selection_state(&mut self) {
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
    }

    fn clamp_tree_selection(&mut self) {
        let tree_len = self.view.tree_entries.len();
        self.selected_tree = if tree_len == 0 {
            0
        } else {
            self.selected_tree.min(tree_len - 1)
        };
    }

    fn clamp_track_selection(&mut self) {
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

    fn move_down(&mut self) {
        self.move_pane_selection(self.focus, 1, 1);
    }

    fn move_up(&mut self) {
        self.move_pane_selection(self.focus, -1, 1);
    }

    fn page_down(&mut self) {
        self.move_pane_selection(self.focus, 1, 10);
    }

    fn page_up(&mut self) {
        self.move_pane_selection(self.focus, -1, 10);
    }

    fn move_command_selection(&mut self, direction: i32, amount: usize) {
        if self.command_roots.is_empty() {
            self.command_selected = 0;
            return;
        }

        if direction >= 0 {
            self.command_selected =
                (self.command_selected + amount).min(self.command_roots.len() - 1);
        } else {
            self.command_selected = self.command_selected.saturating_sub(amount);
        }
    }

    fn move_pane_selection(&mut self, pane: FocusPane, direction: i32, amount: usize) {
        match pane {
            FocusPane::Tree => {
                let len = self.tree_entries().len();
                if len > 0 {
                    if direction >= 0 {
                        self.selected_tree = (self.selected_tree + amount).min(len - 1);
                    } else {
                        self.selected_tree = self.selected_tree.saturating_sub(amount);
                    }
                    self.selected_track_row = 0;
                }
            }
            FocusPane::Tracks => {
                for _ in 0..amount {
                    if let Some(row) = self.next_track_row(direction) {
                        if row == self.selected_track_row {
                            break;
                        }
                        self.selected_track_row = row;
                    }
                }
            }
        }
        self.sync_selection();
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            FocusPane::Tree => FocusPane::Tracks,
            FocusPane::Tracks => FocusPane::Tree,
        };
    }

    fn activate(&mut self, conn: &Connection) -> Result<()> {
        let focus = self.focus;
        match self.focus {
            FocusPane::Tree => {
                if let Some((index, _track)) = self.selected_scope_tracks().first() {
                    self.play_index(conn, *index)?;
                } else {
                    self.message = String::from("no tracks in this selection");
                }
            }
            FocusPane::Tracks => self.play_selected_row(conn)?,
        }
        self.focus = focus;
        self.sync_selection();
        Ok(())
    }

    fn space_action(&mut self) {
        match self.focus {
            FocusPane::Tree => {
                self.toggle_artist_expansion();
                self.sync_selection();
            }
            FocusPane::Tracks => {
                self.message = String::from("space is tree expand; use x/c/v/b/z for playback");
            }
        }
    }

    fn toggle_artist_expansion(&mut self) {
        let Some(entry) = self.selected_tree_entry() else {
            return;
        };
        if matches!(
            entry,
            TreeEntry::Compilation | TreeEntry::CompilationAlbum { .. }
        ) {
            self.compilations_expanded = !self.compilations_expanded;
            self.message = if self.compilations_expanded {
                String::from("expanded Compilations")
            } else {
                String::from("collapsed Compilations")
            };
            return;
        }
        let artist = entry.artist().to_string();
        if self.expanded_artists.remove(&artist) {
            self.message = format!("collapsed {artist}");
        } else {
            self.expanded_artists.insert(artist.clone());
            self.message = format!("expanded {artist}");
        }
    }

    fn play_selected_row(&mut self, conn: &Connection) -> Result<()> {
        if let Some(index) = self.selected_playable_track_index() {
            self.play_index(conn, index)?;
        }
        Ok(())
    }

    fn play_next(&mut self, conn: &Connection) -> Result<()> {
        if let Some(index) = self.next_playback_index(1) {
            self.play_index(conn, index)?;
        } else {
            self.message = String::from("end of filtered playback view");
        }
        Ok(())
    }

    fn play_previous(&mut self, conn: &Connection) -> Result<()> {
        if let Some(index) = self.next_playback_index(-1) {
            self.play_index(conn, index)?;
        } else {
            self.message = String::from("start of filtered playback view");
        }
        Ok(())
    }

    fn play_from_controls(&mut self, conn: &Connection) -> Result<()> {
        match self.logical_state() {
            PlaybackState::Paused => self.resume_current()?,
            PlaybackState::Playing => {
                self.message = String::from("already playing");
            }
            PlaybackState::Stopped => self.play_selected_row(conn)?,
        }
        Ok(())
    }

    fn toggle_pause_current(&mut self) -> Result<()> {
        match self.logical_state() {
            PlaybackState::Playing => self.suspend_current()?,
            PlaybackState::Paused => self.resume_current()?,
            PlaybackState::Stopped => {
                self.message = String::from("nothing playing");
            }
        }
        Ok(())
    }

    fn play_index(&mut self, conn: &Connection, index: usize) -> Result<()> {
        if index >= self.tracks.len() {
            return Ok(());
        }

        self.finish_current(conn, false)?;
        let track = self.tracks[index].clone();
        match self.player.load(Path::new(&track.path)) {
            Ok(()) => {
                self.suspended_position_ms = None;
                self.message = format!("playing {}", track.display_title());
                self.current = Some(PlayingTrack {
                    index,
                    track,
                    last_position_ms: 0,
                    listened_ms: 0,
                });
                self.publish_now_playing();
                self.sync_media_playback(true);
            }
            Err(error) => {
                self.message = format!("could not play {}: {error:#}", track.path);
            }
        }
        Ok(())
    }

    fn toggle_pause(&mut self, conn: &Connection) -> Result<()> {
        match self.logical_state() {
            PlaybackState::Playing => self.suspend_current()?,
            PlaybackState::Paused => self.resume_current()?,
            PlaybackState::Stopped => self.play_selected_row(conn)?,
        }
        Ok(())
    }

    fn stop_current(&mut self, conn: &Connection) -> Result<()> {
        self.finish_current(conn, false)?;
        self.player.stop()?;
        self.message = String::from("stopped");
        self.sync_media_playback(true);
        Ok(())
    }

    fn finish_current(&mut self, conn: &Connection, natural_end: bool) -> Result<()> {
        let Some(mut current) = self.current.take() else {
            return Ok(());
        };
        if let Some(position_ms) = self.suspended_position_ms.take() {
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
        Ok(())
    }

    fn update_playback(&mut self, conn: &Connection) -> Result<bool> {
        if self.current.is_none() {
            return Ok(false);
        }
        if self.suspended_position_ms.is_some() {
            self.sync_media_playback(false);
            return Ok(false);
        }

        self.capture_current_progress();
        let mut changed = false;

        if self.current.is_some() && self.player.is_finished() {
            let next_index = self.next_auto_advance_index();
            self.finish_current(conn, true)?;
            if let Some(index) = next_index {
                self.play_index(conn, index)?;
            } else {
                self.player.stop()?;
            }
            changed = true;
        }
        self.sync_media_playback(false);
        Ok(changed)
    }

    fn shutdown(&mut self, conn: &Connection) -> Result<()> {
        self.finish_current(conn, false)?;
        self.player.stop()?;
        self.sync_media_playback(true);
        Ok(())
    }

    fn handle_media_commands(&mut self, conn: &Connection) -> Result<bool> {
        let mut handled = false;
        while let Some(command) = self.media_session.next_command() {
            handled = true;
            match command {
                MediaCommand::Play => {
                    if self.current.is_some() {
                        self.resume_current()?;
                    } else {
                        self.play_selected_row(conn)?;
                    }
                }
                MediaCommand::Pause => {
                    self.suspend_current()?;
                }
                MediaCommand::Toggle => self.toggle_pause(conn)?,
                MediaCommand::Stop => self.stop_current(conn)?,
                MediaCommand::Next => self.play_next(conn)?,
                MediaCommand::Previous => self.play_previous(conn)?,
                MediaCommand::SeekTo(position_ms) => {
                    if self.current.is_some() {
                        self.seek_to(position_ms)?;
                    }
                }
            }
        }
        Ok(handled)
    }

    fn handle_key(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.shutdown(conn)?;
            return Ok(true);
        }
        if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.clear_command_output();
            self.refresh(conn)?;
            return Ok(false);
        }

        if self.command_mode {
            match key.code {
                KeyCode::Esc => {
                    self.command_mode = false;
                    self.command.clear();
                    if self.clear_command_output() {
                        self.message = String::from("output cleared");
                    } else if self.filter.is_empty() {
                        self.message = String::from("command cancelled");
                    } else {
                        self.clear_filter();
                    }
                }
                KeyCode::Enter => self.submit_command(conn),
                KeyCode::Tab => self.complete_command(conn)?,
                KeyCode::Backspace => {
                    self.command.pop();
                }
                KeyCode::Char(char) => self.command.push(char),
                _ => {}
            }
            return Ok(false);
        }

        if self.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    self.clear_filter();
                }
                KeyCode::Enter | KeyCode::Tab => self.confirm_filter(),
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.selected_tree = 0;
                    self.selected_track_row = 0;
                    self.sync_selection();
                }
                KeyCode::Char(char) => {
                    self.filter.push(char);
                    self.selected_tree = 0;
                    self.selected_track_row = 0;
                    self.sync_selection();
                }
                _ => {}
            }
            return Ok(false);
        }

        if self.command_focus {
            return self.handle_command_focus_key(conn, key);
        }

        if !matches!(key.code, KeyCode::Esc | KeyCode::Char(':')) {
            self.clear_command_output();
        }

        match key.code {
            KeyCode::Char('q') => {
                self.shutdown(conn)?;
                return Ok(true);
            }
            KeyCode::Esc => self.handle_escape(),
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::Enter => self.activate(conn)?,
            KeyCode::Char(' ') => self.space_action(),
            KeyCode::Char('e') => {
                self.toggle_artist_expansion();
                self.sync_selection();
            }
            KeyCode::Char('c') | KeyCode::Char('p') => {
                self.toggle_pause_current()?;
            }
            KeyCode::Char('C') => self.toggle_continuous(),
            KeyCode::Char('x') => self.play_from_controls(conn)?,
            KeyCode::Char('v') => self.stop_current(conn)?,
            KeyCode::Char('b') => self.play_next(conn)?,
            KeyCode::Char('z') => self.play_previous(conn)?,
            KeyCode::Char('L') => self.toggle_play_target(),
            KeyCode::Char('R') => self.toggle_repeat(),
            KeyCode::Char('S') => self.toggle_shuffle(),
            KeyCode::Char('i') => self.toggle_info_panel(),
            KeyCode::Char('I') => self.select_current_track(),
            KeyCode::Char(':') => {
                self.filter_mode = false;
                self.command_mode = true;
                self.command.clear();
                self.clear_command_output();
                self.message = String::from("typing command");
            }
            KeyCode::Char('/') => {
                self.filter_mode = true;
                self.message = String::from("typing filter");
            }
            KeyCode::Left | KeyCode::Char('h') => self.seek_relative(-SCRUB_SECONDS)?,
            KeyCode::Right | KeyCode::Char('l') => self.seek_relative(SCRUB_SECONDS)?,
            KeyCode::Char(',') => self.seek_relative(-60)?,
            KeyCode::Char('.') => self.seek_relative(60)?,
            _ => {}
        }
        Ok(false)
    }

    fn handle_command_focus_key(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => {
                self.shutdown(conn)?;
                return Ok(true);
            }
            KeyCode::Esc => {
                if self.clear_command_output() {
                    self.message = String::from("output cleared");
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_command_selection(1, 1),
            KeyCode::Up | KeyCode::Char('k') => self.move_command_selection(-1, 1),
            KeyCode::PageDown => self.move_command_selection(1, 10),
            KeyCode::PageUp => self.move_command_selection(-1, 10),
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_selected_library_root(conn)?,
            KeyCode::Tab => {
                self.clear_command_output();
                self.focus = FocusPane::Tree;
                self.apply_selection_state();
            }
            KeyCode::Char(':') => {
                self.clear_command_output();
                self.filter_mode = false;
                self.command_mode = true;
                self.command.clear();
                self.message = String::from("typing command");
            }
            KeyCode::Char('/') => {
                self.clear_command_output();
                self.filter_mode = true;
                self.message = String::from("typing filter");
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        if self.filter_mode || self.command_mode || self.command_focus {
            return false;
        }

        let direction = match mouse.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return false,
        };

        let Some(pane) = mouse_pane(
            mouse.column,
            mouse.row,
            terminal_width,
            terminal_height,
            self.reserved_bottom_rows(),
            self.info_area_visible(),
            self.input_bar_visible(),
        ) else {
            return false;
        };
        self.clear_command_output();
        self.move_pane_selection(pane, direction, MOUSE_SCROLL_LINES);
        true
    }

    fn seek_relative(&mut self, delta_seconds: i64) -> Result<()> {
        if self.current.is_none() {
            self.message = String::from("nothing playing");
            return Ok(());
        }
        let current_position_ms = self.player.position().as_millis() as i64;
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

    fn seek_to(&mut self, position_ms: i64) -> Result<bool> {
        let position_ms = position_ms.max(0);
        if self.suspended_position_ms.is_some() {
            self.suspended_position_ms = Some(position_ms);
            if let Some(current) = &mut self.current {
                current.align_position(position_ms);
            }
            self.sync_media_playback(true);
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
        self.sync_media_playback(true);
        Ok(true)
    }

    fn capture_current_progress(&mut self) {
        if self.current.is_none() || self.suspended_position_ms.is_some() {
            return;
        }
        let state = self.player.state();
        let position = self.player.position();
        if let Some(current) = &mut self.current {
            current.tick_position(position, state);
        }
    }

    fn suspend_current(&mut self) -> Result<()> {
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
        self.sync_media_playback(true);
        Ok(())
    }

    fn resume_current(&mut self) -> Result<()> {
        let Some(current) = self.current.as_ref() else {
            self.message = String::from("nothing playing");
            return Ok(());
        };
        let Some(position_ms) = self.suspended_position_ms.take() else {
            self.player.play()?;
            self.message = String::from("playing");
            self.sync_media_playback(true);
            return Ok(());
        };

        let path = current.track.path.clone();
        self.player.load(Path::new(&path))?;
        if position_ms > 0 {
            self.player
                .seek(Duration::from_millis(position_ms.max(0) as u64))?;
        }
        self.player.play()?;
        if let Some(current) = &mut self.current {
            current.align_position(position_ms);
        }
        self.message = String::from("playing");
        self.sync_media_playback(true);
        Ok(())
    }

    fn publish_now_playing(&mut self) {
        let Some(current) = &self.current else {
            return;
        };
        let cover_path = current.track.cover_path.as_deref().map(Path::new);
        let now_playing = NowPlaying {
            title: Some(current.track.display_title()),
            artist: current.track.artist.as_deref(),
            album: current.track.album.as_deref(),
            duration_ms: current.track.duration_ms,
            artwork_path: cover_path,
        };
        if let Err(error) = self.media_session.set_now_playing(&now_playing) {
            self.message = format!("media metadata unavailable: {error:#}");
        }
    }

    fn sync_media_playback(&mut self, force: bool) {
        let state = self.logical_state();
        let position_ms = self.current_position_ms();
        let position_s = position_ms / 1000;
        if !force
            && self.last_media_state == Some(state)
            && self.last_media_position_s == Some(position_s)
        {
            return;
        }

        if let Err(error) = self.media_session.set_playback_state(state, position_ms) {
            self.message = format!("media controls unavailable: {error:#}");
        } else {
            self.last_media_state = Some(state);
            self.last_media_position_s = Some(position_s);
        }
    }

    fn current_position_ms(&self) -> i64 {
        if let Some(position_ms) = self.suspended_position_ms {
            position_ms
        } else if self.current.is_some() {
            self.player.position().as_millis() as i64
        } else {
            0
        }
    }

    fn logical_state(&self) -> PlaybackState {
        if self.current.is_some() && self.suspended_position_ms.is_some() {
            PlaybackState::Paused
        } else {
            self.player.state()
        }
    }

    fn rebuild_search_cache(&mut self) {
        self.view.search_texts = self.tracks.iter().map(track_search_text).collect();
    }

    fn rebuild_filtered_indices(&mut self) {
        self.view.filtered_indices.clear();
        let query = FilterQuery::parse(&self.filter);
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

    fn rebuild_tree_entries(&mut self) {
        self.view.tree_entries.clear();
        if self
            .view
            .filtered_indices
            .iter()
            .any(|index| self.tracks[*index].compilation)
        {
            self.view.tree_entries.push(TreeEntry::Compilation);
            if self.compilations_expanded {
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
            if self.expanded_artists.contains(&artist) {
                let album = track.tree_album().to_string();
                if seen_albums.insert((artist.clone(), album.clone())) {
                    self.view
                        .tree_entries
                        .push(TreeEntry::Album { artist, album });
                }
            }
        }
    }

    fn rebuild_track_rows(&mut self) {
        self.view.track_rows.clear();
        let Some(entry) = self.selected_tree_entry().cloned() else {
            return;
        };
        let mut album_durations = HashMap::new();
        let mut album_years = HashMap::new();
        let mut album_discs = HashMap::new();
        for track in &self.tracks {
            if tree_entry_matches_track(&entry, track) {
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
        }

        let mut current_album: Option<String> = None;
        let mut current_disc: Option<i64> = None;
        for index in self.track_indices_for_entry(&entry) {
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

    fn tree_entries(&self) -> &[TreeEntry] {
        &self.view.tree_entries
    }

    fn track_rows(&self) -> &[TrackRow] {
        &self.view.track_rows
    }

    fn tree_entry_is_current(&self, entry: &TreeEntry) -> bool {
        let Some(current) = &self.current else {
            return false;
        };

        let current_artist = current.track.tree_artist();
        let current_album = current.track.tree_album();
        match entry {
            TreeEntry::Compilation => current.track.compilation && !self.compilations_expanded,
            TreeEntry::CompilationAlbum { album } => {
                current.track.compilation && current_album == album && self.compilations_expanded
            }
            TreeEntry::Artist { artist } => {
                artist == current_artist && !self.expanded_artists.contains(artist)
            }
            TreeEntry::Album { artist, album } => {
                artist == current_artist
                    && album == current_album
                    && self.expanded_artists.contains(artist)
            }
        }
    }

    fn selected_tree_entry(&self) -> Option<&TreeEntry> {
        self.view.tree_entries.get(self.selected_tree)
    }

    fn selected_scope_tracks(&self) -> Vec<(usize, &LibraryTrack)> {
        let Some(entry) = self.selected_tree_entry() else {
            return Vec::new();
        };
        self.track_indices_for_entry(entry)
            .into_iter()
            .map(|index| (index, &self.tracks[index]))
            .collect()
    }

    fn track_indices_for_entry(&self, entry: &TreeEntry) -> Vec<usize> {
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

    fn selected_playable_track_index(&self) -> Option<usize> {
        let rows = self.track_rows();
        if let Some(TrackRow::Track { track_index, .. }) = rows.get(self.selected_track_row) {
            return Some(*track_index);
        }

        rows.iter()
            .skip(self.selected_track_row)
            .find_map(|row| match row {
                TrackRow::Track { track_index, .. } => Some(*track_index),
                TrackRow::AlbumHeader { .. } | TrackRow::DiscDivider { .. } => None,
            })
            .or_else(|| {
                rows.iter().rev().find_map(|row| match row {
                    TrackRow::Track { track_index, .. } => Some(*track_index),
                    TrackRow::AlbumHeader { .. } | TrackRow::DiscDivider { .. } => None,
                })
            })
    }

    fn nearest_track_row(&self, from: usize) -> Option<usize> {
        let rows = self.track_rows();
        if matches!(rows.get(from), Some(TrackRow::Track { .. })) {
            return Some(from);
        }

        rows.iter()
            .enumerate()
            .skip(from)
            .find_map(|(row, entry)| matches!(entry, TrackRow::Track { .. }).then_some(row))
            .or_else(|| {
                rows.iter()
                    .enumerate()
                    .take(from)
                    .rev()
                    .find_map(|(row, entry)| matches!(entry, TrackRow::Track { .. }).then_some(row))
            })
    }

    fn next_track_row(&self, direction: i32) -> Option<usize> {
        let rows = self.track_rows();
        if rows.is_empty() {
            return None;
        }

        let current = self.selected_track_row.min(rows.len() - 1);
        if direction >= 0 {
            rows.iter()
                .enumerate()
                .skip(current + 1)
                .find_map(|(row, entry)| matches!(entry, TrackRow::Track { .. }).then_some(row))
                .or_else(|| {
                    matches!(rows.get(current), Some(TrackRow::Track { .. })).then_some(current)
                })
        } else {
            rows.iter()
                .enumerate()
                .take(current)
                .rev()
                .find_map(|(row, entry)| matches!(entry, TrackRow::Track { .. }).then_some(row))
                .or_else(|| {
                    matches!(rows.get(current), Some(TrackRow::Track { .. })).then_some(current)
                })
        }
    }

    fn next_playback_index(&mut self, direction: i32) -> Option<usize> {
        let sequence = self.playback_sequence_indices();
        if sequence.is_empty() {
            return None;
        }

        let anchor = self.playback_anchor_index();
        if self.shuffle {
            return self.next_shuffle_playback_index(&sequence, anchor, direction);
        }

        self.next_ordered_playback_index(&sequence, anchor, direction)
    }

    fn next_auto_advance_index(&mut self) -> Option<usize> {
        self.continuous
            .then(|| self.next_playback_index(1))
            .flatten()
    }

    fn next_ordered_playback_index(
        &self,
        sequence: &[usize],
        anchor: Option<usize>,
        direction: i32,
    ) -> Option<usize> {
        if let Some(anchor) = anchor {
            if let Some(position) = sequence.iter().position(|index| *index == anchor) {
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

            if let Some(selected) = self
                .selected_playable_track_index()
                .filter(|selected| sequence.contains(selected))
            {
                return Some(selected);
            }
        }

        if direction >= 0 {
            sequence.first().copied()
        } else {
            sequence.last().copied()
        }
    }

    fn next_shuffle_playback_index(
        &mut self,
        sequence: &[usize],
        anchor: Option<usize>,
        direction: i32,
    ) -> Option<usize> {
        self.ensure_shuffle_order(sequence);
        if self.shuffle_order.is_empty() {
            return None;
        }

        if let Some(anchor) = anchor {
            if let Some(position) = self.shuffle_order.iter().position(|index| *index == anchor) {
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

            if let Some(selected) = self
                .selected_playable_track_index()
                .filter(|selected| sequence.contains(selected))
            {
                return Some(selected);
            }
        }

        if direction >= 0 {
            self.shuffle_order.first().copied()
        } else {
            self.shuffle_order.last().copied()
        }
    }

    fn playback_sequence_indices(&self) -> Vec<usize> {
        let Some(anchor) = self.playback_anchor_index() else {
            return self.view.filtered_indices.clone();
        };
        let Some(anchor_track) = self.tracks.get(anchor) else {
            return self.view.filtered_indices.clone();
        };

        match self.play_target {
            PlayTarget::Library => self.view.filtered_indices.clone(),
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
                .collect(),
        }
    }

    fn playback_anchor_index(&self) -> Option<usize> {
        self.current
            .as_ref()
            .map(|current| current.index)
            .or_else(|| self.selected_playable_track_index())
    }

    fn ensure_shuffle_order(&mut self, sequence: &[usize]) {
        if self.shuffle_scope != sequence {
            self.rebuild_shuffle_order(sequence);
        }
    }

    fn rebuild_shuffle_order(&mut self, sequence: &[usize]) {
        self.shuffle_scope = sequence.to_vec();
        self.shuffle_order = sequence.to_vec();
        for index in (1..self.shuffle_order.len()).rev() {
            let swap_with = (self.next_shuffle_u64() as usize) % (index + 1);
            self.shuffle_order.swap(index, swap_with);
        }
    }

    fn reset_shuffle_order(&mut self) {
        self.shuffle_scope.clear();
        self.shuffle_order.clear();
    }

    fn next_shuffle_u64(&mut self) -> u64 {
        self.shuffle_seed = self
            .shuffle_seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.shuffle_seed
    }

    fn select_current_track(&mut self) {
        if let Some(index) = self.current.as_ref().map(|current| current.index) {
            self.select_track_index(index);
            self.message = String::from("selected current track");
        } else {
            self.message = String::from("nothing playing");
        }
    }

    fn select_track_index(&mut self, index: usize) {
        if let Some(track) = self.tracks.get(index) {
            let artist = track.tree_artist().to_string();
            let album = track.tree_album().to_string();
            let is_compilation = track.compilation;
            let current_entry_matches = self
                .selected_tree_entry()
                .map(|entry| match entry {
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
                    self.compilations_expanded = true;
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
                        self.selected_tree = position;
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
                    self.selected_tree = position;
                    tree_changed = true;
                } else if let Some(position) = self.tree_entries().iter().position(|entry| {
                    matches!(
                        entry,
                        TreeEntry::Artist {
                            artist: entry_artist
                        } if entry_artist == &artist
                    )
                }) {
                    self.selected_tree = position;
                    tree_changed = true;
                }
                if tree_changed {
                    self.sync_selection();
                }
            }
        }

        if let Some(position) = self.track_rows().iter().position(|row| {
            matches!(
                row,
                TrackRow::Track {
                    track_index,
                    ..
                } if *track_index == index
            )
        }) {
            self.selected_track_row = position;
        }
        self.apply_selection_state();
    }

    fn filter_display(&self) -> &str {
        if self.filter.is_empty() {
            "none"
        } else {
            &self.filter
        }
    }

    fn confirm_filter(&mut self) {
        let warning = FilterQuery::parse(&self.filter)
            .warning()
            .map(str::to_string);
        self.filter_mode = false;
        self.focus = FocusPane::Tree;
        self.selected_tree = 0;
        self.selected_track_row = 0;
        self.reset_shuffle_order();
        self.sync_selection();
        self.message = warning.unwrap_or_else(|| format!("filter: {}", self.filter_display()));
    }

    fn clear_filter(&mut self) {
        let selected_tree_entry = self.selected_tree_entry().cloned();
        let selected_track_index = self.selected_playable_track_index();

        self.filter_mode = false;
        self.filter.clear();
        self.reset_shuffle_order();
        self.rebuild_filtered_indices();
        self.rebuild_tree_entries();
        if let Some(position) = selected_tree_entry.as_ref().and_then(|entry| {
            self.tree_entries()
                .iter()
                .position(|candidate| candidate == entry)
        }) {
            self.selected_tree = position;
        } else {
            self.clamp_tree_selection();
        }

        self.rebuild_track_rows();
        if let Some(position) = selected_track_index.and_then(|index| {
            self.track_rows().iter().position(|row| {
                matches!(
                    row,
                    TrackRow::Track {
                        track_index,
                        ..
                    } if *track_index == index
                )
            })
        }) {
            self.selected_track_row = position;
        } else {
            self.clamp_track_selection();
        }
        self.apply_selection_state();
        self.message = String::from("filter cleared");
    }

    fn handle_escape(&mut self) {
        if self.clear_command_output() {
            self.message = String::from("output cleared");
        } else {
            self.clear_filter();
        }
    }

    fn show_command_output(&mut self, lines: Vec<String>) {
        self.command_output = lines;
        self.command_output_kind = CommandOutputKind::Text;
        self.command_roots.clear();
        self.command_selected = 0;
        self.command_focus = false;
    }

    fn show_library_roots(&mut self, roots: Vec<db::LibraryRoot>, selected_path: Option<&str>) {
        let active_count = roots.iter().filter(|root| root.active).count();
        let mut output = vec![format!(
            "library roots ({active_count} active / {} total)",
            roots.len()
        )];
        output.extend(
            roots
                .iter()
                .map(|root| format!("{} {}", if root.active { "[x]" } else { "[ ]" }, root.path)),
        );

        self.command_selected = selected_path
            .and_then(|path| roots.iter().position(|root| root.path == path))
            .unwrap_or(0)
            .min(roots.len().saturating_sub(1));
        self.command_focus = !roots.is_empty();
        self.command_output_kind = CommandOutputKind::LibraryRoots;
        self.command_roots = roots;
        self.command_output = output;
    }

    fn clear_command_output(&mut self) -> bool {
        if self.command_output.is_empty() && self.command_roots.is_empty() && !self.command_focus {
            false
        } else {
            self.command_output.clear();
            self.command_output_kind = CommandOutputKind::Text;
            self.command_roots.clear();
            self.command_selected = 0;
            self.command_focus = false;
            true
        }
    }

    #[cfg(test)]
    fn execute_command(&mut self, conn: &Connection) {
        self.command_mode = false;
        let command = std::mem::take(&mut self.command);
        let result = self.run_command(conn, command.trim());
        self.finish_command_result(result);
    }

    fn submit_command(&mut self, conn: &Connection) {
        self.command_mode = false;
        let command = std::mem::take(&mut self.command);
        if command_needs_busy(command.trim()) {
            self.pending_command = Some(command.clone());
            self.show_command_output(vec![
                format!("working: {}", display_command(&command)),
                String::from("scanning files recursively..."),
            ]);
            self.message = format!("working: {}", display_command(&command));
            self.show_transient_status(self.message.clone());
        } else {
            let result = self.run_command(conn, command.trim());
            self.finish_command_result(result);
        }
    }

    fn execute_pending_command(&mut self, conn: &Connection) -> bool {
        let Some(command) = self.pending_command.take() else {
            return false;
        };
        let result = self.run_command(conn, command.trim());
        self.finish_command_result(result);
        true
    }

    fn finish_command_result(&mut self, result: Result<String>) {
        self.message = match result {
            Ok(message) => message,
            Err(error) => format!("command failed: {error:#}"),
        };
        self.show_transient_status(self.message.clone());
    }

    fn complete_command(&mut self, conn: &Connection) -> Result<()> {
        let result = complete_command_input(conn, &self.command)?;
        if let Some(replacement) = result.replacement {
            self.command = replacement;
        }
        if let Some(notice) = result.notice {
            self.message = notice;
            self.show_transient_status(self.message.clone());
        }
        Ok(())
    }

    fn run_command(&mut self, conn: &Connection, input: &str) -> Result<String> {
        let input = input.strip_prefix(':').unwrap_or(input).trim();
        if input.is_empty() {
            return Ok(String::from("empty command"));
        }

        let mut parts = input.splitn(2, char::is_whitespace);
        let command = parts.next().unwrap_or_default().to_ascii_lowercase();
        let rest = parts.next().unwrap_or_default().trim();

        match command.as_str() {
            "add" => {
                self.clear_command_output();
                self.command_add(conn, rest)
            }
            "remove" | "rm" => {
                self.clear_command_output();
                self.command_remove(conn, rest)
            }
            "update" | "u" => {
                self.clear_command_output();
                self.command_update(conn, rest)
            }
            "library" | "roots" => self.command_library(conn),
            "filter" | "f" => {
                self.clear_command_output();
                self.filter = rest.to_string();
                self.confirm_filter();
                Ok(format!("filter: {}", self.filter_display()))
            }
            "clear" | "clear-filter" => {
                self.clear_command_output();
                self.clear_filter();
                Ok(String::from("filter cleared"))
            }
            "clear-output" | "close" | "hide" => {
                if self.clear_command_output() {
                    Ok(String::from("output cleared"))
                } else {
                    Ok(String::from("no output to clear"))
                }
            }
            _ => Ok(format!("unknown command: {command}")),
        }
    }

    fn command_add(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        let Some(path) = command_path(raw_path) else {
            return Ok(String::from("usage: :add PATH"));
        };
        let root = scanner::canonical_root(&path)?;
        let report = scanner::scan_path(conn, &self.paths, &root)?;
        db::upsert_library_root(conn, &root)?;
        db::mark_library_root_scanned(conn, &root)?;
        self.refresh(conn)?;
        Ok(scan_status("added", &root, &report))
    }

    fn command_remove(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        let Some(path) = command_path(raw_path) else {
            return Ok(String::from("usage: :remove PATH"));
        };
        let root = path.canonicalize().unwrap_or(path);
        if db::deactivate_library_root(conn, &root)? {
            self.refresh(conn)?;
            Ok(format!("removed {} from library", root.display()))
        } else {
            Ok(format!("no library root: {}", root.display()))
        }
    }

    fn command_update(&mut self, conn: &Connection, raw_path: &str) -> Result<String> {
        if let Some(path) = command_path(raw_path) {
            let root = scanner::canonical_root(&path)?;
            let report = scanner::rescan_path(conn, &self.paths, &root)?;
            db::upsert_library_root(conn, &root)?;
            db::mark_library_root_scanned(conn, &root)?;
            self.refresh(conn)?;
            return Ok(scan_status("updated", &root, &report));
        }

        let roots = db::active_library_roots(conn)?;
        if roots.is_empty() {
            return Ok(String::from("no active library roots; use :add PATH"));
        }

        let mut files_seen = 0;
        let mut tracks_stored = 0;
        let mut art_cached = 0;
        let mut files_skipped = 0;
        let mut files_marked_missing = 0;
        let mut duplicate_tracks_merged = 0;
        let mut errors = 0;
        for root in &roots {
            let path = PathBuf::from(&root.path);
            match scanner::rescan_path(conn, &self.paths, &path) {
                Ok(report) => {
                    files_seen += report.files_seen;
                    tracks_stored += report.tracks_stored;
                    art_cached += report.art_cached;
                    files_skipped += report.files_skipped;
                    files_marked_missing += report.files_marked_missing;
                    duplicate_tracks_merged += report.duplicate_tracks_merged;
                    errors += report.errors.len();
                    db::mark_library_root_scanned(conn, &path)?;
                }
                Err(_) => errors += 1,
            }
        }
        self.refresh(conn)?;
        Ok(format!(
            "updated {} roots, scanned {} files, stored {} tracks, cached {} covers, skipped {}, missing {}, merged {}, errors {}",
            roots.len(),
            files_seen,
            tracks_stored,
            art_cached,
            files_skipped,
            files_marked_missing,
            duplicate_tracks_merged,
            errors
        ))
    }

    fn command_library(&mut self, conn: &Connection) -> Result<String> {
        let roots = db::library_roots(conn)?;
        if roots.is_empty() {
            self.show_command_output(vec![
                String::from("library roots"),
                String::from("<legacy all scanned tracks>"),
            ]);
            return Ok(String::from("library roots: <legacy all scanned tracks>"));
        }

        self.show_library_roots(roots, None);

        let active: Vec<&str> = self
            .command_roots
            .iter()
            .filter(|root| root.active)
            .map(|root| root.path.as_str())
            .collect();
        if active.is_empty() {
            Ok(String::from("library roots: <none active>"))
        } else {
            Ok(format!("library roots: {}", active.join("; ")))
        }
    }

    fn toggle_selected_library_root(&mut self, conn: &Connection) -> Result<()> {
        if self.command_output_kind != CommandOutputKind::LibraryRoots {
            return Ok(());
        }
        let Some(root) = self.command_roots.get(self.command_selected).cloned() else {
            self.message = String::from("no library root selected");
            return Ok(());
        };

        let next_active = !root.active;
        if db::set_library_root_active(conn, Path::new(&root.path), next_active)? {
            self.refresh(conn)?;
            let roots = db::library_roots(conn)?;
            self.show_library_roots(roots, Some(&root.path));
            self.message = format!(
                "{} {}",
                if next_active { "enabled" } else { "disabled" },
                root.path
            );
            self.show_transient_status(self.message.clone());
        } else {
            self.message = format!("no library root: {}", root.path);
        }
        Ok(())
    }

    fn filter_bar_visible(&self) -> bool {
        self.filter_mode || !self.filter.is_empty()
    }

    fn input_bar_visible(&self) -> bool {
        self.command_mode || self.filter_bar_visible()
    }

    fn command_output_visible(&self) -> bool {
        !self.command_output.is_empty()
    }

    fn info_area_visible(&self) -> bool {
        self.info_panel_visible
            || self.command_mode
            || self.command_output_visible()
            || self.filter_mode
    }

    fn command_output_height(&self) -> u16 {
        (self.command_output.len() as u16).min(COMMAND_OUTPUT_MAX_ROWS)
    }

    fn reserved_bottom_rows(&self) -> u16 {
        BOTTOM_STATUS_ROWS
    }

    fn toggle_info_panel(&mut self) {
        self.info_panel_visible = !self.info_panel_visible;
        self.message = format!(
            "info panel {}",
            if self.info_panel_visible {
                "shown"
            } else {
                "hidden"
            }
        );
        self.show_transient_status(self.message.clone());
    }

    fn toggle_play_target(&mut self) {
        self.play_target = self.play_target.next();
        self.reset_shuffle_order();
        self.message = format!("play target: {}", self.play_target.label());
        self.show_transient_status(self.message.clone());
    }

    fn toggle_continuous(&mut self) {
        self.continuous = !self.continuous;
        self.message = format!("continuous {}", if self.continuous { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    fn toggle_repeat(&mut self) {
        self.repeat = !self.repeat;
        self.message = format!("repeat {}", if self.repeat { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.reset_shuffle_order();
        self.message = format!("shuffle {}", if self.shuffle { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    fn show_transient_status(&mut self, text: String) {
        self.transient_status = Some(TransientStatus {
            text,
            until: Instant::now() + TRANSIENT_STATUS_DURATION,
        });
    }

    fn active_transient_status(&self) -> Option<&str> {
        self.transient_status
            .as_ref()
            .filter(|status| Instant::now() < status.until)
            .map(|status| status.text.as_str())
    }

    fn expire_transient_status(&mut self) -> bool {
        if self
            .transient_status
            .as_ref()
            .is_some_and(|status| Instant::now() >= status.until)
        {
            self.transient_status = None;
            true
        } else {
            false
        }
    }

    fn increment_cached_play_count(&mut self, media_item_id: i64) {
        for track in &mut self.tracks {
            if track.media_item_id == media_item_id {
                track.play_count += 1;
            }
        }
    }

    fn tick_interval(&self) -> Duration {
        if self.transient_status.is_some() {
            return MEDIA_IDLE_TICK;
        }
        match self.logical_state() {
            PlaybackState::Playing => ACTIVE_TICK,
            PlaybackState::Paused => MEDIA_IDLE_TICK,
            PlaybackState::Stopped => STOPPED_TICK,
        }
    }
}

fn track_search_text(track: &LibraryTrack) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {} {} {} {}",
        track.display_title(),
        track.display_artist(),
        track.display_album(),
        track.album_artist.as_deref().unwrap_or_default(),
        track
            .album_year
            .map(|year| year.to_string())
            .unwrap_or_default(),
        track.release_date.as_deref().unwrap_or_default(),
        track.composer.as_deref().unwrap_or_default(),
        track.genre.as_deref().unwrap_or_default(),
        track_root_label(track).unwrap_or_default(),
        if track.compilation { "compilation" } else { "" },
        track.play_count,
        track.path
    )
    .to_ascii_lowercase()
}

#[derive(Debug, Default)]
struct FilterQuery {
    terms: Vec<FilterTerm>,
    warnings: Vec<String>,
}

impl FilterQuery {
    fn parse(input: &str) -> Self {
        let mut query = Self::default();
        for token in split_filter_tokens(input) {
            match FilterTerm::parse(&token) {
                Ok(Some(term)) => query.terms.push(term),
                Ok(None) => {}
                Err(warning) => {
                    query.warnings.push(warning);
                    query.terms.push(FilterTerm::Invalid);
                }
            }
        }
        query
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    fn matches(&self, track: &LibraryTrack, haystack: &str) -> bool {
        self.terms.iter().all(|term| term.matches(track, haystack))
    }

    fn warning(&self) -> Option<&str> {
        self.warnings.first().map(String::as_str)
    }
}

#[derive(Debug)]
enum FilterTerm {
    Bare {
        needle: String,
        negated: bool,
    },
    Field {
        field: FilterField,
        matcher: FilterMatcher,
        negated: bool,
    },
    Invalid,
}

impl FilterTerm {
    fn parse(token: &str) -> std::result::Result<Option<Self>, String> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }

        let (negated, body) = token
            .strip_prefix('-')
            .map(|body| (true, body.trim()))
            .unwrap_or((false, token));
        if body.is_empty() {
            return Ok(None);
        }

        let Some((field_name, value)) = body.split_once(':') else {
            return Ok(Some(Self::Bare {
                needle: body.to_ascii_lowercase(),
                negated,
            }));
        };

        let Some(field) = FilterField::parse(field_name) else {
            return Err(format!("unknown filter field: {field_name}"));
        };

        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }

        let matcher = field.matcher(value)?;
        Ok(Some(Self::Field {
            field,
            matcher,
            negated,
        }))
    }

    fn matches(&self, track: &LibraryTrack, haystack: &str) -> bool {
        match self {
            Self::Bare { needle, negated } => apply_negation(haystack.contains(needle), *negated),
            Self::Field {
                field,
                matcher,
                negated,
            } => apply_negation(field.matches(track, matcher), *negated),
            Self::Invalid => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FilterField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    ReleaseDate,
    Genre,
    Composer,
    Root,
    Path,
    Compilation,
    Plays,
    TrackNumber,
    DiscNumber,
}

impl FilterField {
    fn parse(field: &str) -> Option<Self> {
        match field.trim().to_ascii_lowercase().as_str() {
            "title" | "track" | "name" => Some(Self::Title),
            "artist" => Some(Self::Artist),
            "album" => Some(Self::Album),
            "albumartist" | "album_artist" | "album-artist" | "aa" => Some(Self::AlbumArtist),
            "year" => Some(Self::Year),
            "date" | "released" | "release" => Some(Self::ReleaseDate),
            "genre" => Some(Self::Genre),
            "composer" => Some(Self::Composer),
            "root" | "library" | "library_root" | "library-root" => Some(Self::Root),
            "path" | "file" => Some(Self::Path),
            "comp" | "compilation" => Some(Self::Compilation),
            "plays" | "playcount" | "play_count" | "play-count" => Some(Self::Plays),
            "trackno" | "track_no" | "track_number" | "track-number" | "number" => {
                Some(Self::TrackNumber)
            }
            "disc" | "discno" | "disc_no" | "disc_number" | "disc-number" => Some(Self::DiscNumber),
            _ => None,
        }
    }

    fn matcher(self, value: &str) -> std::result::Result<FilterMatcher, String> {
        match self {
            Self::Year | Self::Plays | Self::TrackNumber | Self::DiscNumber => {
                parse_number_matcher(value)
                    .map(FilterMatcher::Number)
                    .ok_or_else(|| format!("expected a number for {}", self.name()))
            }
            Self::Compilation => parse_bool(value)
                .map(FilterMatcher::Bool)
                .ok_or_else(|| String::from("compilation expects true or false")),
            Self::ReleaseDate => parse_number_matcher(value)
                .map(FilterMatcher::Number)
                .or_else(|| Some(FilterMatcher::Text(value.to_ascii_lowercase())))
                .ok_or_else(|| String::from("expected a date or year")),
            _ => Ok(FilterMatcher::Text(value.to_ascii_lowercase())),
        }
    }

    fn matches(self, track: &LibraryTrack, matcher: &FilterMatcher) -> bool {
        match (self, matcher) {
            (Self::Title, FilterMatcher::Text(needle)) => {
                text_matches(track.display_title(), needle)
            }
            (Self::Artist, FilterMatcher::Text(needle)) => {
                text_matches(track.display_artist(), needle)
            }
            (Self::Album, FilterMatcher::Text(needle)) => {
                text_matches(track.display_album(), needle)
            }
            (Self::AlbumArtist, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.album_artist.as_deref(), needle)
            }
            (Self::Genre, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.genre.as_deref(), needle)
            }
            (Self::Composer, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.composer.as_deref(), needle)
            }
            (Self::Root, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.library_root.as_deref(), needle)
                    || track_root_label(track).is_some_and(|root| text_matches(&root, needle))
            }
            (Self::Path, FilterMatcher::Text(needle)) => text_matches(&track.path, needle),
            (Self::ReleaseDate, FilterMatcher::Text(needle)) => {
                optional_text_matches(track.release_date.as_deref(), needle)
                    || track
                        .album_year
                        .is_some_and(|year| text_matches(&year.to_string(), needle))
            }
            (Self::Year, FilterMatcher::Number(matcher)) => matcher.matches(track_year(track)),
            (Self::ReleaseDate, FilterMatcher::Number(matcher)) => {
                matcher.matches(track_year(track))
            }
            (Self::Plays, FilterMatcher::Number(matcher)) => {
                matcher.matches(Some(track.play_count))
            }
            (Self::TrackNumber, FilterMatcher::Number(matcher)) => {
                matcher.matches(track.track_number)
            }
            (Self::DiscNumber, FilterMatcher::Number(matcher)) => {
                matcher.matches(track.disc_number)
            }
            (Self::Compilation, FilterMatcher::Bool(value)) => track.compilation == *value,
            _ => false,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::AlbumArtist => "albumartist",
            Self::Year => "year",
            Self::ReleaseDate => "date",
            Self::Genre => "genre",
            Self::Composer => "composer",
            Self::Root => "root",
            Self::Path => "path",
            Self::Compilation => "compilation",
            Self::Plays => "plays",
            Self::TrackNumber => "trackno",
            Self::DiscNumber => "disc",
        }
    }
}

#[derive(Debug)]
enum FilterMatcher {
    Text(String),
    Bool(bool),
    Number(NumberMatcher),
}

#[derive(Debug)]
enum NumberMatcher {
    Equal(i64),
    Greater(i64),
    GreaterEqual(i64),
    Less(i64),
    LessEqual(i64),
    Range(Option<i64>, Option<i64>),
}

impl NumberMatcher {
    fn matches(&self, value: Option<i64>) -> bool {
        let Some(value) = value else {
            return false;
        };
        match self {
            Self::Equal(target) => value == *target,
            Self::Greater(target) => value > *target,
            Self::GreaterEqual(target) => value >= *target,
            Self::Less(target) => value < *target,
            Self::LessEqual(target) => value <= *target,
            Self::Range(start, end) => {
                start.is_none_or(|start| value >= start) && end.is_none_or(|end| value <= end)
            }
        }
    }
}

fn split_filter_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_char) = quote {
            if character == quote_char {
                quote = None;
            } else {
                token.push(character);
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            character if character.is_whitespace() => {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
            }
            character => token.push(character),
        }
    }

    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_number_matcher(value: &str) -> Option<NumberMatcher> {
    let value = value.trim();
    for (prefix, matcher) in [
        (
            ">=",
            NumberMatcher::GreaterEqual as fn(i64) -> NumberMatcher,
        ),
        ("<=", NumberMatcher::LessEqual as fn(i64) -> NumberMatcher),
        (">", NumberMatcher::Greater as fn(i64) -> NumberMatcher),
        ("<", NumberMatcher::Less as fn(i64) -> NumberMatcher),
    ] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return parse_filter_i64(rest).map(matcher);
        }
    }

    if let Some((start, end)) = value.split_once("..") {
        let start = (!start.trim().is_empty())
            .then(|| parse_filter_i64(start))
            .flatten();
        let end = (!end.trim().is_empty())
            .then(|| parse_filter_i64(end))
            .flatten();
        return (start.is_some() || end.is_some()).then_some(NumberMatcher::Range(start, end));
    }

    parse_filter_i64(value).map(NumberMatcher::Equal)
}

fn parse_filter_i64(value: &str) -> Option<i64> {
    value.trim().parse().ok()
}

fn track_year(track: &LibraryTrack) -> Option<i64> {
    track.album_year.or_else(|| {
        track
            .release_date
            .as_deref()
            .and_then(|date| date.as_bytes().windows(4).find_map(parse_year_window))
    })
}

fn parse_year_window(window: &[u8]) -> Option<i64> {
    window
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| std::str::from_utf8(window).ok()?.parse().ok())
        .flatten()
}

fn apply_negation(value: bool, negated: bool) -> bool {
    if negated {
        !value
    } else {
        value
    }
}

fn text_matches(value: &str, needle: &str) -> bool {
    value.to_ascii_lowercase().contains(needle)
}

fn optional_text_matches(value: Option<&str>, needle: &str) -> bool {
    value.is_some_and(|value| text_matches(value, needle))
}

fn tree_entry_matches_track(entry: &TreeEntry, track: &LibraryTrack) -> bool {
    match entry {
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

fn track_root_label(track: &LibraryTrack) -> Option<String> {
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

struct CompletionResult {
    replacement: Option<String>,
    notice: Option<String>,
}

#[derive(Clone)]
struct CompletionCandidate {
    value: String,
    is_dir: bool,
}

fn complete_command_input(conn: &Connection, input: &str) -> Result<CompletionResult> {
    let Some((command, before_arg, arg)) = split_command_arg(input) else {
        return Ok(complete_command_name(input));
    };

    match command.to_ascii_lowercase().as_str() {
        "add" | "update" | "u" => Ok(complete_path_arg(
            before_arg,
            arg,
            filesystem_candidates(arg),
        )),
        "remove" | "rm" => Ok(complete_path_arg(
            before_arg,
            arg,
            library_root_candidates(conn, arg)?,
        )),
        _ => Ok(CompletionResult {
            replacement: None,
            notice: Some(format!("{command} does not take path completion")),
        }),
    }
}

fn split_command_arg(input: &str) -> Option<(&str, &str, &str)> {
    let trimmed = input.trim_start();
    let leading_width = input.len() - trimmed.len();
    let command_width = trimmed.find(char::is_whitespace)?;
    let command = &trimmed[..command_width];
    let after_command = leading_width + command_width;
    let arg_start = input[after_command..]
        .find(|character: char| !character.is_whitespace())
        .map(|offset| after_command + offset)
        .unwrap_or(input.len());

    Some((command, &input[..arg_start], &input[arg_start..]))
}

fn complete_command_name(input: &str) -> CompletionResult {
    let prefix = input.trim_start();
    let leading = &input[..input.len() - prefix.len()];
    let matches: Vec<String> = COMMAND_NAMES
        .iter()
        .filter(|command| command.starts_with(prefix))
        .map(|command| (*command).to_string())
        .collect();

    complete_text(leading, prefix, matches, true)
}

fn complete_path_arg(
    before_arg: &str,
    arg: &str,
    candidates: Vec<CompletionCandidate>,
) -> CompletionResult {
    if candidates.is_empty() {
        return CompletionResult {
            replacement: None,
            notice: Some(String::from("no completion matches")),
        };
    }

    if candidates.len() == 1 {
        let candidate = &candidates[0];
        let suffix = if candidate.is_dir { "/" } else { " " };
        return CompletionResult {
            replacement: Some(format!("{before_arg}{}{suffix}", candidate.value)),
            notice: None,
        };
    }

    let values: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            if candidate.is_dir {
                format!("{}/", candidate.value)
            } else {
                candidate.value.clone()
            }
        })
        .collect();
    let common = common_prefix(&values);
    let replacement =
        (display_width(&common) > display_width(arg)).then(|| format!("{before_arg}{common}"));

    CompletionResult {
        replacement,
        notice: Some(matches_notice(&values)),
    }
}

fn complete_text(
    leading: &str,
    prefix: &str,
    matches: Vec<String>,
    append_space_on_unique: bool,
) -> CompletionResult {
    if matches.is_empty() {
        return CompletionResult {
            replacement: None,
            notice: Some(String::from("no completion matches")),
        };
    }

    if matches.len() == 1 {
        let suffix = if append_space_on_unique { " " } else { "" };
        return CompletionResult {
            replacement: Some(format!("{leading}{}{suffix}", matches[0])),
            notice: None,
        };
    }

    let common = common_prefix(&matches);
    CompletionResult {
        replacement: (display_width(&common) > display_width(prefix))
            .then(|| format!("{leading}{common}")),
        notice: Some(matches_notice(&matches)),
    }
}

fn filesystem_candidates(arg: &str) -> Vec<CompletionCandidate> {
    if arg == "~" {
        return vec![CompletionCandidate {
            value: String::from("~"),
            is_dir: true,
        }];
    }

    let lookup_arg = unquote_command_arg(arg);
    let lookup_path = expand_command_path(lookup_arg);
    let trailing_separator = lookup_arg.ends_with('/');
    let directory = if trailing_separator {
        lookup_path.clone()
    } else {
        lookup_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let name_prefix = if trailing_separator {
        String::new()
    } else {
        lookup_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let display_prefix = display_path_prefix(lookup_arg, trailing_separator);

    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(&name_prefix) {
            continue;
        }
        if !name_prefix.starts_with('.') && file_name.starts_with('.') {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        candidates.push(CompletionCandidate {
            value: format!("{display_prefix}{file_name}"),
            is_dir,
        });
    }
    sort_candidates(candidates)
}

fn library_root_candidates(conn: &Connection, arg: &str) -> Result<Vec<CompletionCandidate>> {
    let lookup_arg = unquote_command_arg(arg);
    let expanded = expand_command_path(lookup_arg);
    let prefix = expanded.to_string_lossy();
    let mut candidates: Vec<CompletionCandidate> = db::active_library_roots(conn)?
        .into_iter()
        .filter(|root| root.path.starts_with(prefix.as_ref()))
        .map(|root| CompletionCandidate {
            value: root.path,
            is_dir: false,
        })
        .collect();

    if candidates.is_empty() {
        candidates = filesystem_candidates(arg);
    }
    Ok(sort_candidates(candidates))
}

fn sort_candidates(mut candidates: Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    candidates.sort_by(|left, right| {
        left.value
            .to_ascii_lowercase()
            .cmp(&right.value.to_ascii_lowercase())
    });
    candidates
}

fn display_path_prefix(raw_path: &str, trailing_separator: bool) -> String {
    if trailing_separator {
        return raw_path.to_string();
    }

    raw_path
        .rfind('/')
        .map(|position| raw_path[..=position].to_string())
        .unwrap_or_default()
}

fn expand_command_path(raw_path: &str) -> PathBuf {
    if raw_path == "~" {
        return env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    }

    if let Some(rest) = raw_path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(raw_path)
}

fn common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for value in values.iter().skip(1) {
        while !value.starts_with(&prefix) {
            if prefix.pop().is_none() {
                return String::new();
            }
        }
    }
    prefix
}

fn matches_notice(matches: &[String]) -> String {
    let shown = matches
        .iter()
        .take(5)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("  ");
    if matches.len() > 5 {
        format!("matches: {shown}  ...")
    } else {
        format!("matches: {shown}")
    }
}

fn command_needs_busy(input: &str) -> bool {
    let input = input.strip_prefix(':').unwrap_or(input).trim();
    let Some(command) = input.split_whitespace().next() else {
        return false;
    };
    matches!(
        command.to_ascii_lowercase().as_str(),
        "add" | "update" | "u"
    )
}

fn display_command(input: &str) -> String {
    let command = input.trim();
    if command.starts_with(':') {
        command.to_string()
    } else {
        format!(":{command}")
    }
}

fn command_path(raw_path: &str) -> Option<PathBuf> {
    let raw_path = unquote_command_arg(raw_path.trim());
    if raw_path.is_empty() {
        return None;
    }

    if raw_path == "~" {
        return env::var_os("HOME").map(PathBuf::from);
    }

    if let Some(rest) = raw_path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return Some(PathBuf::from(home).join(rest));
        }
    }

    Some(PathBuf::from(raw_path))
}

fn unquote_command_arg(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        if first == b'"' || first == b'\'' {
            return &value[1..];
        }
    }
    value
}

fn scan_status(action: &str, root: &Path, report: &scanner::ScanReport) -> String {
    let mut status = format!(
        "{action} {}: stored {} tracks, cached {} covers, skipped {}",
        root.display(),
        report.tracks_stored,
        report.art_cached,
        report.files_skipped
    );
    if !report.errors.is_empty() {
        status.push_str(&format!(", errors {}", report.errors.len()));
    }
    if report.files_marked_missing > 0 {
        status.push_str(&format!(", missing {}", report.files_marked_missing));
    }
    if report.duplicate_tracks_merged > 0 {
        status.push_str(&format!(", merged {}", report.duplicate_tracks_merged));
    }
    status
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    conn: &Connection,
    _paths: &AppPaths,
    app: &mut App,
) -> Result<()> {
    let mut needs_draw = true;
    let mut last_render_position_s = None;
    let mut next_tick = Instant::now();

    loop {
        if Instant::now() >= next_tick {
            needs_draw |= app.expire_transient_status();
            app.media_session.tick();
            needs_draw |= app.handle_media_commands(conn)?;
            needs_draw |= app.update_playback(conn)?;

            if app.current.is_some() {
                let position_s = app.current_position_ms() / 1000;
                if app.logical_state() == PlaybackState::Playing
                    && last_render_position_s != Some(position_s)
                {
                    needs_draw = true;
                }
            }

            next_tick = Instant::now() + app.tick_interval();
        }

        if needs_draw {
            terminal.draw(|frame| render(frame, app))?;
            last_render_position_s = app
                .current
                .as_ref()
                .map(|_| app.current_position_ms() / 1000);
            needs_draw = false;
        }

        if app.execute_pending_command(conn) {
            needs_draw = true;
            next_tick = Instant::now();
            continue;
        }

        let input_wait = next_tick
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        if event::poll(input_wait)? {
            match event::read()? {
                Event::Key(key) => {
                    if app.handle_key(conn, key)? {
                        break;
                    }
                    needs_draw = true;
                    next_tick = Instant::now();
                }
                Event::Resize(_, _) => needs_draw = true,
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    if app.handle_mouse(mouse, size.width, size.height) {
                        needs_draw = true;
                        next_tick = Instant::now();
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn mouse_pane(
    column: u16,
    row: u16,
    terminal_width: u16,
    terminal_height: u16,
    reserved_bottom_rows: u16,
    info_visible: bool,
    input_visible: bool,
) -> Option<FocusPane> {
    let main_height = terminal_height.saturating_sub(reserved_bottom_rows);
    if terminal_width == 0 || main_height == 0 || row >= main_height {
        return None;
    }

    let info_height = if info_visible {
        info_panel_height(main_height, input_visible)
    } else {
        0
    };
    let browser_height = main_height
        .saturating_sub(info_height)
        .saturating_sub(u16::from(input_visible));
    if browser_height == 0 || row >= browser_height {
        return None;
    }

    if terminal_width < STACKED_PANE_WIDTH {
        let tree_height = percent_floor(browser_height, NARROW_TREE_PERCENT).max(1);
        if row < tree_height {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    } else {
        let tree_width = percent_floor(terminal_width, WIDE_TREE_PERCENT).max(1);
        if column < tree_width {
            return Some(FocusPane::Tree);
        }

        Some(FocusPane::Tracks)
    }
}

fn render(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    let info_visible = app.info_area_visible();
    let input_visible = app.input_bar_visible();
    let info_height = if info_visible {
        info_panel_height(
            area.height.saturating_sub(BOTTOM_STATUS_ROWS),
            input_visible,
        )
    } else {
        0
    };
    let mut constraints = vec![Constraint::Min(6)];
    if info_visible {
        constraints.push(Constraint::Length(info_height));
    }
    if input_visible {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Length(1));

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    render_browser_panes(frame, app, vertical[0]);

    let mut row = 1;
    if info_visible {
        render_info_pane(frame, app, vertical[row]);
        row += 1;
    }
    if input_visible {
        render_input_bar(frame, app, vertical[row]);
        row += 1;
    }

    let now = Paragraph::new(now_playing_line(app, usize::from(vertical[row].width)))
        .style(now_playing_row_style())
        .alignment(Alignment::Left);
    frame.render_widget(now, vertical[row]);
    row += 1;

    let status = Paragraph::new(playback_line(app, usize::from(vertical[row].width)))
        .alignment(Alignment::Left);
    frame.render_widget(status, vertical[row]);
}

fn render_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    if area.width < STACKED_PANE_WIDTH {
        render_stacked_browser_panes(frame, app, area);
    } else {
        render_wide_browser_panes(frame, app, area);
    }
}

fn render_wide_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(WIDE_TREE_PERCENT),
            Constraint::Percentage(100 - WIDE_TREE_PERCENT),
        ])
        .split(area);

    render_tree_pane(frame, app, columns[0]);
    render_tracks_pane(frame, app, columns[1]);
}

fn render_stacked_browser_panes(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let stack = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(NARROW_TREE_PERCENT),
            Constraint::Percentage(100 - NARROW_TREE_PERCENT),
        ])
        .split(area);

    render_tree_pane(frame, app, stack[0]);
    render_tracks_pane(frame, app, stack[1]);
}

fn render_tree_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let tree_active = pane_active(app, FocusPane::Tree);
    let tree = List::new(tree_items(app))
        .block(
            Block::default()
                .title("Library")
                .borders(Borders::ALL)
                .border_style(pane_border_style(tree_active)),
        )
        .scroll_padding(LIST_SCROLL_PADDING)
        .highlight_style(pane_highlight_style(tree_active));
    frame.render_stateful_widget(tree, area, &mut app.tree_state);
}

fn render_tracks_pane(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let tracks_active = pane_active(app, FocusPane::Tracks);
    let tracks_title = selected_scope_title(app);
    let track_width = usize::from(area.width.saturating_sub(2));
    let tracks = List::new(track_items(app, track_width))
        .block(
            Block::default()
                .title(tracks_title)
                .borders(Borders::ALL)
                .border_style(pane_border_style(tracks_active)),
        )
        .scroll_padding(LIST_SCROLL_PADDING)
        .highlight_style(pane_highlight_style(tracks_active));
    frame.render_stateful_widget(tracks, area, &mut app.track_state);
}

fn render_info_pane(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let command_info = app.command_mode || app.command_output_visible();
    let filter_info = !command_info && app.filter_mode;
    let info_inner_width = usize::from(area.width.saturating_sub(2));
    let info_inner_height = area.height.saturating_sub(2);
    let info_lines = if command_info {
        command_info_lines(app, info_inner_width, info_inner_height)
    } else if filter_info {
        filter_info_lines(app, info_inner_width, info_inner_height)
    } else {
        metadata_lines(app, info_inner_width)
    };
    let command_style = command_pane_style(app);
    let mut info_block = Block::default()
        .title(command_info_title(app))
        .borders(Borders::ALL)
        .border_style(if command_info {
            command_border_style(app)
        } else if filter_info {
            pane_border_style(true)
        } else {
            pane_border_style(false)
        });
    if command_info {
        info_block = info_block.style(command_style);
    }
    let mut info = Paragraph::new(info_lines)
        .block(info_block)
        .alignment(Alignment::Left);
    if command_info {
        info = info.style(command_style);
    }
    frame.render_widget(info, area);
}

fn render_input_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let input = Paragraph::new(input_line(app, usize::from(area.width)))
        .style(input_bar_style(app))
        .alignment(Alignment::Left);
    frame.render_widget(input, area);
}

fn command_info_title(app: &App) -> &'static str {
    if app.command_output_kind == CommandOutputKind::LibraryRoots {
        "Library"
    } else if app.filter_mode && !app.command_output_visible() {
        "Filter"
    } else if app.command_mode || app.command_output_visible() {
        "Command"
    } else {
        "Info"
    }
}

fn percent_floor(value: u16, percent: u16) -> u16 {
    ((u32::from(value) * u32::from(percent)) / 100) as u16
}

fn info_panel_height(available_height: u16, input_visible: bool) -> u16 {
    if available_height == 0 {
        return 0;
    }
    let reserved = TRACKS_MIN_HEIGHT + u16::from(input_visible);
    let height = available_height.saturating_sub(reserved);
    if height == 0 {
        1
    } else {
        height.min(INFO_PANEL_HEIGHT)
    }
}

fn pane_highlight_style(active: bool) -> Style {
    if active {
        Style::default()
            .bg(Color::Blue)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::White).fg(Color::Black)
    }
}

fn pane_active(app: &App, pane: FocusPane) -> bool {
    !app.command_mode && !app.filter_mode && !app.command_focus && app.focus == pane
}

fn pane_border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn input_bar_focused(app: &App) -> bool {
    app.command_mode || app.filter_mode
}

fn input_bar_style(app: &App) -> Style {
    if input_bar_focused(app) {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

fn command_pane_style(app: &App) -> Style {
    if app.command_mode {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

fn command_border_style(app: &App) -> Style {
    if app.command_mode {
        Style::default().fg(Color::White).bg(Color::Blue)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    }
}

fn placeholder_input_style(app: &App) -> Style {
    if input_bar_focused(app) {
        Style::default().fg(Color::Gray).bg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray).bg(Color::White)
    }
}

fn now_playing_row_style() -> Style {
    Style::default().fg(Color::Black).bg(Color::White)
}

fn tree_items(app: &App) -> Vec<ListItem<'static>> {
    let entries = app.tree_entries();
    if entries.is_empty() {
        return vec![ListItem::new(Line::from("no scanned tracks"))];
    }

    entries
        .iter()
        .map(|entry| ListItem::new(tree_item_line(app, entry)))
        .collect()
}

fn tree_item_line(app: &App, entry: &TreeEntry) -> Line<'static> {
    match entry {
        TreeEntry::Compilation => {
            let marker = if app.compilations_expanded {
                "[-]"
            } else {
                "[+]"
            };
            let current_prefix = if app.tree_entry_is_current(entry) {
                "> "
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(current_prefix, Style::default().fg(Color::LightGreen)),
                Span::styled(
                    "Compilations",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])
        }
        TreeEntry::CompilationAlbum { album } => Line::from(vec![
            Span::raw("    "),
            Span::styled(
                if app.tree_entry_is_current(entry) {
                    "> "
                } else {
                    ""
                },
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(album.clone(), Style::default().fg(Color::Cyan)),
        ]),
        TreeEntry::Artist { artist } => {
            let expanded = app.expanded_artists.contains(artist);
            let marker = if expanded { "[-]" } else { "[+]" };
            let current_prefix = if app.tree_entry_is_current(entry) {
                "> "
            } else {
                ""
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(current_prefix, Style::default().fg(Color::LightGreen)),
                Span::styled(
                    artist.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])
        }
        TreeEntry::Album { album, .. } => Line::from(vec![
            Span::raw("    "),
            Span::styled(
                if app.tree_entry_is_current(entry) {
                    "> "
                } else {
                    ""
                },
                Style::default().fg(Color::LightGreen),
            ),
            Span::styled(album.clone(), Style::default().fg(Color::Cyan)),
        ]),
    }
}

fn track_items(app: &App, width: usize) -> Vec<ListItem<'static>> {
    let rows = app.track_rows();
    if rows.is_empty() {
        return vec![ListItem::new(Line::from("no tracks in this view"))];
    }

    rows.iter()
        .map(|row| match row {
            TrackRow::AlbumHeader {
                album,
                album_year,
                duration_ms,
            } => ListItem::new(album_header_line(album, *album_year, *duration_ms, width)),
            TrackRow::DiscDivider { disc_number } => {
                ListItem::new(disc_divider_line(*disc_number, width))
            }
            TrackRow::Track {
                track_index,
                show_disc_number,
            } => ListItem::new(track_line(app, *track_index, *show_disc_number, width)),
        })
        .collect()
}

fn album_header_line(
    album: &str,
    album_year: Option<i64>,
    duration_ms: i64,
    width: usize,
) -> Line<'static> {
    let duration = db::format_duration(Some(duration_ms));
    let right = match album_year {
        Some(year) => format!("{year} {duration}"),
        None => duration,
    };
    let title_width = width.saturating_sub(display_width(&right) + 1);
    let title = truncate_to_width(album, title_width);
    let divider_width = width.saturating_sub(display_width(&title) + display_width(&right));
    Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            album_divider(divider_width),
            Style::default().fg(Color::LightMagenta),
        ),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ])
}

fn disc_divider_line(disc_number: Option<i64>, width: usize) -> Line<'static> {
    let label = disc_number
        .map(|disc| format!(" disc {disc} "))
        .unwrap_or_else(|| " disc ".to_string());
    let divider_width = width.saturating_sub(display_width(&label));
    let left = divider_width / 2;
    let right = divider_width.saturating_sub(left);
    Line::from(Span::styled(
        format!("{}{}{}", "-".repeat(left), label, "-".repeat(right)),
        Style::default().fg(Color::DarkGray),
    ))
}

fn track_line(
    app: &App,
    track_index: usize,
    show_disc_number: bool,
    width: usize,
) -> Line<'static> {
    let track = &app.tracks[track_index];
    let is_current = app
        .current
        .as_ref()
        .map(|current| current.index == track_index)
        .unwrap_or(false);
    let marker = if is_current { ">" } else { " " };
    let number = match (show_disc_number, track.disc_number, track.track_number) {
        (true, Some(disc), Some(track)) => format!("{disc}.{track:02}."),
        (_, _, Some(track)) => format!("{track:02}."),
        _ => "   ".to_string(),
    };
    let title_style = if is_current {
        Style::default().fg(Color::LightYellow)
    } else {
        Style::default()
    };
    let duration = db::format_duration(track.duration_ms);
    let play_count = format!("  x{}", track.play_count);
    let fixed_left = format!("{marker} {number} {play_count}");
    let title_width =
        width.saturating_sub(display_width(&fixed_left) + display_width(&duration) + 1);

    right_aligned_line(
        vec![
            Span::styled(marker, Style::default().fg(Color::LightGreen)),
            Span::raw(" "),
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                truncate_to_width(track.display_title(), title_width),
                title_style,
            ),
            Span::styled(play_count, Style::default().fg(Color::DarkGray)),
        ],
        vec![Span::styled(duration, Style::default().fg(Color::DarkGray))],
        width,
    )
}

fn right_aligned_line(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let left_width = spans_width(&left);
    let right_width = spans_width(&right);
    let padding = width.saturating_sub(left_width + right_width).max(1);
    left.push(Span::raw(" ".repeat(padding)));
    left.extend(right);
    Line::from(left)
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn album_divider(width: usize) -> String {
    match width {
        0 => String::new(),
        1 => " ".to_string(),
        2 => "  ".to_string(),
        width => format!(" {} ", "-".repeat(width - 2)),
    }
}

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width {
            break;
        }
        out.push(character);
        width += character_width;
    }
    out
}

fn fit_to_width(text: &str, width: usize) -> String {
    let mut text = truncate_to_width(text, width);
    let padding = width.saturating_sub(display_width(&text));
    if padding > 0 {
        text.push_str(&" ".repeat(padding));
    }
    text
}

fn selected_scope_title(app: &App) -> String {
    match app.selected_tree_entry() {
        Some(TreeEntry::Compilation) => "Compilations".to_string(),
        Some(TreeEntry::CompilationAlbum { album, .. }) => {
            format!("Compilations - {album}")
        }
        Some(TreeEntry::Artist { artist }) => artist.clone(),
        Some(TreeEntry::Album { artist, album, .. }) => format!("{artist} - {album}"),
        None => "Tracks".to_string(),
    }
}

fn now_playing_line(app: &App, width: usize) -> Line<'static> {
    let Some(current) = &app.current else {
        return right_aligned_line(
            vec![Span::raw(" idle ")],
            vec![Span::styled(
                format!("{} tracks", app.tracks.len()),
                Style::default().add_modifier(Modifier::BOLD),
            )],
            width,
        );
    };

    let left = format!(
        " {} - {}",
        current.track.display_artist(),
        current.track.display_title()
    );
    let right = match (current.track.display_album(), current.track.album_year) {
        ("", Some(year)) => format!("({year})"),
        ("", None) => String::new(),
        (album, Some(year)) => format!("{album} ({year})"),
        (album, None) => album.to_string(),
    };
    let left_width = width.saturating_sub(display_width(&right) + 1);

    right_aligned_line(
        vec![Span::styled(
            truncate_to_width(&left, left_width),
            Style::default().add_modifier(Modifier::BOLD),
        )],
        vec![Span::styled(right, Style::default().fg(Color::DarkGray))],
        width,
    )
}

fn playback_line(app: &App, width: usize) -> Line<'static> {
    if app.active_transient_status().is_some() {
        return Line::from(playback_progress_spans(app, width, 0));
    }

    let mut right = vec![Span::styled(
        format!(
            "{} | {}% | ",
            app.play_target.label(),
            progress_percent(app)
        ),
        Style::default().fg(Color::DarkGray),
    )];
    right.extend(playback_flag_spans(app));

    right_aligned_line(
        playback_progress_spans(app, width, spans_width(&right)),
        right,
        width,
    )
}

fn input_line(app: &App, width: usize) -> Line<'static> {
    if app.command_mode {
        command_line(app, width)
    } else {
        filter_line(app, width)
    }
}

fn command_info_lines(app: &App, width: usize, height: u16) -> Vec<Line<'static>> {
    let style = command_pane_style(app);
    if app.command_output_kind == CommandOutputKind::LibraryRoots {
        library_root_lines(app, width, height, style)
    } else if app.command_output_visible() {
        command_output_lines(app, width, height.min(app.command_output_height()), style)
    } else {
        command_help_lines(width, style)
    }
}

fn library_root_lines(app: &App, width: usize, height: u16, style: Style) -> Vec<Line<'static>> {
    let height = usize::from(height.min(COMMAND_OUTPUT_MAX_ROWS));
    if height == 0 {
        return Vec::new();
    }

    let roots = &app.command_roots;
    if roots.is_empty() {
        return command_output_lines(app, width, height as u16, style);
    }

    let active_count = roots.iter().filter(|root| root.active).count();
    let mut lines = vec![Line::from(Span::styled(
        truncate_to_width(
            &format!(
                " library roots ({active_count} active / {} total)",
                roots.len()
            ),
            width,
        ),
        style.add_modifier(Modifier::BOLD),
    ))];

    let root_slots = height.saturating_sub(1);
    if root_slots == 0 {
        return lines;
    }

    let selected = app.command_selected.min(roots.len() - 1);
    let offset = selected.saturating_add(1).saturating_sub(root_slots);
    for (index, root) in roots.iter().enumerate().skip(offset).take(root_slots) {
        let content = format!(" {} {}", if root.active { "[x]" } else { "[ ]" }, root.path);
        let selected_row = app.command_focus && index == selected;
        let row_style = if selected_row {
            pane_highlight_style(true)
        } else {
            style
        };
        let content = if selected_row {
            fit_to_width(&content, width)
        } else {
            truncate_to_width(&content, width)
        };
        lines.push(Line::from(Span::styled(content, row_style)));
    }
    lines
}

fn command_output_lines(app: &App, width: usize, height: u16, style: Style) -> Vec<Line<'static>> {
    let height = usize::from(height.min(COMMAND_OUTPUT_MAX_ROWS));
    if height == 0 {
        return Vec::new();
    }
    let hidden = if app.command_output.len() > height {
        app.command_output.len() - (height - 1)
    } else {
        0
    };
    let mut lines = Vec::new();
    for (index, text) in app.command_output.iter().take(height).enumerate() {
        let content = if hidden > 0 && index + 1 == height {
            format!(" ... {hidden} more")
        } else {
            format!(" {text}")
        };
        let style = if index == 0 {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        };
        lines.push(Line::from(Span::styled(
            truncate_to_width(&content, width),
            style,
        )));
    }
    lines
}

fn command_help_lines(width: usize, style: Style) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        " command mode",
        style.add_modifier(Modifier::BOLD),
    ))];
    lines.extend(command_list_lines(width, style));
    lines.push(Line::from(Span::styled(
        " Tab completes commands and paths",
        style,
    )));
    lines.push(Line::from(Span::styled(" Enter runs  Esc cancels", style)));
    lines
}

fn filter_info_lines(app: &App, width: usize, height: u16) -> Vec<Line<'static>> {
    let height = usize::from(height);
    if height == 0 {
        return Vec::new();
    }

    let query = FilterQuery::parse(&app.filter);
    let mut lines = vec![Line::from(Span::styled(
        " filter syntax",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ))];

    if let Some(warning) = query.warning() {
        lines.push(Line::from(Span::styled(
            truncate_to_width(&format!(" {warning}"), width),
            Style::default().fg(Color::LightRed),
        )));
    }

    for text in [
        "bare text searches title, artist, album, genre, composer, root, date, and path",
        "field:value narrows a field; prefix - to exclude a term",
        "fields: title artist album albumartist year date genre composer root path compilation plays trackno disc",
        "examples: genre:ambient year:2010..2020",
        "          root:Instrumental -compilation:true plays:>5",
    ] {
        lines.push(Line::from(Span::styled(
            truncate_to_width(&format!(" {text}"), width),
            Style::default().fg(Color::Gray),
        )));
    }

    lines.truncate(height);
    lines
}

fn command_list_lines(width: usize, style: Style) -> Vec<Line<'static>> {
    let prefix = " commands: ";
    let indent = " ".repeat(display_width(prefix));
    let mut lines = Vec::new();
    let mut current = prefix.to_string();

    for command in COMMAND_NAMES {
        let separator_width = usize::from(!current.ends_with(' '));
        let next_width = display_width(&current) + separator_width + display_width(command);
        if next_width <= width || current == prefix {
            if !current.ends_with(' ') {
                current.push(' ');
            }
            current.push_str(command);
        } else {
            lines.push(Line::from(Span::styled(
                truncate_to_width(&current, width),
                style,
            )));
            current = format!("{indent}{command}");
        }
    }

    lines.push(Line::from(Span::styled(
        truncate_to_width(&current, width),
        style,
    )));
    lines
}

fn metadata_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let Some(track) = app
        .selected_playable_track_index()
        .and_then(|index| app.tracks.get(index))
        .or_else(|| app.current.as_ref().map(|current| &current.track))
    else {
        return vec![Line::from(Span::styled(
            " no selected track",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    vec![
        Line::from(Span::styled(
            " selected track",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        metadata_pair("title", track.display_title(), width),
        metadata_pair("artist", fallback_text(track.display_artist()), width),
        metadata_pair("album", fallback_text(track.display_album()), width),
        metadata_pair(
            "composer",
            fallback_optional(track.composer.as_deref()),
            width,
        ),
        metadata_pair("genre", fallback_optional(track.genre.as_deref()), width),
        metadata_pair("released", release_date_text(track), width),
        metadata_track_position_pair(track, width),
        metadata_pair("length", db::format_duration(track.duration_ms), width),
        metadata_pair("plays", track.play_count.to_string(), width),
    ]
}

fn release_date_text(track: &LibraryTrack) -> String {
    track
        .release_date
        .clone()
        .or_else(|| track.album_year.map(|year| year.to_string()))
        .unwrap_or_else(|| "--".to_string())
}

fn metadata_track_position_pair(track: &LibraryTrack, width: usize) -> Line<'static> {
    let label = format!(" {:<9}", "track");
    let mut remaining = width.saturating_sub(display_width(&label));
    let mut spans = vec![Span::styled(label, Style::default().fg(Color::DarkGray))];
    let value_style = Style::default().fg(Color::White);
    let label_style = Style::default().fg(Color::DarkGray);

    if let Some(track_text) = track_number_text(track) {
        push_limited_span(&mut spans, &mut remaining, &track_text, value_style);
        if let Some(disc_text) = disc_number_text(track) {
            push_limited_span(&mut spans, &mut remaining, "  disc ", label_style);
            push_limited_span(&mut spans, &mut remaining, &disc_text, value_style);
        }
    } else if let Some(disc_text) = disc_number_text(track) {
        push_limited_span(&mut spans, &mut remaining, "disc ", label_style);
        push_limited_span(&mut spans, &mut remaining, &disc_text, value_style);
    } else {
        push_limited_span(&mut spans, &mut remaining, "--", value_style);
    }

    Line::from(spans)
}

fn track_number_text(track: &LibraryTrack) -> Option<String> {
    let track_number = track.track_number?;
    Some(match track.track_total {
        Some(track_total) => format!("{track_number}/{track_total}"),
        None => track_number.to_string(),
    })
}

fn disc_number_text(track: &LibraryTrack) -> Option<String> {
    let disc_number = track.disc_number?;
    Some(match track.disc_total {
        Some(disc_total) => format!("{disc_number}/{disc_total}"),
        None => disc_number.to_string(),
    })
}

fn metadata_pair(label: &str, value: impl AsRef<str>, width: usize) -> Line<'static> {
    let label = format!(" {label:<9}");
    let value_width = width.saturating_sub(display_width(&label));
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(
            truncate_to_width(value.as_ref(), value_width),
            Style::default().fg(Color::White),
        ),
    ])
}

fn push_limited_span(
    spans: &mut Vec<Span<'static>>,
    remaining: &mut usize,
    text: &str,
    style: Style,
) {
    if *remaining == 0 {
        return;
    }
    let text = truncate_to_width(text, *remaining);
    *remaining = (*remaining).saturating_sub(display_width(&text));
    spans.push(Span::styled(text, style));
}

fn fallback_text(value: &str) -> &str {
    if value.is_empty() {
        "--"
    } else {
        value
    }
}

fn fallback_optional(value: Option<&str>) -> &str {
    value.filter(|value| !value.is_empty()).unwrap_or("--")
}

fn command_line(app: &App, width: usize) -> Line<'static> {
    let text_width = width.saturating_sub(1);
    let style = input_bar_style(app);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(":", style),
        Span::styled(
            truncate_to_width(&format!("{}_", app.command), text_width.saturating_sub(1)),
            style,
        ),
    ])
}

fn filter_line(app: &App, width: usize) -> Line<'static> {
    let text_width = width.saturating_sub(1);
    let style = input_bar_style(app);
    let filter = if app.filter.is_empty() {
        Span::styled(
            truncate_to_width(
                "none_",
                text_width.saturating_sub(display_width("filter: ")),
            ),
            placeholder_input_style(app),
        )
    } else if app.filter_mode {
        Span::styled(
            truncate_to_width(
                &format!("{}_", app.filter),
                text_width.saturating_sub(display_width("filter: ")),
            ),
            style,
        )
    } else {
        Span::styled(
            truncate_to_width(
                &app.filter,
                text_width.saturating_sub(display_width("filter: ")),
            ),
            style,
        )
    };

    Line::from(vec![
        Span::raw(" "),
        Span::styled("filter: ", style),
        filter,
    ])
}

fn playback_flag_spans(app: &App) -> Vec<Span<'static>> {
    vec![
        Span::styled("C", active_flag_style(app.continuous)),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled("R", active_flag_style(app.repeat)),
        Span::styled(" ", Style::default().fg(Color::DarkGray)),
        Span::styled("S", active_flag_style(app.shuffle)),
    ]
}

fn active_flag_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn playback_progress_spans(app: &App, width: usize, right_width: usize) -> Vec<Span<'static>> {
    if let Some(status) = app.active_transient_status() {
        let available_width = width.saturating_sub(right_width + 1);
        return vec![
            Span::raw(" "),
            Span::styled(
                truncate_to_width(status, available_width.saturating_sub(1)),
                Style::default().fg(Color::White),
            ),
        ];
    }

    let position = db::format_duration(Some(app.current_position_ms()));
    let duration = app
        .current
        .as_ref()
        .map(|current| db::format_duration(current.track.duration_ms))
        .unwrap_or_else(|| "--:--".to_string());
    let time = format!("{position} / {duration}");
    let fixed_width = display_width(" > ") + display_width(&time) + display_width(" []");
    let available_bar_width = width.saturating_sub(fixed_width + right_width + 2);
    let bar_width = if available_bar_width >= 24 {
        available_bar_width.min(56)
    } else {
        available_bar_width
    };
    let playing = app.logical_state() == PlaybackState::Playing;
    let state_marker = if playing { ">" } else { "|" };
    let marker_style = if playing {
        Style::default().fg(Color::LightGreen)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    vec![
        Span::styled(format!(" {state_marker} "), marker_style),
        Span::styled(time, Style::default().fg(Color::White)),
        Span::raw(" "),
        Span::styled(
            format!("[{}]", progress_bar(app, bar_width)),
            Style::default().fg(Color::LightMagenta),
        ),
    ]
}

fn progress_percent(app: &App) -> i64 {
    let Some(current) = &app.current else {
        return 0;
    };
    let Some(duration_ms) = current.track.duration_ms.filter(|duration| *duration > 0) else {
        return 0;
    };
    ((app.current_position_ms().clamp(0, duration_ms) * 100) / duration_ms).clamp(0, 100)
}

fn progress_bar(app: &App, width: usize) -> String {
    let Some(current) = &app.current else {
        return "-".repeat(width);
    };
    let Some(duration_ms) = current.track.duration_ms.filter(|duration| *duration > 0) else {
        return "-".repeat(width);
    };
    let position_ms = app.current_position_ms().clamp(0, duration_ms);
    let filled = ((position_ms as usize) * width) / (duration_ms as usize);
    format!("{}{}", "=".repeat(filled), "-".repeat(width - filled))
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(Into::into)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_session::NoopMediaSession;
    use crate::player::NullPlayer;
    use ratatui::backend::TestBackend;
    use tempfile::tempdir;

    #[test]
    fn playback_sequence_respects_filter() {
        let mut app = test_app(vec![
            test_track(1, "keep one"),
            test_track(2, "skip this"),
            test_track(3, "keep two"),
        ]);
        app.filter = "keep".to_string();
        app.sync_selection();
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        assert_eq!(app.next_playback_index(1), Some(2));
        assert_eq!(app.next_playback_index(-1), None);
    }

    #[test]
    fn continuous_controls_auto_advance_only() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        assert_eq!(app.next_auto_advance_index(), Some(1));

        app.toggle_continuous();

        assert!(!app.continuous);
        assert_eq!(app.next_auto_advance_index(), None);
        assert_eq!(app.next_playback_index(1), Some(1));
    }

    #[test]
    fn playback_target_limits_sequence_to_current_artist_or_album() {
        let mut other_album = test_track(2, "same artist other album");
        other_album.album = Some("Other Album".to_string());
        let mut other_artist = test_track(3, "other artist track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![
            test_track(1, "first track"),
            other_album,
            other_artist,
        ]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        app.play_target = PlayTarget::Artist;
        assert_eq!(app.playback_sequence_indices(), vec![0, 1]);

        app.play_target = PlayTarget::Album;
        assert_eq!(app.playback_sequence_indices(), vec![0]);
    }

    #[test]
    fn repeat_wraps_playback_sequence() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        assert_eq!(app.next_playback_index(1), None);

        app.repeat = true;
        assert_eq!(app.next_playback_index(1), Some(0));
    }

    #[test]
    fn shuffle_uses_a_permuted_playback_order() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
            test_track(3, "third track"),
        ]);
        app.shuffle = true;
        app.shuffle_seed = 1;

        let next = app.next_playback_index(1);

        assert!(next.is_some());
        assert_eq!(app.shuffle_scope, vec![0, 1, 2]);
        assert_eq!(app.shuffle_order.len(), 3);
        assert_ne!(app.shuffle_order, vec![0, 1, 2]);
    }

    #[test]
    fn now_playing_line_splits_track_and_album() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 50_000,
            listened_ms: 0,
        });

        let text = line_text(&now_playing_line(&app, 80));

        assert_eq!(display_width(&text), 80);
        assert!(text.starts_with(" Artist - first track"));
        assert!(text.ends_with("Album (2018)"));
        assert_eq!(
            now_playing_row_style(),
            Style::default().fg(Color::Black).bg(Color::White)
        );
    }

    #[test]
    fn playback_line_shows_time_bar_and_play_modes() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 50_000,
            listened_ms: 0,
        });
        app.player.seek(Duration::from_millis(50_000)).unwrap();
        app.player.play().unwrap();
        app.play_target = PlayTarget::Album;
        app.repeat = true;
        app.shuffle = true;

        let line = playback_line(&app, 120);
        let text = line_text(&line);

        assert!(text.contains(" > 0:50 / 1:40 ["));
        assert!(text.contains("[============================----------------------------]"));
        assert!(text.contains("album from library | 50% | C R S"));
        assert_eq!(line.spans[0].style, Style::default().fg(Color::LightGreen));
        assert_eq!(
            line.spans[line.spans.len() - 5].style,
            Style::default().fg(Color::White)
        );
        assert_eq!(
            line.spans[line.spans.len() - 3].style,
            Style::default().fg(Color::White)
        );
        assert_eq!(
            line.spans[line.spans.len() - 1].style,
            Style::default().fg(Color::White)
        );
    }

    #[test]
    fn playback_line_uses_bar_marker_when_not_playing() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 50_000,
            listened_ms: 0,
        });
        app.suspended_position_ms = Some(50_000);

        let line = playback_line(&app, 80);
        let text = line_text(&line);

        assert!(text.contains(" | 0:50 / 1:40 ["));
        assert!(text.contains("| C R S"));
        assert_eq!(line.spans[0].style, Style::default().fg(Color::DarkGray));
        assert_eq!(
            line.spans[line.spans.len() - 5].style,
            Style::default().fg(Color::White)
        );
        assert_eq!(
            line.spans[line.spans.len() - 3].style,
            Style::default().fg(Color::DarkGray)
        );
        assert_eq!(
            line.spans[line.spans.len() - 1].style,
            Style::default().fg(Color::DarkGray)
        );
    }

    #[test]
    fn mode_toggles_show_transient_playback_status() {
        let mut app = test_app(vec![test_track(1, "first track")]);

        app.toggle_repeat();

        let text = line_text(&playback_line(&app, 80));
        assert!(text.contains(" repeat on"));
        assert!(!text.contains("| C R S"));
    }

    #[test]
    fn continuous_flag_reflects_toggle_state() {
        let mut app = test_app(vec![test_track(1, "first track")]);

        app.continuous = false;

        let line = playback_line(&app, 80);

        assert_eq!(
            line.spans[line.spans.len() - 5].style,
            Style::default().fg(Color::DarkGray)
        );
    }

    #[test]
    fn key_controls_match_cmus_style_bindings() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();

        assert!(!app
            .handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap());

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.message, "nothing playing");

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE))
            .unwrap();
        assert!(!app.continuous);
        assert_eq!(app.message, "continuous off");

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.repeat);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.shuffle);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.play_target, PlayTarget::Artist);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.play_target, PlayTarget::Artist);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.repeat);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.shuffle);
    }

    #[test]
    fn command_mode_executes_library_commands() {
        let data_dir = tempdir().unwrap();
        let library_dir = tempdir().unwrap();
        let db_path = data_dir.path().join("gmus.sqlite3");
        let conn = db::open(&db_path).unwrap();
        let mut app = test_app(Vec::new());
        app.paths = AppPaths {
            data_dir: data_dir.path().to_path_buf(),
            db_path,
            art_dir: data_dir.path().join("art"),
        };

        app.command = format!("add {}", library_dir.path().display());
        app.execute_command(&conn);

        let roots = db::active_library_roots(&conn).unwrap();
        assert_eq!(roots.len(), 1);
        assert!(app.message.starts_with("added "));

        app.command = String::from("library");
        app.execute_command(&conn);
        assert!(app.message.contains(library_dir.path().to_str().unwrap()));
        assert!(app.command_focus);
        assert_eq!(app.command_output_kind, CommandOutputKind::LibraryRoots);
        assert!(app.command_output[0].starts_with("library roots"));
        assert!(app.command_output[1].contains("[x]"));
        assert!(app.command_output[1].contains(library_dir.path().to_str().unwrap()));

        app.command = format!("remove {}", library_dir.path().display());
        app.execute_command(&conn);

        assert!(db::active_library_roots(&conn).unwrap().is_empty());
        assert!(app.message.starts_with("removed "));
    }

    #[test]
    fn library_command_focuses_root_list_and_toggles_roots() {
        let data_dir = tempdir().unwrap();
        let root_a = tempdir().unwrap();
        let root_b = tempdir().unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, root_a.path()).unwrap();
        db::upsert_library_root(&conn, root_b.path()).unwrap();
        let mut app = test_app(Vec::new());

        app.command = String::from("library");
        app.execute_command(&conn);

        assert!(app.command_focus);
        assert_eq!(app.command_output_kind, CommandOutputKind::LibraryRoots);
        assert_eq!(app.command_roots.len(), 2);
        assert_eq!(app.command_selected, 0);
        assert_eq!(command_info_title(&app), "Library");
        assert_eq!(
            command_info_lines(&app, 80, 10)[1].spans[0].style,
            pane_highlight_style(true)
        );

        app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.command_selected, 1);
        let toggled_path = app.command_roots[1].path.clone();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        let roots = db::library_roots(&conn).unwrap();
        assert!(
            !roots
                .iter()
                .find(|root| root.path == toggled_path)
                .unwrap()
                .active
        );
        assert!(app.command_focus);
        assert_eq!(app.command_roots[app.command_selected].path, toggled_path);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .unwrap();

        let roots = db::library_roots(&conn).unwrap();
        assert!(
            roots
                .iter()
                .find(|root| root.path == toggled_path)
                .unwrap()
                .active
        );
    }

    #[test]
    fn colon_opens_command_bar() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .unwrap();

        assert!(app.command_mode);
        assert!(app.input_bar_visible());
        assert_eq!(line_text(&input_line(&app, 20)), " :l_");
        assert_eq!(
            input_line(&app, 20).spans[1].style,
            Style::default().fg(Color::White).bg(Color::Blue)
        );
    }

    #[test]
    fn library_output_renders_in_info_pane() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.command_output = vec![
            String::from("library roots (1 active / 1 total)"),
            String::from("[x] /tmp/music"),
        ];

        let lines = command_info_lines(&app, 80, 10);

        assert!(app.command_output_visible());
        assert_eq!(app.command_output_height(), 2);
        assert_eq!(line_text(&lines[0]), " library roots (1 active / 1 total)");
        assert_eq!(line_text(&lines[1]), " [x] /tmp/music");
        assert_eq!(
            lines[0].spans[0].style,
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn command_help_lists_available_commands() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.command_mode = true;
        app.command = String::from("library");

        let text = lines_text(&command_info_lines(&app, 120, 10));

        assert!(text.contains("commands: add remove update library filter clear clear-output"));
        assert!(!text.contains(":library_"));
        assert_eq!(
            command_info_lines(&app, 120, 10)[0].spans[0].style,
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn command_help_wraps_command_list() {
        let lines = command_help_lines(28, Style::default().fg(Color::Black).bg(Color::White));
        let text = lines_text(&lines);
        let command_lines: Vec<String> = lines
            .iter()
            .map(line_text)
            .filter(|line| line.contains("commands:") || line.starts_with("           "))
            .collect();

        assert!(command_lines.len() > 1);
        assert!(text.contains("commands: add remove"));
        assert!(text.contains("clear-output"));
        assert!(command_lines.iter().all(|line| display_width(line) <= 28));
    }

    #[test]
    fn info_panel_toggle_preserves_command_info_overlay() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .unwrap();
        assert!(!app.info_panel_visible);
        assert!(!app.info_area_visible());

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.info_area_visible());

        app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.command_mode);
        assert!(!app.info_panel_visible);
        assert!(!app.info_area_visible());

        app.show_command_output(vec![String::from("library roots")]);
        assert!(app.info_area_visible());
        app.clear_command_output();
        assert!(!app.info_area_visible());
    }

    #[test]
    fn escape_clears_command_output_before_filter() {
        let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
        let conn = Connection::open_in_memory().unwrap();
        app.filter = String::from("keep");
        app.command_output = vec![
            String::from("library roots"),
            String::from("[x] /tmp/music"),
        ];
        app.sync_selection();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();

        assert!(app.command_output.is_empty());
        assert_eq!(app.filter, "keep");
        assert_eq!(app.playback_sequence_indices(), &[0]);
    }

    #[test]
    fn normal_navigation_clears_command_output() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        let conn = Connection::open_in_memory().unwrap();
        app.command_output = vec![
            String::from("library roots"),
            String::from("[x] /tmp/music"),
        ];

        app.handle_key(&conn, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();

        assert!(app.command_output.is_empty());
    }

    #[test]
    fn metadata_pane_shows_selected_track_details() {
        let mut track = test_track(1, "first track");
        track.composer = Some("Someone Quiet".to_string());
        track.genre = Some("Ambient".to_string());
        track.track_total = Some(12);
        track.disc_number = Some(1);
        track.disc_total = Some(2);
        let app = test_app(vec![track]);

        let lines = metadata_lines(&app, 80);
        let text = lines_text(&lines);
        let track_line = &lines[7];

        assert!(text.contains("selected track"));
        assert!(text.contains("title    first track"));
        assert!(text.contains("artist   Artist"));
        assert!(text.contains("album    Album"));
        assert!(text.contains("composer Someone Quiet"));
        assert!(text.contains("genre    Ambient"));
        assert!(text.contains("released 2018-05-11"));
        assert!(text.contains("track    1/12  disc 1/2"));
        assert!(text.contains("plays    0"));
        assert!(!text.contains("/tmp/first track.flac"));
        assert_eq!(
            track_line.spans[0].style,
            Style::default().fg(Color::DarkGray)
        );
        assert_eq!(track_line.spans[1].style, Style::default().fg(Color::White));
        assert_eq!(
            track_line.spans[2].style,
            Style::default().fg(Color::DarkGray)
        );
        assert_eq!(track_line.spans[3].style, Style::default().fg(Color::White));
    }

    #[test]
    fn scan_commands_queue_busy_output_before_running() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();
        app.command_mode = true;
        app.command = String::from("update");

        app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        assert!(!app.command_mode);
        assert_eq!(app.pending_command.as_deref(), Some("update"));
        assert!(app.command_output[0].contains("working: :update"));
        assert!(app.command_output[1].contains("scanning files"));
    }

    #[test]
    fn tab_completes_command_names() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();
        app.command_mode = true;
        app.command = String::from("lib");

        app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.command, "library ");
    }

    #[test]
    fn tab_completes_filesystem_paths_for_add() {
        let parent = tempdir().unwrap();
        let music = parent.path().join("MusicRoot");
        fs::create_dir(&music).unwrap();
        let mut app = test_app(vec![test_track(1, "first track")]);
        let conn = Connection::open_in_memory().unwrap();
        app.command_mode = true;
        app.command = format!("add {}/Mu", parent.path().display());

        app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();

        assert_eq!(
            app.command,
            format!("add {}/MusicRoot/", parent.path().display())
        );
    }

    #[test]
    fn tab_completes_active_roots_for_remove() {
        let data_dir = tempdir().unwrap();
        let library_dir = tempdir().unwrap();
        let conn = db::open(&data_dir.path().join("gmus.sqlite3")).unwrap();
        db::upsert_library_root(&conn, library_dir.path()).unwrap();
        let root = library_dir.path().to_string_lossy();
        let prefix_len = root.len().saturating_sub(2);
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.command_mode = true;
        app.command = format!("remove {}", &root[..prefix_len]);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.command, format!("remove {root} "));
    }

    #[test]
    fn expired_transient_status_clears_on_tick() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.transient_status = Some(TransientStatus {
            text: "repeat on".to_string(),
            until: Instant::now() - Duration::from_secs(1),
        });

        assert!(app.expire_transient_status());
        assert!(app.transient_status.is_none());
    }

    #[test]
    fn playback_bar_scales_down_with_width() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 50_000,
            listened_ms: 0,
        });
        app.suspended_position_ms = Some(50_000);

        let text = line_text(&playback_line(&app, 44));

        assert!(text.contains("[==--]"));
    }

    #[test]
    fn failed_seek_updates_message_without_crashing() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.player = Box::new(FailingSeekPlayer);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 197_500,
            listened_ms: 0,
        });

        app.seek_relative(5).unwrap();

        assert!(app.message.contains("seek failed"));
        assert!(app.message.contains("decoder refused seek"));
    }

    #[test]
    fn filter_line_has_its_own_prompt() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.filter_mode = true;
        app.filter = "beat".to_string();
        let line = filter_line(&app, 40);

        assert_eq!(line_text(&line), " filter: beat_");
        assert_eq!(
            line.spans[1].style,
            Style::default().fg(Color::White).bg(Color::Blue)
        );
        assert_eq!(
            line.spans[2].style,
            Style::default().fg(Color::White).bg(Color::Blue)
        );
        assert!(!line_text(&playback_line(&app, 80)).contains("filter:"));
    }

    #[test]
    fn filter_line_uses_gray_placeholder_and_persists_for_active_filter() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.filter_mode = true;
        let placeholder = filter_line(&app, 40);

        assert_eq!(line_text(&placeholder), " filter: none_");
        assert_eq!(
            placeholder.spans[2].style,
            Style::default().fg(Color::Gray).bg(Color::Blue)
        );

        app.filter_mode = false;
        app.filter = "beat".to_string();

        assert!(app.filter_bar_visible());
        let active_filter = filter_line(&app, 40);
        assert_eq!(line_text(&active_filter), " filter: beat");
        assert_eq!(
            active_filter.spans[1].style,
            Style::default().fg(Color::Black).bg(Color::White)
        );
    }

    #[test]
    fn fielded_filter_matches_metadata_ranges_and_roots() {
        let mut ambient = test_track(1, "quiet one");
        ambient.genre = Some("Ambient".to_string());
        ambient.album_year = Some(2018);
        ambient.release_date = Some("2018-05-11".to_string());
        ambient.library_root = Some("/tmp/Instrumental".to_string());

        let mut rock = test_track(2, "loud one");
        rock.genre = Some("Rock".to_string());
        rock.album_year = Some(2024);
        rock.library_root = Some("/tmp/Vocal".to_string());

        let mut app = test_app(vec![ambient, rock]);
        app.filter = "genre:ambient year:2010..2020 root:instrumental".to_string();
        app.sync_selection();

        assert_eq!(app.playback_sequence_indices(), vec![0]);
    }

    #[test]
    fn fielded_filter_supports_quoted_values_negation_booleans_and_counts() {
        let mut wanted = test_track(1, "wanted track");
        wanted.artist = Some("Other Artist".to_string());
        wanted.genre = Some("Ambient".to_string());
        wanted.play_count = 6;
        wanted.compilation = false;

        let mut skipped = test_track(2, "skipped track");
        skipped.artist = Some("Other Artist".to_string());
        skipped.genre = Some("Podcast".to_string());
        skipped.play_count = 10;
        skipped.compilation = true;

        let mut app = test_app(vec![wanted, skipped]);
        app.filter =
            "artist:\"Other Artist\" -genre:podcast compilation:false plays:>5".to_string();
        app.sync_selection();

        assert_eq!(app.playback_sequence_indices(), vec![0]);
    }

    #[test]
    fn unknown_filter_field_shows_hint_and_matches_nothing() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.filter = "mood:blue".to_string();
        app.sync_selection();

        let query = FilterQuery::parse(&app.filter);

        assert_eq!(app.playback_sequence_indices(), Vec::<usize>::new());
        assert_eq!(query.warning(), Some("unknown filter field: mood"));
    }

    #[test]
    fn filter_info_pane_hints_fields_while_typing() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.filter_mode = true;
        app.filter = "genre:ambient".to_string();

        let text = lines_text(&filter_info_lines(&app, 80, 8));

        assert!(app.info_area_visible());
        assert_eq!(command_info_title(&app), "Filter");
        assert!(text.contains("fields: title artist album"));
        assert!(text.contains("examples: genre:ambient year:2010..2020"));
    }

    #[test]
    fn album_header_shows_year_and_right_aligned_duration() {
        let line = album_header_line("Album", Some(2018), 100_000, 24);
        let text = line_text(&line);

        assert_eq!(display_width(&text), 24);
        assert!(text.starts_with("Album"));
        assert!(text.contains("--------"));
        assert!(text.ends_with("2018 1:40"));
        assert_eq!(
            line.spans[1].style,
            Style::default().fg(Color::LightMagenta)
        );
    }

    #[test]
    fn track_line_right_aligns_duration() {
        let app = test_app(vec![test_track(1, "first track")]);
        let line = track_line(&app, 0, false, 32);
        let text = line_text(&line);

        assert_eq!(display_width(&text), 32);
        assert!(text.starts_with("  01. first track"));
        assert!(text.ends_with("1:40"));
    }

    #[test]
    fn single_disc_albums_hide_disc_number() {
        let mut track = test_track(1, "first track");
        track.disc_number = Some(1);
        let app = test_app(vec![track]);
        let line = track_line(&app, 0, false, 32);
        let text = line_text(&line);

        assert!(text.starts_with("  01. first track"));
        assert!(!text.contains("1.01."));
    }

    #[test]
    fn multi_disc_albums_add_divider_and_show_disc_numbers() {
        let mut disc_one = test_track(1, "disc one track");
        disc_one.disc_number = Some(1);
        let mut disc_two = test_track(2, "disc two track");
        disc_two.disc_number = Some(2);
        disc_two.track_number = Some(1);
        let app = test_app(vec![disc_one, disc_two]);

        assert!(matches!(
            app.track_rows().get(1),
            Some(TrackRow::Track {
                show_disc_number: true,
                ..
            })
        ));
        assert!(matches!(
            app.track_rows().get(2),
            Some(TrackRow::DiscDivider {
                disc_number: Some(2)
            })
        ));
        let divider = match app.track_rows().get(2) {
            Some(TrackRow::DiscDivider { disc_number }) => disc_divider_line(*disc_number, 24),
            row => panic!("expected disc divider, got {row:?}"),
        };
        assert_eq!(divider.spans[0].style, Style::default().fg(Color::DarkGray));

        let line = track_line(&app, 1, true, 40);
        assert!(line_text(&line).starts_with("  2.01. disc two track"));
    }

    #[test]
    fn album_headers_keep_scanned_years() {
        let app = test_app(vec![test_track(1, "first track")]);

        match app.track_rows().first() {
            Some(TrackRow::AlbumHeader { album_year, .. }) => {
                assert_eq!(*album_year, Some(2018));
            }
            row => panic!("expected album header, got {row:?}"),
        }
    }

    #[test]
    fn tab_confirms_filter_and_focuses_library() {
        let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
        let conn = Connection::open_in_memory().unwrap();
        app.focus = FocusPane::Tracks;
        app.filter_mode = true;
        app.filter = "keep".to_string();

        let should_quit = app
            .handle_key(&conn, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();

        assert!(!should_quit);
        assert!(!app.filter_mode);
        assert_eq!(app.focus, FocusPane::Tree);
        assert_eq!(app.selected_tree, 0);
        assert_eq!(app.selected_track_row, 1);
        assert_eq!(app.playback_sequence_indices(), &[0]);
    }

    #[test]
    fn enter_confirms_filter_and_focuses_library() {
        let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
        let conn = Connection::open_in_memory().unwrap();
        app.focus = FocusPane::Tracks;
        app.filter_mode = true;
        app.filter = "keep".to_string();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        assert!(!app.filter_mode);
        assert_eq!(app.focus, FocusPane::Tree);
        assert_eq!(app.playback_sequence_indices(), &[0]);
    }

    #[test]
    fn escape_clears_filter_entry() {
        let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
        let conn = Connection::open_in_memory().unwrap();
        app.filter_mode = true;
        app.filter = "keep".to_string();
        app.sync_selection();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();

        assert!(!app.filter_mode);
        assert!(app.filter.is_empty());
        assert_eq!(app.message, "filter cleared");
        assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    }

    #[test]
    fn escape_clears_active_filter_outside_filter_entry() {
        let mut app = test_app(vec![test_track(1, "keep one"), test_track(2, "skip this")]);
        let conn = Connection::open_in_memory().unwrap();
        app.filter = "keep".to_string();
        app.sync_selection();

        app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();

        assert!(!app.filter_mode);
        assert!(app.filter.is_empty());
        assert_eq!(app.message, "filter cleared");
        assert_eq!(app.playback_sequence_indices(), &[0, 1]);
    }

    #[test]
    fn escape_preserves_valid_selection_when_clearing_filter() {
        let mut other_artist = test_track(2, "other track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
        let conn = Connection::open_in_memory().unwrap();
        app.filter = "other".to_string();
        app.sync_selection();
        assert_eq!(
            app.selected_tree_entry().map(TreeEntry::artist),
            Some("Other Artist")
        );
        assert_eq!(app.selected_playable_track_index(), Some(1));

        app.handle_key(&conn, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();

        assert!(app.filter.is_empty());
        assert_eq!(
            app.selected_tree_entry().map(TreeEntry::artist),
            Some("Other Artist")
        );
        assert_eq!(app.selected_playable_track_index(), Some(1));
    }

    #[test]
    fn track_pane_selection_skips_album_headers() {
        let mut second_album = test_track(3, "second album track");
        second_album.album = Some("Another Album".to_string());
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
            second_album,
        ]);
        app.sync_selection();

        assert_eq!(app.selected_track_row, 1);
        app.focus = FocusPane::Tracks;
        app.move_down();
        assert_eq!(app.selected_track_row, 2);
        app.move_down();
        assert_eq!(app.selected_track_row, 4);
        app.move_up();
        assert_eq!(app.selected_track_row, 2);
    }

    #[test]
    fn mouse_scroll_moves_tree_pane_without_changing_focus() {
        let mut tracks = Vec::new();
        for id in 1..=6 {
            let mut track = test_track(id, &format!("track {id}"));
            track.artist = Some(format!("Artist {id}"));
            track.album_artist = track.artist.clone();
            tracks.push(track);
        }
        let mut app = test_app(tracks);
        app.focus = FocusPane::Tracks;

        let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 1), 100, 30);

        assert!(handled);
        assert_eq!(app.focus, FocusPane::Tracks);
        assert_eq!(app.selected_tree, 1);
        assert_eq!(app.selected_track_row, 1);
    }

    #[test]
    fn mouse_scroll_moves_track_pane_and_skips_album_headers() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
            test_track(3, "third track"),
            test_track(4, "fourth track"),
        ]);

        let handled = app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 60, 10), 100, 30);

        assert!(handled);
        assert_eq!(app.selected_track_row, 2);
    }

    #[test]
    fn mouse_scroll_ignores_bottom_status_area_and_filter_mode() {
        let mut app = test_app(vec![test_track(1, "first track")]);

        assert!(!app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 28), 100, 30,));
        app.filter_mode = true;
        assert!(!app.handle_mouse(mouse_event(MouseEventKind::ScrollDown, 1, 1), 100, 30,));
    }

    #[test]
    fn narrow_mouse_hit_testing_uses_stacked_panes() {
        assert_eq!(
            mouse_pane(10, 1, 74, 30, 2, false, false),
            Some(FocusPane::Tree)
        );
        assert_eq!(
            mouse_pane(10, 20, 74, 30, 2, false, false),
            Some(FocusPane::Tracks)
        );
        assert_eq!(mouse_pane(10, 28, 74, 30, 2, false, false), None);
    }

    #[test]
    fn wide_mouse_hit_testing_uses_split_panes() {
        assert_eq!(
            mouse_pane(10, 20, 100, 30, 2, false, false),
            Some(FocusPane::Tree)
        );
        assert_eq!(
            mouse_pane(60, 20, 100, 30, 2, false, false),
            Some(FocusPane::Tracks)
        );
        assert_eq!(
            mouse_pane(90, 20, 100, 30, 2, false, false),
            Some(FocusPane::Tracks)
        );
    }

    #[test]
    fn mouse_hit_testing_ignores_bottom_info_and_input_rows() {
        assert_eq!(
            mouse_pane(60, 5, 100, 30, 2, true, true),
            Some(FocusPane::Tracks)
        );
        assert_eq!(
            mouse_pane(10, 12, 100, 30, 2, true, true),
            Some(FocusPane::Tree)
        );
        assert_eq!(mouse_pane(60, 15, 100, 30, 2, true, true), None);
        assert_eq!(mouse_pane(60, 20, 100, 30, 2, true, true), None);
        assert_eq!(mouse_pane(10, 28, 100, 30, 2, true, true), None);
    }

    #[test]
    fn render_keeps_tree_selection_padded_from_bottom_when_possible() {
        let mut tracks = Vec::new();
        for id in 1..=20 {
            let mut track = test_track(id, &format!("track {id}"));
            track.artist = Some(format!("Artist {id:02}"));
            track.album_artist = track.artist.clone();
            tracks.push(track);
        }
        let mut app = test_app(tracks);
        app.selected_tree = 10;
        app.sync_selection();
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &mut app)).unwrap();

        assert!(app.tree_state.offset() > 0);
        assert!(app.selected_tree - app.tree_state.offset() <= 4);
    }

    #[test]
    fn inactive_pane_selection_is_visible() {
        assert_eq!(
            pane_highlight_style(true),
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(
            pane_highlight_style(false),
            Style::default().bg(Color::White).fg(Color::Black)
        );
    }

    #[test]
    fn command_and_filter_focus_make_both_pane_selections_inactive() {
        let mut app = test_app(vec![test_track(1, "first track")]);

        assert!(pane_active(&app, FocusPane::Tree));
        assert!(!pane_active(&app, FocusPane::Tracks));

        app.command_mode = true;
        assert!(!pane_active(&app, FocusPane::Tree));
        assert!(!pane_active(&app, FocusPane::Tracks));

        app.command_mode = false;
        app.focus = FocusPane::Tracks;
        assert!(!pane_active(&app, FocusPane::Tree));
        assert!(pane_active(&app, FocusPane::Tracks));

        app.filter_mode = true;
        assert!(!pane_active(&app, FocusPane::Tree));
        assert!(!pane_active(&app, FocusPane::Tracks));

        app.filter_mode = false;
        app.command_focus = true;
        assert!(!pane_active(&app, FocusPane::Tree));
        assert!(!pane_active(&app, FocusPane::Tracks));
    }

    #[test]
    fn tab_keeps_both_pane_selections() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        app.focus = FocusPane::Tracks;
        app.selected_tree = 0;
        app.selected_track_row = 2;
        app.apply_selection_state();

        app.toggle_focus();

        assert_eq!(app.focus, FocusPane::Tree);
        assert_eq!(app.selected_tree, 0);
        assert_eq!(app.selected_track_row, 2);
        assert_eq!(app.tree_state.selected(), Some(0));
        assert_eq!(app.track_state.selected(), Some(2));
    }

    #[test]
    fn changing_tree_selection_resets_track_selection() {
        let mut second_artist = test_track(2, "second artist track");
        second_artist.artist = Some("Other Artist".to_string());
        second_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), second_artist]);
        app.focus = FocusPane::Tracks;
        app.selected_track_row = 1;
        app.toggle_focus();

        app.move_down();

        assert_eq!(app.focus, FocusPane::Tree);
        assert_eq!(app.selected_tree, 1);
        assert_eq!(app.selected_track_row, 1);
        assert_eq!(app.track_state.selected(), Some(1));
    }

    #[test]
    fn current_tree_marker_uses_artist_when_collapsed() {
        let mut second_album = test_track(2, "second album track");
        second_album.album = Some("Another Album".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), second_album]);
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        assert!(app.tree_entry_is_current(&app.tree_entries()[0]));
    }

    #[test]
    fn current_tree_marker_uses_album_when_artist_expanded() {
        let mut second_album = test_track(2, "second album track");
        second_album.album = Some("Another Album".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), second_album]);
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });
        app.expanded_artists.insert("Artist".to_string());
        app.sync_selection();

        assert!(!app.tree_entry_is_current(&app.tree_entries()[0]));
        assert!(!app.tree_entry_is_current(&app.tree_entries()[1]));
        assert!(app.tree_entry_is_current(&app.tree_entries()[2]));
    }

    #[test]
    fn compilations_artist_appears_first_and_preserves_normal_artist() {
        let mut compilation = test_track(1, "compilation track");
        compilation.compilation = true;
        compilation.artist = Some("Contributing Artist".to_string());
        compilation.album_artist = Some("Contributing Artist".to_string());
        let mut app = test_app(vec![compilation]);

        assert!(matches!(
            app.tree_entries().first(),
            Some(TreeEntry::Compilation)
        ));
        assert!(app.tree_entries().iter().any(|entry| {
            matches!(
                entry,
                TreeEntry::Artist { artist } if artist == "Contributing Artist"
            )
        }));

        let artist_position = app
            .tree_entries()
            .iter()
            .position(|entry| {
                matches!(
                    entry,
                    TreeEntry::Artist { artist } if artist == "Contributing Artist"
                )
            })
            .unwrap();
        app.selected_tree = artist_position;
        app.sync_selection();

        assert_eq!(app.selected_scope_tracks().len(), 1);
    }

    #[test]
    fn compilations_entry_expands_to_albums() {
        let mut first = test_track(1, "first compilation track");
        first.compilation = true;
        first.album = Some("First Collection".to_string());
        let mut second = test_track(2, "second compilation track");
        second.compilation = true;
        second.album = Some("Second Collection".to_string());
        let mut app = test_app(vec![first, second]);

        assert!(matches!(
            app.tree_entries().first(),
            Some(TreeEntry::Compilation)
        ));
        assert!(!app
            .tree_entries()
            .iter()
            .any(|entry| { matches!(entry, TreeEntry::CompilationAlbum { .. }) }));
        assert!(
            line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[+] Compilations")
        );

        app.space_action();

        assert!(app.compilations_expanded);
        assert!(app.tree_entries().iter().any(|entry| {
            matches!(
                entry,
                TreeEntry::CompilationAlbum { album, .. } if album == "First Collection"
            )
        }));
        assert!(app.tree_entries().iter().any(|entry| {
            matches!(
                entry,
                TreeEntry::CompilationAlbum { album, .. } if album == "Second Collection"
            )
        }));
        assert!(
            line_text(&tree_item_line(&app, &app.tree_entries()[0])).contains("[-] Compilations")
        );
    }

    #[test]
    fn expanded_compilation_marks_current_album() {
        let mut first = test_track(1, "first compilation track");
        first.compilation = true;
        first.album = Some("First Collection".to_string());
        let mut second = test_track(2, "second compilation track");
        second.compilation = true;
        second.album = Some("Second Collection".to_string());
        let mut app = test_app(vec![first, second]);
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        assert!(app.tree_entry_is_current(&app.tree_entries()[0]));
        app.space_action();

        assert!(!app.tree_entry_is_current(&app.tree_entries()[0]));
        assert!(app.tree_entries().iter().any(|entry| {
            matches!(
                entry,
                TreeEntry::CompilationAlbum { album, .. }
                    if album == "Second Collection" && app.tree_entry_is_current(entry)
            )
        }));
    }

    #[test]
    fn enter_on_compilations_plays_first_compilation_track() {
        let mut compilation = test_track(2, "compilation track");
        compilation.compilation = true;
        let mut app = test_app(vec![test_track(1, "regular track"), compilation]);
        let conn = Connection::open_in_memory().unwrap();

        app.activate(&conn).unwrap();

        assert!(matches!(
            app.tree_entries().first(),
            Some(TreeEntry::Compilation)
        ));
        assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
    }

    #[test]
    fn compilation_view_groups_albums_across_contributing_artists() {
        let mut first = test_track(1, "esper one");
        first.compilation = true;
        first.artist = Some("Vangelis".to_string());
        first.album = Some("Blade Runner Esper Edition".to_string());
        first.album_artist = Some("Vangelis".to_string());

        let mut other_album = test_track(2, "elsewhere");
        other_album.compilation = true;
        other_album.artist = Some("Another Artist".to_string());
        other_album.album = Some("Other Album".to_string());
        other_album.album_artist = Some("Another Artist".to_string());

        let mut second = test_track(3, "esper two");
        second.compilation = true;
        second.artist = Some("Dialog".to_string());
        second.album = Some("Blade Runner Esper Edition".to_string());
        second.album_artist = Some("Dialog".to_string());
        second.track_number = Some(2);

        let app = test_app(vec![first, other_album, second]);

        let album_headers: Vec<String> = app
            .track_rows()
            .iter()
            .filter_map(|row| match row {
                TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
                _ => None,
            })
            .collect();
        let track_indices: Vec<usize> = app
            .track_rows()
            .iter()
            .filter_map(|row| match row {
                TrackRow::Track { track_index, .. } => Some(*track_index),
                _ => None,
            })
            .collect();

        assert_eq!(
            album_headers,
            vec!["Blade Runner Esper Edition", "Other Album"]
        );
        assert_eq!(track_indices, vec![0, 2, 1]);
    }

    #[test]
    fn compilation_view_merges_same_album_across_roots() {
        let mut vocal = test_track(1, "esper vocal");
        vocal.compilation = true;
        vocal.album = Some("Blade Runner Esper Edition".to_string());
        vocal.library_root = Some("/tmp/Vocal".to_string());

        let mut instrumental = test_track(2, "esper instrumental");
        instrumental.compilation = true;
        instrumental.album = Some("Blade Runner Esper Edition".to_string());
        instrumental.library_root = Some("/tmp/Instrumental".to_string());

        let app = test_app(vec![vocal, instrumental]);

        let album_headers: Vec<String> = app
            .track_rows()
            .iter()
            .filter_map(|row| match row {
                TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
                _ => None,
            })
            .collect();
        let track_indices: Vec<usize> = app
            .track_rows()
            .iter()
            .filter_map(|row| match row {
                TrackRow::Track { track_index, .. } => Some(*track_index),
                _ => None,
            })
            .collect();

        assert_eq!(album_headers, vec!["Blade Runner Esper Edition"]);
        assert_eq!(track_indices, vec![0, 1]);
    }

    #[test]
    fn expanded_artist_merges_same_album_across_roots() {
        let mut vocal = test_track(1, "first side");
        vocal.artist = Some("Moby".to_string());
        vocal.album_artist = Some("Moby".to_string());
        vocal.album = Some("All Visible Objects".to_string());
        vocal.library_root = Some("/tmp/Vocal".to_string());

        let mut instrumental = test_track(2, "second side");
        instrumental.artist = Some("Moby".to_string());
        instrumental.album_artist = Some("Moby".to_string());
        instrumental.album = Some("All Visible Objects".to_string());
        instrumental.library_root = Some("/tmp/Instrumental".to_string());

        let mut app = test_app(vec![vocal, instrumental]);
        app.expanded_artists.insert("Moby".to_string());
        app.sync_selection();

        let album_entries: Vec<String> = app
            .tree_entries()
            .iter()
            .filter_map(|entry| match entry {
                TreeEntry::Album { album, .. } => Some(album.clone()),
                _ => None,
            })
            .collect();
        let album_headers: Vec<String> = app
            .track_rows()
            .iter()
            .filter_map(|row| match row {
                TrackRow::AlbumHeader { album, .. } => Some(album.clone()),
                _ => None,
            })
            .collect();

        assert_eq!(album_entries, vec!["All Visible Objects"]);
        assert_eq!(album_headers, vec!["All Visible Objects"]);
    }

    #[test]
    fn enter_on_artist_plays_first_listed_track() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        let conn = Connection::open_in_memory().unwrap();
        app.sync_selection();

        app.activate(&conn).unwrap();

        assert_eq!(app.current.as_ref().map(|current| current.index), Some(0));
        assert_eq!(app.focus, FocusPane::Tree);
    }

    #[test]
    fn selecting_current_track_does_not_change_focus() {
        let mut app = test_app(vec![
            test_track(1, "first track"),
            test_track(2, "second track"),
        ]);
        app.focus = FocusPane::Tree;

        app.select_track_index(1);

        assert_eq!(app.selected_track_row, 2);
        assert_eq!(app.focus, FocusPane::Tree);
    }

    #[test]
    fn playback_does_not_move_browser_selection() {
        let mut other_artist = test_track(2, "other artist track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
        let conn = Connection::open_in_memory().unwrap();
        app.sync_selection();

        app.play_index(&conn, 1).unwrap();

        assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
        assert_eq!(app.selected_tree, 0);
        assert_eq!(app.selected_track_row, 1);
        assert_eq!(app.focus, FocusPane::Tree);
    }

    #[test]
    fn next_track_does_not_move_browser_selection() {
        let mut other_artist = test_track(2, "other artist track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
        let conn = Connection::open_in_memory().unwrap();
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        app.play_next(&conn).unwrap();

        assert_eq!(app.current.as_ref().map(|current| current.index), Some(1));
        assert_eq!(app.selected_tree, 0);
        assert_eq!(app.selected_track_row, 1);
        assert_eq!(app.focus, FocusPane::Tree);
    }

    #[test]
    fn user_can_select_current_track_explicitly() {
        let mut other_artist = test_track(2, "other artist track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        app.select_current_track();

        assert_eq!(app.selected_tree, 1);
        assert_eq!(app.selected_track_row, 1);
        assert_eq!(app.focus, FocusPane::Tree);
    }

    #[test]
    fn uppercase_i_selects_current_track_after_lowercase_i_toggles_info() {
        let mut other_artist = test_track(2, "other artist track");
        other_artist.artist = Some("Other Artist".to_string());
        other_artist.album_artist = Some("Other Artist".to_string());
        let mut app = test_app(vec![test_track(1, "first track"), other_artist]);
        let conn = Connection::open_in_memory().unwrap();
        app.current = Some(PlayingTrack {
            index: 1,
            track: app.tracks[1].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .unwrap();

        assert!(!app.info_panel_visible);
        assert_eq!(app.selected_tree, 0);

        app.handle_key(&conn, KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE))
            .unwrap();

        assert_eq!(app.selected_tree, 1);
        assert_eq!(app.selected_track_row, 1);
    }

    #[test]
    fn pause_suspends_player_until_resume() {
        let mut app = test_app(vec![test_track(1, "first track")]);
        app.current = Some(PlayingTrack {
            index: 0,
            track: app.tracks[0].clone(),
            last_position_ms: 0,
            listened_ms: 0,
        });
        app.player.play().unwrap();

        app.suspend_current().unwrap();

        assert_eq!(app.logical_state(), PlaybackState::Paused);
        assert_eq!(app.player.state(), PlaybackState::Stopped);
        assert_eq!(app.suspended_position_ms, Some(0));

        app.resume_current().unwrap();

        assert_eq!(app.logical_state(), PlaybackState::Playing);
        assert_eq!(app.suspended_position_ms, None);
    }

    fn test_app(tracks: Vec<LibraryTrack>) -> App {
        let mut app = App {
            paths: test_paths(),
            tracks,
            view: ViewCache::default(),
            tree_state: ListState::default(),
            track_state: ListState::default(),
            selected_tree: 0,
            selected_track_row: 0,
            expanded_artists: HashSet::new(),
            compilations_expanded: false,
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
            pending_command: None,
            info_panel_visible: true,
            play_target: PlayTarget::Library,
            continuous: true,
            repeat: false,
            shuffle: false,
            shuffle_seed: 0x476d_7573_2026_0528,
            shuffle_scope: Vec::new(),
            shuffle_order: Vec::new(),
            player: Box::new(NullPlayer::default()),
            media_session: Box::new(NoopMediaSession),
            current: None,
            suspended_position_ms: None,
            last_media_state: None,
            last_media_position_s: None,
            transient_status: None,
            message: String::new(),
        };
        app.rebuild_search_cache();
        app.sync_selection();
        app
    }

    fn test_paths() -> AppPaths {
        AppPaths {
            data_dir: PathBuf::from("/tmp/gmus-test"),
            db_path: PathBuf::from("/tmp/gmus-test/gmus.sqlite3"),
            art_dir: PathBuf::from("/tmp/gmus-test/art"),
        }
    }

    fn test_track(id: i64, title: &str) -> LibraryTrack {
        LibraryTrack {
            media_item_id: id,
            location_id: id,
            path: format!("/tmp/{title}.flac"),
            library_root: None,
            title: Some(title.to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: None,
            album_year: Some(2018),
            release_date: Some("2018-05-11".to_string()),
            composer: None,
            genre: None,
            cover_path: None,
            track_number: Some(id),
            track_total: Some(10),
            disc_number: None,
            disc_total: None,
            duration_ms: Some(100_000),
            compilation: false,
            play_count: 0,
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn lines_text(lines: &[Line<'_>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    struct FailingSeekPlayer;

    impl PlayerBackend for FailingSeekPlayer {
        fn load(&mut self, _path: &Path) -> Result<()> {
            Ok(())
        }

        fn play(&mut self) -> Result<()> {
            Ok(())
        }

        fn pause(&mut self) -> Result<()> {
            Ok(())
        }

        fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        fn seek(&mut self, _position: Duration) -> Result<()> {
            anyhow::bail!("decoder refused seek")
        }

        fn sleep_until_end(&self) {}

        fn position(&self) -> Duration {
            Duration::from_millis(197_500)
        }

        fn is_finished(&self) -> bool {
            false
        }

        fn state(&self) -> PlaybackState {
            PlaybackState::Playing
        }
    }
}
