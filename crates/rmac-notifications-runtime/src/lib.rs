//! Live authority coordinator for notification banners.
//!
//! The coordinator copies no title, body, app identity, action, or target. It
//! combines notification metadata with compositor topology and appearance
//! motion, then emits typed commands for the Linux service/UI adapter.

use rmac_notifications::banner::{
    CloseCause, Config, Effect, Motion, OutputId, PlacementContext, PlacementPolicy, Schedule,
    Snapshot, Stack,
};
use rmac_notifications::{Notification, NotificationId, PostOutcome, Time};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Redraw,
    CapturePreviousFocus,
    RestorePreviousFocus,
    Expire(NotificationId),
    Dismiss(NotificationId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub commands: Vec<Command>,
    pub schedule: Schedule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidConfig,
    NoOutput,
    UnknownNotification,
    UnknownBanner,
}

#[derive(Debug)]
pub struct Coordinator {
    stack: Stack,
    placement_policy: PlacementPolicy,
    topology: PlacementContext,
}

impl Coordinator {
    pub fn new(
        config: Config,
        placement_policy: PlacementPolicy,
        appearance: &rmac_appearance::Snapshot,
    ) -> Result<Self, Error> {
        let motion = motion_for(appearance);
        Ok(Self {
            stack: Stack::new(config, motion).map_err(|_| Error::InvalidConfig)?,
            placement_policy,
            topology: PlacementContext::default(),
        })
    }

    pub fn apply_post(
        &mut self,
        outcome: PostOutcome,
        notifications: &[Notification],
        now: Time,
    ) -> Result<Update, Error> {
        if !outcome.delivery.banner {
            let effects = self.stack.reconcile_closed(outcome.id, now);
            return Ok(self.update(effects, now));
        }
        let notification = notifications
            .iter()
            .find(|notification| notification.id == outcome.id)
            .ok_or(Error::UnknownNotification)?;
        let output = self
            .topology
            .resolve(self.placement_policy)
            .map_err(|_| Error::NoOutput)?;
        let timeout_ms = notification
            .expires_at
            .map(|deadline| deadline.0.saturating_sub(notification.updated_at.0));
        let effects = self
            .stack
            .post(outcome.id, output, timeout_ms, outcome.announce_as_new, now);
        Ok(self.update(effects, now))
    }

    pub fn apply_compositor(
        &mut self,
        compositor: &rmac_compositor::Snapshot,
        now: Time,
    ) -> Result<Update, Error> {
        let previous = self.topology.clone();
        self.topology = placement_context(compositor);
        if self.topology.connected.is_empty() {
            self.topology = previous;
            return if self.topology.connected.is_empty() {
                Err(Error::NoOutput)
            } else {
                Ok(self.update(Vec::new(), now))
            };
        }
        let fallback = self
            .topology
            .resolve(self.placement_policy)
            .map_err(|_| Error::NoOutput)?;
        let effects = self
            .stack
            .outputs_changed(&self.topology.connected, &fallback)
            .map_err(|_| Error::NoOutput)?;
        Ok(self.update(effects, now))
    }

    pub fn apply_appearance(
        &mut self,
        appearance: &rmac_appearance::Snapshot,
        now: Time,
    ) -> Update {
        let effects = self.stack.set_motion(motion_for(appearance), now);
        self.update(effects, now)
    }

    pub fn set_hovered(
        &mut self,
        id: NotificationId,
        hovered: bool,
        now: Time,
    ) -> Result<Update, Error> {
        let effects = self
            .stack
            .set_hovered(id, hovered, now)
            .map_err(|_| Error::UnknownBanner)?;
        Ok(self.update(effects, now))
    }

    pub fn set_focus(&mut self, id: Option<NotificationId>, now: Time) -> Result<Update, Error> {
        let effects = self
            .stack
            .focus(id, now)
            .map_err(|_| Error::UnknownBanner)?;
        Ok(self.update(effects, now))
    }

    pub fn request_dismiss(&mut self, id: NotificationId, now: Time) -> Result<Update, Error> {
        let effects = self
            .stack
            .close(id, CloseCause::Dismissed, now)
            .map_err(|_| Error::UnknownBanner)?;
        Ok(self.update(effects, now))
    }

    /// Reconciles a close already committed by the notification service. A
    /// history-only, flood-retired, or duplicate close is intentionally inert.
    pub fn apply_closed(&mut self, id: NotificationId, now: Time) -> Update {
        let effects = self.stack.reconcile_closed(id, now);
        self.update(effects, now)
    }

    /// Called after the service has successfully dispatched an action.
    pub fn action_completed(
        &mut self,
        id: NotificationId,
        notification_remains: bool,
        now: Time,
    ) -> Result<Update, Error> {
        if notification_remains {
            return Ok(self.update(Vec::new(), now));
        }
        let effects = self
            .stack
            .close(id, CloseCause::Action, now)
            .map_err(|_| Error::UnknownBanner)?;
        Ok(self.update(effects, now))
    }

    pub fn advance(&mut self, now: Time) -> Update {
        let effects = self.stack.advance(now);
        self.update(effects, now)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.stack.snapshot()
    }

    pub fn schedule(&self, now: Time) -> Schedule {
        self.stack.schedule(now)
    }

    fn update(&self, effects: Vec<Effect>, now: Time) -> Update {
        Update {
            commands: translate(effects),
            schedule: self.stack.schedule(now),
        }
    }
}

fn motion_for(appearance: &rmac_appearance::Snapshot) -> Motion {
    match appearance.motion {
        rmac_appearance::MotionPreference::Full => Motion::Full,
        rmac_appearance::MotionPreference::Reduced => Motion::Reduced,
    }
}

fn placement_context(compositor: &rmac_compositor::Snapshot) -> PlacementContext {
    let connected: Vec<_> = compositor
        .outputs
        .iter()
        .filter(|output| output.enabled())
        .filter_map(|output| OutputId::parse(output.id.0.clone()).ok())
        .collect();
    let active = compositor
        .focus
        .output
        .as_ref()
        .and_then(|output| OutputId::parse(output.0.clone()).ok())
        .filter(|output| connected.contains(output));
    PlacementContext {
        active,
        pointer: None,
        primary: connected.first().cloned(),
        connected,
    }
}

fn translate(effects: Vec<Effect>) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut redraw = false;
    for effect in effects {
        match effect {
            Effect::VisualChanged | Effect::RetireVisual { .. } => redraw = true,
            Effect::CapturePreviousFocus => commands.push(Command::CapturePreviousFocus),
            Effect::RestorePreviousFocus => commands.push(Command::RestorePreviousFocus),
            Effect::Close {
                id,
                cause: CloseCause::Expired,
            } => commands.push(Command::Expire(id)),
            Effect::Close {
                id,
                cause: CloseCause::Dismissed,
            } => commands.push(Command::Dismiss(id)),
            Effect::Close {
                cause: CloseCause::Action | CloseCause::Authority,
                ..
            } => {}
        }
    }
    if redraw {
        commands.insert(0, Command::Redraw);
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_compositor::{
        FocusState, LogicalOutput, LogicalPoint, LogicalSize, Output,
        OutputId as CompositorOutputId, OutputMode, PhysicalSize,
    };
    use rmac_notifications::protocol::{self, PortalInput};
    use rmac_notifications::{DeliveryPolicy, Server, TimeoutPolicy};

    fn output(name: &str) -> Output {
        Output {
            id: CompositorOutputId(name.into()),
            make: "private".into(),
            model: "private".into(),
            serial: Some("private".into()),
            physical_size_mm: None,
            modes: vec![OutputMode {
                physical_size: PhysicalSize {
                    width: 1920,
                    height: 1080,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(LogicalOutput {
                position: LogicalPoint { x: 0.0, y: 0.0 },
                size: LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    fn posted(timeout: rmac_notifications::Timeout) -> (PostOutcome, Vec<Notification>) {
        let request = protocol::portal(PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            title: Some("Private".into()),
            ..PortalInput::default()
        })
        .unwrap();
        let mut request = request;
        request.timeout = timeout;
        let mut server = Server::new(10, TimeoutPolicy::default());
        let outcome = server
            .post(request, Time(100), DeliveryPolicy::default())
            .unwrap();
        (outcome, server.active().cloned().collect())
    }

    fn coordinator(motion: rmac_appearance::MotionPreference) -> Coordinator {
        let appearance = rmac_appearance::Snapshot {
            motion,
            ..rmac_appearance::Snapshot::default()
        };
        Coordinator::new(
            Config::default(),
            PlacementPolicy::ActiveOutput,
            &appearance,
        )
        .unwrap()
    }

    #[test]
    fn post_uses_focused_connected_output_and_authoritative_timeout_duration() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
        let compositor = rmac_compositor::Snapshot {
            outputs: vec![output("eDP-private"), output("HDMI-private")],
            focus: FocusState {
                output: Some(CompositorOutputId("HDMI-private".into())),
                ..FocusState::default()
            },
            ..rmac_compositor::Snapshot::default()
        };
        coordinator.apply_compositor(&compositor, Time(0)).unwrap();
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Milliseconds(2_000));
        let update = coordinator
            .apply_post(outcome, &notifications, Time(500))
            .unwrap();
        assert_eq!(update.commands, vec![Command::Redraw]);
        assert_eq!(
            coordinator.snapshot().banners[0].output.as_str(),
            "HDMI-private"
        );
        assert_eq!(update.schedule.wake_at, Some(Time(2_500)));
    }

    #[test]
    fn paused_banner_expires_through_one_specific_service_command() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("one")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Milliseconds(1_000));
        let id = outcome.id;
        coordinator
            .apply_post(outcome, &notifications, Time(0))
            .unwrap();
        coordinator.set_hovered(id, true, Time(400)).unwrap();
        assert!(coordinator.advance(Time(2_000)).commands.is_empty());
        coordinator.set_hovered(id, false, Time(2_000)).unwrap();
        let update = coordinator.advance(Time(2_600));
        assert_eq!(update.commands, vec![Command::Redraw, Command::Expire(id)]);
    }

    #[test]
    fn hotplug_moves_banner_without_copying_or_closing_notification() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("gone")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
        coordinator
            .apply_post(outcome, &notifications, Time(0))
            .unwrap();
        let update = coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("fallback")],
                    ..Default::default()
                },
                Time(1),
            )
            .unwrap();
        assert_eq!(update.commands, vec![Command::Redraw]);
        assert_eq!(
            coordinator.snapshot().banners[0].output.as_str(),
            "fallback"
        );
    }

    #[test]
    fn reduced_motion_change_completes_exit_and_preserves_focus_commands() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Full);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("one")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
        let id = outcome.id;
        coordinator
            .apply_post(outcome, &notifications, Time(0))
            .unwrap();
        coordinator.set_focus(Some(id), Time(1)).unwrap();
        coordinator.request_dismiss(id, Time(2)).unwrap();
        let reduced = rmac_appearance::Snapshot {
            motion: rmac_appearance::MotionPreference::Reduced,
            ..Default::default()
        };
        let update = coordinator.apply_appearance(&reduced, Time(3));
        assert_eq!(
            update.commands,
            vec![
                Command::Redraw,
                Command::Dismiss(id),
                Command::RestorePreviousFocus
            ]
        );
    }

    #[test]
    fn unavailable_topology_keeps_last_known_good_output() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("known")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let empty = coordinator
            .apply_compositor(&rmac_compositor::Snapshot::default(), Time(1))
            .unwrap();
        assert!(empty.commands.is_empty());
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
        coordinator
            .apply_post(outcome, &notifications, Time(2))
            .unwrap();
        assert_eq!(coordinator.snapshot().banners[0].output.as_str(), "known");
    }

    #[test]
    fn authoritative_close_disarms_a_pending_dismiss_command() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Full);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("one")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let (outcome, notifications) = posted(rmac_notifications::Timeout::Never);
        let id = outcome.id;
        coordinator
            .apply_post(outcome, &notifications, Time(0))
            .unwrap();
        coordinator.advance(Time(220));
        let dismissing = coordinator.request_dismiss(id, Time(300)).unwrap();
        assert_eq!(dismissing.schedule.wake_at, Some(Time(480)));

        let reconciled = coordinator.apply_closed(id, Time(350));
        assert!(reconciled.commands.is_empty());
        assert_eq!(reconciled.schedule.wake_at, Some(Time(480)));
        assert_eq!(
            coordinator.advance(Time(480)).commands,
            vec![Command::Redraw]
        );
        assert!(coordinator.apply_closed(id, Time(500)).commands.is_empty());
    }

    #[test]
    fn policy_suppressed_replacement_removes_an_existing_visual() {
        let mut coordinator = coordinator(rmac_appearance::MotionPreference::Reduced);
        coordinator
            .apply_compositor(
                &rmac_compositor::Snapshot {
                    outputs: vec![output("one")],
                    ..Default::default()
                },
                Time(0),
            )
            .unwrap();
        let (mut outcome, notifications) = posted(rmac_notifications::Timeout::Never);
        coordinator
            .apply_post(outcome, &notifications, Time(0))
            .unwrap();
        assert_eq!(coordinator.snapshot().banners.len(), 1);

        outcome.delivery.banner = false;
        let update = coordinator
            .apply_post(outcome, &notifications, Time(1))
            .unwrap();
        assert_eq!(update.commands, vec![Command::Redraw]);
        assert!(coordinator.snapshot().banners.is_empty());
    }
}
