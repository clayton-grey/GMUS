use std::path::Path;
use std::time::Duration;

use anyhow::Result;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

#[allow(dead_code)]
pub trait PlayerBackend {
    fn load(&mut self, path: &Path) -> Result<()>;
    fn play(&mut self) -> Result<()>;
    fn pause(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn seek(&mut self, position: Duration) -> Result<()>;
    fn sleep_until_end(&self);
    fn position(&self) -> Duration;
    fn is_finished(&self) -> bool;
    fn state(&self) -> PlaybackState;
}

pub fn default_player_backend() -> Result<Box<dyn PlayerBackend>> {
    #[cfg(feature = "playback-rodio")]
    {
        return Ok(Box::<rodio_backend::LazyRodioPlayer>::default());
    }

    #[cfg(not(feature = "playback-rodio"))]
    {
        anyhow::bail!(
            "GMUS was built without a playback backend; enable the playback-rodio feature"
        );
    }
}

pub fn play_count_threshold_met(duration_ms: Option<i64>, played_ms: i64) -> bool {
    let played_ms = played_ms.max(0);
    if played_ms >= 240_000 {
        return true;
    }

    duration_ms
        .filter(|duration| *duration > 0)
        .map(|duration| played_ms * 2 >= duration)
        .unwrap_or(false)
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct NullPlayer {
    state: PlaybackState,
    position: Duration,
}

impl PlayerBackend for NullPlayer {
    fn load(&mut self, _path: &Path) -> Result<()> {
        self.state = PlaybackState::Stopped;
        self.position = Duration::ZERO;
        Ok(())
    }

    fn play(&mut self) -> Result<()> {
        self.state = PlaybackState::Playing;
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.state = PlaybackState::Paused;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.state = PlaybackState::Stopped;
        self.position = Duration::ZERO;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        self.position = position;
        Ok(())
    }

    fn sleep_until_end(&self) {}

    fn position(&self) -> Duration {
        self.position
    }

    fn is_finished(&self) -> bool {
        true
    }

    fn state(&self) -> PlaybackState {
        self.state
    }
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self::Stopped
    }
}

#[cfg(test)]
mod tests {
    use super::play_count_threshold_met;

    #[test]
    fn counts_half_of_known_duration() {
        assert!(play_count_threshold_met(Some(100_000), 50_000));
        assert!(!play_count_threshold_met(Some(100_000), 49_999));
    }

    #[test]
    fn counts_long_unknown_duration() {
        assert!(play_count_threshold_met(None, 240_000));
        assert!(!play_count_threshold_met(None, 239_999));
    }
}

#[cfg(feature = "playback-rodio")]
mod rodio_backend {
    use std::fs::File;
    use std::path::Path;
    use std::time::Duration;

    use anyhow::{Context, Result};
    use rodio::{
        ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate, Source,
    };

    use super::{PlaybackState, PlayerBackend};

    #[derive(Default)]
    pub struct LazyRodioPlayer {
        inner: Option<RodioPlayer>,
    }

    impl PlayerBackend for LazyRodioPlayer {
        fn load(&mut self, path: &Path) -> Result<()> {
            if let Some(mut inner) = self.inner.take() {
                inner.stop()?;
            }
            self.inner = Some(RodioPlayer::load_path(path)?);
            Ok(())
        }

        fn play(&mut self) -> Result<()> {
            if let Some(inner) = &mut self.inner {
                inner.play()?;
            }
            Ok(())
        }

        fn pause(&mut self) -> Result<()> {
            if let Some(inner) = &mut self.inner {
                inner.pause()?;
            }
            Ok(())
        }

        fn stop(&mut self) -> Result<()> {
            if let Some(mut inner) = self.inner.take() {
                inner.stop()?;
            }
            Ok(())
        }

        fn seek(&mut self, position: Duration) -> Result<()> {
            if let Some(inner) = &mut self.inner {
                inner.seek(position)?;
            }
            Ok(())
        }

        fn sleep_until_end(&self) {
            if let Some(inner) = &self.inner {
                inner.sleep_until_end();
            }
        }

        fn position(&self) -> Duration {
            self.inner
                .as_ref()
                .map(PlayerBackend::position)
                .unwrap_or(Duration::ZERO)
        }

        fn is_finished(&self) -> bool {
            self.inner
                .as_ref()
                .map(PlayerBackend::is_finished)
                .unwrap_or(true)
        }

        fn state(&self) -> PlaybackState {
            self.inner
                .as_ref()
                .map(PlayerBackend::state)
                .unwrap_or(PlaybackState::Stopped)
        }
    }

    pub struct RodioPlayer {
        _sink: MixerDeviceSink,
        player: Player,
        state: PlaybackState,
    }

    impl RodioPlayer {
        fn load_path(path: &Path) -> Result<Self> {
            let file = File::open(path)
                .with_context(|| format!("opening audio file {}", path.display()))?;
            let source = Decoder::try_from(file)
                .with_context(|| format!("decoding audio file {}", path.display()))?;
            let sink = open_sink(source.channels(), source.sample_rate())?;
            let player = Player::connect_new(&sink.mixer());
            player.append(source);
            player.play();
            Ok(Self {
                _sink: sink,
                player,
                state: PlaybackState::Playing,
            })
        }
    }

    impl PlayerBackend for RodioPlayer {
        fn load(&mut self, path: &Path) -> Result<()> {
            self.stop()?;
            *self = Self::load_path(path)?;
            Ok(())
        }

        fn play(&mut self) -> Result<()> {
            self.player.play();
            self.state = PlaybackState::Playing;
            Ok(())
        }

        fn pause(&mut self) -> Result<()> {
            self.player.pause();
            self.state = PlaybackState::Paused;
            Ok(())
        }

        fn stop(&mut self) -> Result<()> {
            self.player.stop();
            self.player.sleep_until_end();
            self.state = PlaybackState::Stopped;
            Ok(())
        }

        fn seek(&mut self, position: Duration) -> Result<()> {
            self.player
                .try_seek(position)
                .with_context(|| format!("seeking to {} ms", position.as_millis()))?;
            Ok(())
        }

        fn sleep_until_end(&self) {
            self.player.sleep_until_end();
        }

        fn position(&self) -> Duration {
            self.player.get_pos()
        }

        fn is_finished(&self) -> bool {
            self.player.empty()
        }

        fn state(&self) -> PlaybackState {
            if self.player.empty() {
                PlaybackState::Stopped
            } else if self.player.is_paused() {
                PlaybackState::Paused
            } else {
                self.state
            }
        }
    }

    fn open_sink(channels: ChannelCount, sample_rate: SampleRate) -> Result<MixerDeviceSink> {
        let mut sink = DeviceSinkBuilder::from_default_device()
            .context("opening the default macOS audio output device")?
            .with_channels(channels)
            .with_sample_rate(sample_rate)
            .open_sink_or_fallback()
            .context("opening a macOS audio output stream")?;
        sink.log_on_drop(false);
        Ok(sink)
    }
}
