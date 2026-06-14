use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::Terminal;
use tempfile::tempdir;

use super::filter::FilterQuery;
use super::formatting::display_width;
use super::keymap::{keymap_lines, keymap_row_for_action, KeyAction};
use super::lines::{
    album_header_line, command_help_lines, command_info_lines, command_info_title,
    disc_divider_line, filter_info_lines, filter_line, input_line, metadata_lines,
    now_playing_line, now_playing_row_style, pane_active, pane_highlight_style, playback_line,
    playlist_entry_text, playlist_header_line, playlist_track_line, rate_info_lines, rate_line,
    track_line, tree_item_line,
};
use super::mouse::{mouse_pane, MouseLayout};
use super::playlist::PlaylistCacheEntry;
use super::renderer::{render, render_playlist_info_pane};
use super::*;
use crate::integration::{
    Integration, IntegrationCommand, IntegrationEvent, NoopIntegration, TrackSnapshot,
};
use crate::player::{NullPlayer, PlaybackState};

mod browser;
mod command;
mod filter;
mod keymap;
mod persistence;
mod playback;
mod playlist;
mod presentation;

fn test_app(tracks: Vec<LibraryTrack>) -> App {
    let mut app = App {
        paths: test_paths(),
        tracks,
        playlists: Vec::new(),
        playlist_cache: PlaylistCache::default(),
        view: ViewCache::default(),
        tree_state: ListState::default(),
        track_state: ListState::default(),
        playlist_state: ListState::default(),
        keymap_state: ListState::default(),
        browser: BrowserState::default(),
        management_panel: ManagementPanelState::default(),
        focus: FocusPane::Tree,
        restore_filter: true,
        restore_track: true,
        input: InputState::default(),
        command_output: CommandOutputState::default(),
        key_bindings: HashMap::new(),
        library_job: None,
        layout: LayoutState::default(),
        playback_mode: PlaybackModeState::default(),
        player: Box::new(NullPlayer::default()),
        integration: IntegrationState::new(Box::new(NoopIntegration)),
        current: None,
        suspended_position_ms: None,
        transient_status: None,
        message: String::new(),
    };
    app.rebuild_search_cache();
    app.sync_selection();
    app
}

fn set_playlist_cache(
    app: &mut App,
    playlist_id: i64,
    media_item_ids: Vec<i64>,
    playlist_track_ids: Vec<i64>,
    track_indices: Vec<usize>,
) {
    assert_eq!(media_item_ids.len(), playlist_track_ids.len());
    assert_eq!(media_item_ids.len(), track_indices.len());
    app.playlist_cache.insert(
        playlist_id,
        media_item_ids
            .into_iter()
            .zip(playlist_track_ids)
            .zip(track_indices)
            .map(
                |((media_item_id, playlist_track_id), track_index)| PlaylistCacheEntry {
                    playlist_track_id,
                    media_item_id,
                    track_index: Some(track_index),
                },
            )
            .collect(),
    );
}

fn test_paths() -> AppPaths {
    AppPaths {
        data_dir: PathBuf::from("/tmp/gmus-test"),
        db_path: PathBuf::from("/tmp/gmus-test/gmus.sqlite3"),
        art_dir: PathBuf::from("/tmp/gmus-test/art"),
    }
}

fn test_app_from_db(conn: &Connection) -> App {
    App::new_with_player(conn, &test_paths(), Box::new(NullPlayer::default())).unwrap()
}

fn wait_for_library_job(app: &mut App, conn: &Connection) -> bool {
    for _ in 0..50 {
        if app.poll_library_job(conn).unwrap() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
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

fn test_track_metadata(path: &str, title: &str, track_number: i64) -> crate::media::TrackMetadata {
    crate::media::TrackMetadata {
        path: path.into(),
        file_size: 10,
        modified_at: Some(1),
        title: Some(title.to_string()),
        artist: Some("Artist".to_string()),
        album: Some("Album".to_string()),
        album_artist: None,
        album_year: Some(2018),
        release_date: Some("2018-05-11".to_string()),
        composer: None,
        genre: None,
        track_number: Some(track_number),
        track_total: Some(10),
        disc_number: None,
        disc_total: None,
        duration_ms: Some(100_000),
        compilation: false,
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

fn buffer_row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
    (0..width)
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

fn playlist_text(app: &App) -> String {
    app.view
        .playlist_entries
        .iter()
        .map(|entry| playlist_entry_text(app, entry))
        .collect::<Vec<_>>()
        .join("\n")
}

fn keymap_text(app: &App) -> String {
    lines_text(&keymap_lines(app, 80))
}

fn playback_bar_width(text: &str) -> usize {
    let start = text.find('[').unwrap();
    let end = text[start..].find(']').unwrap() + start;
    display_width(&text[start + 1..end])
}

fn test_conn() -> Connection {
    db::open_in_memory_for_tests().unwrap()
}

fn set_command_input(app: &mut App, command: impl Into<String>) {
    app.input.enter_command();
    app.input.replace_command(command.into());
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
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
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

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
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

struct FailingRatePlayer;

impl PlayerBackend for FailingRatePlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
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
        Ok(())
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        anyhow::bail!("decoder refused rate change")
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::ZERO
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn state(&self) -> PlaybackState {
        PlaybackState::Stopped
    }
}

struct OutputFailedPlayer;

impl PlayerBackend for OutputFailedPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        anyhow::bail!("no audio output available")
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
        Ok(())
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::from_millis(50_000)
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn output_failed(&self) -> bool {
        true
    }

    fn state(&self) -> PlaybackState {
        PlaybackState::Stopped
    }
}

struct StalledOutputPlayer {
    playing: bool,
}

impl PlayerBackend for StalledOutputPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        Ok(())
    }

    fn play(&mut self) -> Result<()> {
        self.playing = true;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        Ok(())
    }

    fn seek(&mut self, _position: Duration) -> Result<()> {
        Ok(())
    }

    fn set_rate(&mut self, _rate: f32) -> Result<()> {
        Ok(())
    }

    fn rate(&self) -> f32 {
        1.0
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        Duration::from_millis(50_000)
    }

    fn is_finished(&self) -> bool {
        false
    }

    fn state(&self) -> PlaybackState {
        if self.playing {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        }
    }
}

struct FailingIntegration;

impl Integration for FailingIntegration {
    fn next_command(&mut self) -> Option<IntegrationCommand> {
        None
    }

    fn publish_event(&mut self, _event: &IntegrationEvent) -> Result<()> {
        anyhow::bail!("integration unavailable")
    }
}

struct RecordingIntegration {
    events: Rc<RefCell<Vec<IntegrationEvent>>>,
}

impl Integration for RecordingIntegration {
    fn next_command(&mut self) -> Option<IntegrationCommand> {
        None
    }

    fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
        self.events.borrow_mut().push(event.clone());
        Ok(())
    }
}
