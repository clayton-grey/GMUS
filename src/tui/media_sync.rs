#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use std::time::Instant;

use anyhow::Result;
use rusqlite::Connection;

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use crate::art::{self, ArtworkMode};
use crate::integration::{
    self, Integration, IntegrationCommand, IntegrationEvent, PlaybackSnapshot, TrackSnapshot,
};
use crate::player::PlaybackState;

use super::App;

pub(super) struct IntegrationState {
    pub(super) backend: Box<dyn Integration>,
    last_playback_state: Option<PlaybackState>,
    last_position_s: Option<i64>,
    error_reported: bool,
    #[cfg_attr(
        not(all(target_os = "macos", feature = "macos-media-session")),
        allow(dead_code)
    )]
    pub(super) track_notifications_visible: bool,
}

impl IntegrationState {
    pub(super) fn new(backend: Box<dyn Integration>) -> Self {
        Self {
            backend,
            last_playback_state: None,
            last_position_s: None,
            error_reported: false,
            track_notifications_visible: true,
        }
    }
}

impl Default for IntegrationState {
    fn default() -> Self {
        Self::new(integration::default_integration())
    }
}

impl App {
    pub(super) fn handle_integration_commands(&mut self, conn: &Connection) -> Result<bool> {
        let mut handled = false;
        while let Some(command) = self.integration.backend.next_command() {
            handled = true;
            match command {
                IntegrationCommand::Play => {
                    if self.current.is_some() {
                        self.resume_current()?;
                    } else {
                        self.play_selected_row(conn)?;
                    }
                }
                IntegrationCommand::Pause => {
                    self.suspend_current()?;
                }
                IntegrationCommand::Toggle => self.toggle_pause(conn)?,
                IntegrationCommand::Stop => self.stop_current(conn)?,
                IntegrationCommand::Next => self.play_next(conn)?,
                IntegrationCommand::Previous => self.play_previous(conn)?,
                IntegrationCommand::SeekTo(position_ms) => {
                    if self.current.is_some() {
                        self.seek_to(position_ms)?;
                    }
                }
            }
        }
        Ok(handled)
    }

    pub(super) fn publish_track_changed(&mut self) {
        let Some(current) = &self.current else {
            return;
        };
        let track = TrackSnapshot {
            title: Some(current.track.display_title().to_string()),
            artist: current.track.artist.clone(),
            album: current.track.album.clone(),
            duration_ms: current.track.duration_ms,
            artwork_path: self.integration_artwork_path(&current.track),
        };
        match self
            .integration
            .backend
            .publish_event(&IntegrationEvent::TrackChanged(track))
        {
            Ok(()) => self.integration.error_reported = false,
            Err(error) => {
                self.report_integration_error(format!("track integration unavailable: {error:#}"));
            }
        }
    }

    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    fn integration_artwork_path(
        &self,
        track: &crate::db::LibraryTrack,
    ) -> Option<std::path::PathBuf> {
        match ArtworkMode::from_env() {
            ArtworkMode::Cached => track.cover_path.clone(),
            ArtworkMode::OnDemand => {
                // Only the macOS media-session backend consumes artwork paths; no-op
                // integrations should not create cache files during track changes.
                let start = Instant::now();
                let result = art::materialize_cover_for_audio_path(
                    &track.file_path,
                    &self.paths.art_dir,
                    track.media_item_id,
                );
                let elapsed = start.elapsed();
                match result {
                    Ok(path) => {
                        if art::trace_enabled() {
                            eprintln!(
                                "gmus artwork: on-demand resolved={} elapsed_ms={} path={}",
                                path.is_some(),
                                elapsed.as_millis(),
                                track.file_path.display()
                            );
                        }
                        path
                    }
                    Err(error) => {
                        if art::trace_enabled() {
                            eprintln!(
                                "gmus artwork: on-demand failed elapsed_ms={} path={} error={error:#}",
                                elapsed.as_millis(),
                                track.file_path.display()
                            );
                        }
                        None
                    }
                }
            }
        }
    }

    #[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
    fn integration_artwork_path(
        &self,
        _track: &crate::db::LibraryTrack,
    ) -> Option<std::path::PathBuf> {
        None
    }

    pub(super) fn sync_integration_playback(&mut self, force: bool) {
        let state = self.logical_state();
        let position_ms = self.current_position_ms();
        let position_s = position_ms / 1000;
        if !force
            && self.integration.last_playback_state == Some(state)
            && self.integration.last_position_s == Some(position_s)
        {
            return;
        }

        let snapshot = PlaybackSnapshot { state, position_ms };
        match self
            .integration
            .backend
            .publish_event(&IntegrationEvent::Playback(snapshot))
        {
            Ok(()) => {
                self.integration.error_reported = false;
                self.integration.last_playback_state = Some(state);
                self.integration.last_position_s = Some(position_s);
            }
            Err(error) => {
                self.report_integration_error(format!("media controls unavailable: {error:#}"));
            }
        }
    }

    fn report_integration_error(&mut self, message: String) {
        if !self.integration.error_reported {
            self.message = message;
            self.integration.error_reported = true;
        }
    }
}
