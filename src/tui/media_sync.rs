use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::media_session::{MediaCommand, NowPlaying};

use super::App;

impl App {
    pub(super) fn handle_media_commands(&mut self, conn: &Connection) -> Result<bool> {
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

    pub(super) fn publish_now_playing(&mut self) {
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
        match self.media_session.set_now_playing(&now_playing) {
            Ok(()) => self.media_session_error_reported = false,
            Err(error) => {
                self.report_media_session_error(format!("media metadata unavailable: {error:#}"));
            }
        }
    }

    pub(super) fn sync_media_playback(&mut self, force: bool) {
        let state = self.logical_state();
        let position_ms = self.current_position_ms();
        let position_s = position_ms / 1000;
        if !force
            && self.last_media_state == Some(state)
            && self.last_media_position_s == Some(position_s)
        {
            return;
        }

        match self.media_session.set_playback_state(state, position_ms) {
            Ok(()) => {
                self.media_session_error_reported = false;
                self.last_media_state = Some(state);
                self.last_media_position_s = Some(position_s);
            }
            Err(error) => {
                self.report_media_session_error(format!("media controls unavailable: {error:#}"));
            }
        }
    }

    fn report_media_session_error(&mut self, message: String) {
        if !self.media_session_error_reported {
            self.message = message;
            self.media_session_error_reported = true;
        }
    }
}
