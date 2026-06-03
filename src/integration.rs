use std::path::PathBuf;
#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use std::sync::mpsc::{self, Receiver};
#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use std::time::Duration;

use anyhow::Result;

use crate::player::PlaybackState;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlaying {
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
    NowPlaying(NowPlaying),
    Playback(PlaybackSnapshot),
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

#[cfg(target_os = "macos")]
pub mod macos {
    #[cfg(feature = "macos-media-session")]
    use anyhow::Result;

    #[cfg(feature = "macos-media-session")]
    use super::{
        mpsc, Duration, Integration, IntegrationCommand, IntegrationEvent, NowPlaying,
        PlaybackSnapshot, PlaybackState, Receiver,
    };

    #[cfg(feature = "macos-media-session")]
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicyAccessory, NSEventMask,
    };
    #[cfg(feature = "macos-media-session")]
    use cocoa::base::{nil, YES};
    #[cfg(feature = "macos-media-session")]
    use cocoa::foundation::{NSAutoreleasePool, NSDate, NSDefaultRunLoopMode};

    #[cfg(feature = "macos-media-session")]
    #[derive(Default)]
    pub struct LazyMacIntegration {
        inner: Option<MacIntegration>,
        unavailable: bool,
    }

    #[cfg(feature = "macos-media-session")]
    impl LazyMacIntegration {
        fn inner_mut(&mut self) -> Result<&mut MacIntegration> {
            if self.inner.is_none() && !self.unavailable {
                match MacIntegration::new() {
                    Ok(integration) => self.inner = Some(integration),
                    Err(error) => {
                        self.unavailable = true;
                        return Err(error);
                    }
                }
            }
            self.inner.as_mut().ok_or_else(|| {
                anyhow::anyhow!("macOS media controls are unavailable for this process")
            })
        }
    }

    #[cfg(feature = "macos-media-session")]
    impl Integration for LazyMacIntegration {
        fn tick(&mut self) {
            if let Some(inner) = &mut self.inner {
                inner.tick();
            }
        }

        fn next_command(&mut self) -> Option<IntegrationCommand> {
            self.inner.as_mut().and_then(Integration::next_command)
        }

        fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
            if matches!(
                event,
                IntegrationEvent::Playback(PlaybackSnapshot {
                    state: PlaybackState::Stopped,
                    ..
                })
            ) {
                if let Some(inner) = &mut self.inner {
                    inner.publish_event(event)?;
                }
                self.inner = None;
                return Ok(());
            }

            self.inner_mut()?.publish_event(event)
        }
    }

    #[cfg(feature = "macos-media-session")]
    pub struct MacIntegration {
        controls: souvlaki::MediaControls,
        receiver: Receiver<IntegrationCommand>,
        appkit_pump: AppKitPump,
    }

    #[cfg(feature = "macos-media-session")]
    #[allow(dead_code)]
    impl MacIntegration {
        pub fn new() -> Result<Self> {
            let appkit_pump = AppKitPump::new();
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
            Ok(Self {
                controls,
                receiver,
                appkit_pump,
            })
        }
    }

    #[cfg(feature = "macos-media-session")]
    impl Integration for MacIntegration {
        fn tick(&mut self) {
            self.appkit_pump.pump_pending_events();
        }

        fn next_command(&mut self) -> Option<IntegrationCommand> {
            self.receiver.try_recv().ok()
        }

        fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
            match event {
                IntegrationEvent::NowPlaying(now_playing) => self.set_now_playing(now_playing),
                IntegrationEvent::Playback(playback) => self.set_playback_state(*playback),
            }
        }
    }

    #[cfg(feature = "macos-media-session")]
    impl MacIntegration {
        fn set_now_playing(&mut self, now_playing: &NowPlaying) -> Result<()> {
            let cover_url = now_playing.artwork_path.as_deref().map(file_url);
            self.controls.set_metadata(souvlaki::MediaMetadata {
                title: now_playing.title.as_deref(),
                album: now_playing.album.as_deref(),
                artist: now_playing.artist.as_deref(),
                cover_url: cover_url.as_deref(),
                duration: now_playing
                    .duration_ms
                    .and_then(|value| u64::try_from(value).ok())
                    .map(Duration::from_millis),
            })?;
            Ok(())
        }

        fn set_playback_state(&mut self, playback: PlaybackSnapshot) -> Result<()> {
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

    #[cfg(feature = "macos-media-session")]
    fn file_url(path: &std::path::Path) -> String {
        format!("file://{}", percent_encode_path(path))
    }

    #[cfg(feature = "macos-media-session")]
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

    #[cfg(feature = "macos-media-session")]
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

    #[cfg(feature = "macos-media-session")]
    struct AppKitPump;

    #[cfg(feature = "macos-media-session")]
    impl AppKitPump {
        fn new() -> Self {
            unsafe {
                let pool = NSAutoreleasePool::new(nil);
                let app = NSApp();
                let _ = app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
                app.finishLaunching();
                pool.drain();
            }
            Self
        }

        fn pump_pending_events(&self) {
            unsafe {
                let pool = NSAutoreleasePool::new(nil);
                let app = NSApp();
                let until = NSDate::distantPast(nil);
                loop {
                    let event = app.nextEventMatchingMask_untilDate_inMode_dequeue_(
                        NSEventMask::NSAnyEventMask.bits(),
                        until,
                        NSDefaultRunLoopMode,
                        YES,
                    );
                    if event == nil {
                        break;
                    }
                    app.sendEvent_(event);
                }
                pool.drain();
            }
        }
    }

    #[cfg(not(feature = "macos-media-session"))]
    #[allow(dead_code)]
    pub struct MacIntegration;
}
