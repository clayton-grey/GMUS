use std::path::PathBuf;

use anyhow::Result;

use crate::player::PlaybackState;

#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
mod macos;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSnapshot {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub artwork_path: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    pub state: PlaybackState,
    pub position_ms: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationEvent {
    TrackChanged(TrackSnapshot),
    Playback(PlaybackSnapshot),
    TrackNotificationsVisible(bool),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    SeekTo(i64),
}

#[allow(dead_code)]
pub trait Integration {
    fn tick(&mut self) {}
    fn next_command(&mut self) -> Option<IntegrationCommand>;
    fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()>;
}

pub fn default_integration() -> Box<dyn Integration> {
    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    {
        Box::<macos::LazyMacIntegration>::default()
    }

    #[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
    {
        Box::new(NoopIntegration)
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct NoopIntegration;

impl Integration for NoopIntegration {
    fn next_command(&mut self) -> Option<IntegrationCommand> {
        None
    }

    fn publish_event(&mut self, _event: &IntegrationEvent) -> Result<()> {
        Ok(())
    }
}
