use anyhow::Result;
use rusqlite::Connection;

use crate::integration::{IntegrationCommand, IntegrationEvent, PlaybackSnapshot, TrackSnapshot};

use super::App;

impl App {
    pub(super) fn handle_integration_commands(&mut self, conn: &Connection) -> Result<bool> {
        let mut handled = false;
        while let Some(command) = self.integration.next_command() {
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
            artwork_path: current.track.cover_path.clone().map(Into::into),
        };
        match self
            .integration
            .publish_event(&IntegrationEvent::TrackChanged(track))
        {
            Ok(()) => self.integration_error_reported = false,
            Err(error) => {
                self.report_integration_error(format!("media metadata unavailable: {error:#}"));
            }
        }
    }

    pub(super) fn sync_integration_playback(&mut self, force: bool) {
        let state = self.logical_state();
        let position_ms = self.current_position_ms();
        let position_s = position_ms / 1000;
        if !force
            && self.last_integration_state == Some(state)
            && self.last_integration_position_s == Some(position_s)
        {
            return;
        }

        let snapshot = PlaybackSnapshot { state, position_ms };
        match self
            .integration
            .publish_event(&IntegrationEvent::Playback(snapshot))
        {
            Ok(()) => {
                self.integration_error_reported = false;
                self.last_integration_state = Some(state);
                self.last_integration_position_s = Some(position_s);
            }
            Err(error) => {
                self.report_integration_error(format!("media controls unavailable: {error:#}"));
            }
        }
    }

    fn report_integration_error(&mut self, message: String) {
        if !self.integration_error_reported {
            self.message = message;
            self.integration_error_reported = true;
        }
    }
}
