use std::time::{Duration, Instant};

use crate::player::PlaybackState;

use super::App;

const ACTIVE_TICK: Duration = Duration::from_millis(1_000);
const MEDIA_IDLE_TICK: Duration = Duration::from_millis(1_000);
const STOPPED_TICK: Duration = Duration::from_secs(60);
const TRANSIENT_STATUS_DURATION: Duration = Duration::from_secs(1);

pub(super) struct TransientStatus {
    pub(super) text: String,
    pub(super) until: Instant,
}

impl App {
    pub(super) fn toggle_info_panel(&mut self) {
        let visible = self.layout.toggle_info_panel();
        self.message = format!("info panel {}", if visible { "shown" } else { "hidden" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_play_target(&mut self) {
        let target = self.playback_mode.advance_target();
        self.message = format!("play target: {}", target.label());
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_continuous(&mut self) {
        let continuous = self.playback_mode.toggle_continuous();
        self.message = format!("continuous {}", if continuous { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_repeat(&mut self) {
        let repeat = self.playback_mode.toggle_repeat();
        self.message = format!("repeat {}", if repeat { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_shuffle(&mut self) {
        let shuffle = self.playback_mode.toggle_shuffle();
        self.message = format!("shuffle {}", if shuffle { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn show_transient_status(&mut self, text: String) {
        self.transient_status = Some(TransientStatus {
            text,
            until: Instant::now() + TRANSIENT_STATUS_DURATION,
        });
    }

    pub(super) fn active_transient_status(&self) -> Option<&str> {
        self.transient_status
            .as_ref()
            .filter(|status| Instant::now() < status.until)
            .map(|status| status.text.as_str())
    }

    pub(super) fn expire_transient_status(&mut self) -> bool {
        if self
            .transient_status
            .as_ref()
            .is_some_and(|status| Instant::now() >= status.until)
        {
            self.transient_status = None;
            true
        } else {
            false
        }
    }

    pub(super) fn tick_interval(&self) -> std::time::Duration {
        if self.transient_status.is_some() {
            return MEDIA_IDLE_TICK;
        }
        match self.logical_state() {
            PlaybackState::Playing => ACTIVE_TICK.div_f32(self.player.rate()),
            PlaybackState::Paused => MEDIA_IDLE_TICK,
            PlaybackState::Stopped => STOPPED_TICK,
        }
    }
}
