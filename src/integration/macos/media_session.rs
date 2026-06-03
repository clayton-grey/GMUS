use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;

use crate::integration::{IntegrationCommand, PlaybackSnapshot, TrackSnapshot};
use crate::player::PlaybackState;

pub(super) struct MediaSession {
    controls: souvlaki::MediaControls,
    receiver: Receiver<IntegrationCommand>,
}

impl MediaSession {
    pub(super) fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let config = souvlaki::PlatformConfig {
            display_name: "GMUS",
            dbus_name: "gmus",
            hwnd: None,
        };
        let mut controls = souvlaki::MediaControls::new(config)?;
        controls.attach(move |event| {
            if let Some(command) = map_event(event) {
                let _ = sender.send(command);
            }
        })?;
        Ok(Self { controls, receiver })
    }

    pub(super) fn next_command(&mut self) -> Option<IntegrationCommand> {
        self.receiver.try_recv().ok()
    }

    pub(super) fn set_now_playing(&mut self, track: &TrackSnapshot) -> Result<()> {
        let cover_url = track.artwork_path.as_deref().map(file_url);
        self.controls.set_metadata(souvlaki::MediaMetadata {
            title: track.title.as_deref(),
            album: track.album.as_deref(),
            artist: track.artist.as_deref(),
            cover_url: cover_url.as_deref(),
            duration: track
                .duration_ms
                .and_then(|value| u64::try_from(value).ok())
                .map(Duration::from_millis),
        })?;
        Ok(())
    }

    pub(super) fn set_playback_state(&mut self, playback: PlaybackSnapshot) -> Result<()> {
        let progress = Some(souvlaki::MediaPosition(Duration::from_millis(
            playback.position_ms.max(0) as u64,
        )));
        let playback = match playback.state {
            PlaybackState::Stopped => souvlaki::MediaPlayback::Stopped,
            PlaybackState::Paused => souvlaki::MediaPlayback::Paused { progress },
            PlaybackState::Playing => souvlaki::MediaPlayback::Playing { progress },
        };
        self.controls.set_playback(playback)?;
        Ok(())
    }
}

fn file_url(path: &std::path::Path) -> String {
    format!("file://{}", percent_encode_path(path))
}

fn map_event(event: souvlaki::MediaControlEvent) -> Option<IntegrationCommand> {
    match event {
        souvlaki::MediaControlEvent::Play => Some(IntegrationCommand::Play),
        souvlaki::MediaControlEvent::Pause => Some(IntegrationCommand::Pause),
        souvlaki::MediaControlEvent::Toggle => Some(IntegrationCommand::Toggle),
        souvlaki::MediaControlEvent::Stop => Some(IntegrationCommand::Stop),
        souvlaki::MediaControlEvent::Next => Some(IntegrationCommand::Next),
        souvlaki::MediaControlEvent::Previous => Some(IntegrationCommand::Previous),
        souvlaki::MediaControlEvent::SetPosition(position) => {
            Some(IntegrationCommand::SeekTo(position.0.as_millis() as i64))
        }
        _ => None,
    }
}

fn percent_encode_path(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    let mut out = String::with_capacity(path.len());
    for byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::percent_encode_path;

    #[test]
    fn percent_encode_path_preserves_path_separators() {
        assert_eq!(
            percent_encode_path(Path::new("/tmp/cover art#.jpg")),
            "/tmp/cover%20art%23.jpg"
        );
    }
}
