use std::path::Path;
use std::time::Duration;

use anyhow::Result;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

#[allow(dead_code)]
pub trait PlayerBackend {
    fn load_and_play(&mut self, path: &Path) -> Result<()>;
    fn play(&mut self) -> Result<()>;
    fn pause(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
    fn seek(&mut self, position: Duration) -> Result<()>;
    fn set_rate(&mut self, rate: f32) -> Result<()>;
    fn rate(&self) -> f32;
    fn sleep_until_end(&self);
    fn position(&self) -> Duration;
    fn is_finished(&self) -> bool;
    fn state(&self) -> PlaybackState;
}

pub fn default_player_backend() -> Result<Box<dyn PlayerBackend>> {
    #[cfg(feature = "playback-rodio")]
    {
        Ok(Box::<rodio_backend::LazyRodioPlayer>::default())
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
#[derive(Debug)]
pub struct NullPlayer {
    state: PlaybackState,
    position: Duration,
    rate: f32,
}

impl Default for NullPlayer {
    fn default() -> Self {
        Self {
            state: PlaybackState::Stopped,
            position: Duration::ZERO,
            rate: 1.0,
        }
    }
}

impl PlayerBackend for NullPlayer {
    fn load_and_play(&mut self, _path: &Path) -> Result<()> {
        self.state = PlaybackState::Playing;
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

    fn set_rate(&mut self, rate: f32) -> Result<()> {
        self.rate = rate;
        Ok(())
    }

    fn rate(&self) -> f32 {
        self.rate
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

#[cfg(test)]
mod tests {
    use super::{play_count_threshold_met, NullPlayer, PlayerBackend};

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

    #[test]
    fn null_player_remembers_playback_rate() {
        let mut player = NullPlayer::default();

        assert_eq!(player.rate(), 1.0);
        player.set_rate(0.75).unwrap();
        assert_eq!(player.rate(), 0.75);
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

    pub struct LazyRodioPlayer {
        inner: Option<RodioPlayer>,
        rate: f32,
    }

    impl Default for LazyRodioPlayer {
        fn default() -> Self {
            Self {
                inner: None,
                rate: 1.0,
            }
        }
    }

    impl PlayerBackend for LazyRodioPlayer {
        fn load_and_play(&mut self, path: &Path) -> Result<()> {
            if let Some(mut inner) = self.inner.take() {
                inner.stop()?;
            }
            self.inner = Some(RodioPlayer::load_path(path, self.rate)?);
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

        fn set_rate(&mut self, rate: f32) -> Result<()> {
            self.rate = rate;
            if let Some(inner) = &mut self.inner {
                inner.set_rate(rate)?;
            }
            Ok(())
        }

        fn rate(&self) -> f32 {
            self.rate
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
        playback_position_anchor: Duration,
        track_position_anchor: Duration,
        rate: f32,
    }

    impl RodioPlayer {
        fn load_path(path: &Path, rate: f32) -> Result<Self> {
            let file = File::open(path)
                .with_context(|| format!("opening audio file {}", path.display()))?;
            let source = Decoder::try_from(file)
                .with_context(|| format!("decoding audio file {}", path.display()))?;
            let sink = open_sink(source.channels(), source.sample_rate())?;
            let player = Player::connect_new(sink.mixer());
            player.set_speed(rate);
            player.append(source);
            player.play();
            Ok(Self {
                _sink: sink,
                player,
                state: PlaybackState::Playing,
                playback_position_anchor: Duration::ZERO,
                track_position_anchor: Duration::ZERO,
                rate,
            })
        }
    }

    impl PlayerBackend for RodioPlayer {
        fn load_and_play(&mut self, path: &Path) -> Result<()> {
            let rate = self.rate();
            self.stop()?;
            *self = Self::load_path(path, rate)?;
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
            let playback_position = playback_position_for_track_position(position, self.rate);
            self.player
                .try_seek(playback_position)
                .with_context(|| format!("seeking to {} ms", position.as_millis()))?;
            self.playback_position_anchor = playback_position;
            self.track_position_anchor = position;
            Ok(())
        }

        fn set_rate(&mut self, rate: f32) -> Result<()> {
            let track_position = self.position();
            let playback_position = self.player.get_pos();
            self.player.set_speed(rate);
            self.playback_position_anchor = playback_position;
            self.track_position_anchor = track_position;
            self.rate = rate;
            Ok(())
        }

        fn rate(&self) -> f32 {
            self.rate
        }

        fn sleep_until_end(&self) {
            self.player.sleep_until_end();
        }

        fn position(&self) -> Duration {
            track_position_from_playback(
                self.player.get_pos(),
                self.playback_position_anchor,
                self.track_position_anchor,
                self.rate,
            )
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

    fn playback_position_for_track_position(position: Duration, rate: f32) -> Duration {
        position.div_f32(rate)
    }

    fn track_position_from_playback(
        playback_position: Duration,
        playback_position_anchor: Duration,
        track_position_anchor: Duration,
        rate: f32,
    ) -> Duration {
        let elapsed = playback_position.saturating_sub(playback_position_anchor);
        track_position_anchor.saturating_add(elapsed.mul_f32(rate))
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use super::{
            playback_position_for_track_position, track_position_from_playback, LazyRodioPlayer,
            PlayerBackend,
        };

        #[test]
        fn lazy_player_remembers_rate_without_active_track() {
            let mut player = LazyRodioPlayer::default();

            player.set_rate(0.75).unwrap();
            player.stop().unwrap();

            assert_eq!(player.rate(), 0.75);
        }

        #[test]
        fn converts_between_stretched_playback_and_track_timelines() {
            assert_eq!(
                track_position_from_playback(
                    Duration::from_secs(20),
                    Duration::ZERO,
                    Duration::ZERO,
                    0.75,
                ),
                Duration::from_secs(15)
            );
            assert_eq!(
                playback_position_for_track_position(Duration::from_secs(15), 0.75),
                Duration::from_secs(20)
            );
        }

        #[test]
        fn track_timeline_stays_continuous_after_rate_change() {
            assert_eq!(
                track_position_from_playback(
                    Duration::from_secs(28),
                    Duration::from_secs(20),
                    Duration::from_secs(15),
                    0.5,
                ),
                Duration::from_secs(19)
            );
        }
    }
}
