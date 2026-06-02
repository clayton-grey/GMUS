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
        self.info_panel_visible = !self.info_panel_visible;
        self.message = format!(
            "info panel {}",
            if self.info_panel_visible {
                "shown"
            } else {
                "hidden"
            }
        );
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_play_target(&mut self) {
        self.play_target = self.play_target.next();
        self.reset_shuffle_order();
        self.message = format!("play target: {}", self.play_target.label());
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_continuous(&mut self) {
        self.continuous = !self.continuous;
        self.message = format!("continuous {}", if self.continuous { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_repeat(&mut self) {
        self.repeat = !self.repeat;
        self.message = format!("repeat {}", if self.repeat { "on" } else { "off" });
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.reset_shuffle_order();
        self.message = format!("shuffle {}", if self.shuffle { "on" } else { "off" });
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
            PlaybackState::Playing => ACTIVE_TICK,
            PlaybackState::Paused => MEDIA_IDLE_TICK,
            PlaybackState::Stopped => STOPPED_TICK,
        }
    }
}
