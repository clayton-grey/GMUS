use std::path::Path;
#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use std::sync::mpsc::{self, Receiver};
#[cfg(all(target_os = "macos", feature = "macos-media-session"))]
use std::time::Duration;

use anyhow::Result;

use crate::player::PlaybackState;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct NowPlaying<'a> {
    pub title: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub duration_ms: Option<i64>,
    pub artwork_path: Option<&'a Path>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    SeekTo(i64),
}

#[allow(dead_code)]
pub trait MediaSession {
    fn tick(&mut self) {}
    fn next_command(&mut self) -> Option<MediaCommand>;
    fn set_now_playing(&mut self, now_playing: &NowPlaying<'_>) -> Result<()>;
    fn set_playback_state(&mut self, state: PlaybackState, position_ms: i64) -> Result<()>;
}

pub fn default_media_session() -> Box<dyn MediaSession> {
    #[cfg(all(target_os = "macos", feature = "macos-media-session"))]
    {
        return Box::<macos::LazyMacMediaSession>::default();
    }

    #[cfg(not(all(target_os = "macos", feature = "macos-media-session")))]
    {
        Box::new(NoopMediaSession)
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct NoopMediaSession;

impl MediaSession for NoopMediaSession {
    fn next_command(&mut self) -> Option<MediaCommand> {
        None
    }

    fn set_now_playing(&mut self, _now_playing: &NowPlaying<'_>) -> Result<()> {
        Ok(())
    }

    fn set_playback_state(&mut self, _state: PlaybackState, _position_ms: i64) -> Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub mod macos {
    #[cfg(feature = "macos-media-session")]
    use anyhow::Result;

    #[cfg(feature = "macos-media-session")]
    use super::{mpsc, Duration, MediaCommand, MediaSession, NowPlaying, PlaybackState, Receiver};

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
    pub struct LazyMacMediaSession {
        inner: Option<MacMediaSession>,
        unavailable: bool,
    }

    #[cfg(feature = "macos-media-session")]
    impl LazyMacMediaSession {
        fn inner_mut(&mut self) -> Result<&mut MacMediaSession> {
            if self.inner.is_none() && !self.unavailable {
                match MacMediaSession::new() {
                    Ok(session) => self.inner = Some(session),
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
    impl MediaSession for LazyMacMediaSession {
        fn tick(&mut self) {
            if let Some(inner) = &mut self.inner {
                inner.tick();
            }
        }

        fn next_command(&mut self) -> Option<MediaCommand> {
            self.inner.as_mut().and_then(MediaSession::next_command)
        }

        fn set_now_playing(&mut self, now_playing: &NowPlaying<'_>) -> Result<()> {
            self.inner_mut()?.set_now_playing(now_playing)
        }

        fn set_playback_state(&mut self, state: PlaybackState, position_ms: i64) -> Result<()> {
            if state == PlaybackState::Stopped {
                if let Some(inner) = &mut self.inner {
                    inner.set_playback_state(state, position_ms)?;
                }
                self.inner = None;
                return Ok(());
            }

            self.inner_mut()?.set_playback_state(state, position_ms)
        }
    }

    #[cfg(feature = "macos-media-session")]
    pub struct MacMediaSession {
        controls: souvlaki::MediaControls,
        receiver: Receiver<MediaCommand>,
        appkit_pump: AppKitPump,
    }

    #[cfg(feature = "macos-media-session")]
    #[allow(dead_code)]
    impl MacMediaSession {
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
    impl MediaSession for MacMediaSession {
        fn tick(&mut self) {
            self.appkit_pump.pump_pending_events();
        }

        fn next_command(&mut self) -> Option<MediaCommand> {
            self.receiver.try_recv().ok()
        }

        fn set_now_playing(&mut self, now_playing: &NowPlaying<'_>) -> Result<()> {
            let cover_url = now_playing.artwork_path.map(file_url);
            self.controls.set_metadata(souvlaki::MediaMetadata {
                title: now_playing.title,
                album: now_playing.album,
                artist: now_playing.artist,
                cover_url: cover_url.as_deref(),
                duration: now_playing
                    .duration_ms
                    .and_then(|value| u64::try_from(value).ok())
                    .map(Duration::from_millis),
            })?;
            Ok(())
        }

        fn set_playback_state(&mut self, state: PlaybackState, position_ms: i64) -> Result<()> {
            let progress = Some(souvlaki::MediaPosition(Duration::from_millis(
                position_ms.max(0) as u64,
            )));
            let playback = match state {
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
    fn map_event(event: souvlaki::MediaControlEvent) -> Option<MediaCommand> {
        match event {
            souvlaki::MediaControlEvent::Play => Some(MediaCommand::Play),
            souvlaki::MediaControlEvent::Pause => Some(MediaCommand::Pause),
            souvlaki::MediaControlEvent::Toggle => Some(MediaCommand::Toggle),
            souvlaki::MediaControlEvent::Stop => Some(MediaCommand::Stop),
            souvlaki::MediaControlEvent::Next => Some(MediaCommand::Next),
            souvlaki::MediaControlEvent::Previous => Some(MediaCommand::Previous),
            souvlaki::MediaControlEvent::SetPosition(position) => {
                Some(MediaCommand::SeekTo(position.0.as_millis() as i64))
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
    pub struct MacMediaSession;
}
