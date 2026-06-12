mod appkit;
mod media_session;
mod notifications;
mod notifier_helper;

use anyhow::{Context, Result};

use super::{Integration, IntegrationCommand, IntegrationEvent, PlaybackSnapshot};
use crate::player::PlaybackState;

use appkit::AppKitPump;
use media_session::MediaSession;
use notifications::MacNotifier;

pub(super) fn run_helper_if_requested() -> Result<bool> {
    notifier_helper::run_if_requested()
}

pub(super) struct LazyMacIntegration {
    inner: Option<MacIntegration>,
    unavailable: bool,
    track_notifications_visible: bool,
}

impl Default for LazyMacIntegration {
    fn default() -> Self {
        Self {
            inner: None,
            unavailable: false,
            track_notifications_visible: true,
        }
    }
}

impl LazyMacIntegration {
    fn inner_mut(&mut self) -> Result<&mut MacIntegration> {
        if self.inner.is_none() && !self.unavailable {
            match MacIntegration::new(self.track_notifications_visible) {
                Ok(integration) => self.inner = Some(integration),
                Err(error) => {
                    self.unavailable = true;
                    return Err(error);
                }
            }
        }
        self.inner
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("macOS integrations are unavailable for this process"))
    }
}

impl Integration for LazyMacIntegration {
    fn tick(&mut self) {
        if let Ok(inner) = self.inner_mut() {
            inner.tick();
        }
    }

    fn next_command(&mut self) -> Option<IntegrationCommand> {
        self.inner_mut().ok().and_then(Integration::next_command)
    }

    fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
        if let IntegrationEvent::TrackNotificationsVisible(visible) = event {
            self.track_notifications_visible = *visible;
            if let Some(inner) = &mut self.inner {
                inner.publish_event(event)?;
            }
            return Ok(());
        }

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
            return Ok(());
        }

        self.inner_mut()?.publish_event(event)
    }
}

struct MacIntegration {
    media_session: MediaSession,
    notifier: MacNotifier,
    appkit_pump: AppKitPump,
}

impl MacIntegration {
    fn new(track_notifications_visible: bool) -> Result<Self> {
        let appkit_pump = AppKitPump::new();
        Ok(Self {
            media_session: MediaSession::new()?,
            notifier: MacNotifier::new(track_notifications_visible),
            appkit_pump,
        })
    }
}

impl Integration for MacIntegration {
    fn tick(&mut self) {
        self.appkit_pump.pump_pending_events();
    }

    fn next_command(&mut self) -> Option<IntegrationCommand> {
        self.media_session.next_command()
    }

    fn publish_event(&mut self, event: &IntegrationEvent) -> Result<()> {
        match event {
            IntegrationEvent::TrackChanged(track) => {
                self.media_session.set_now_playing(track)?;
                self.notifier
                    .notify_track_changed(track)
                    .context("publishing macOS track notification")?;
                Ok(())
            }
            IntegrationEvent::Playback(playback) => {
                self.media_session.set_playback_state(*playback)
            }
            IntegrationEvent::TrackNotificationsVisible(visible) => {
                self.notifier.set_visible(*visible);
                Ok(())
            }
        }
    }
}
