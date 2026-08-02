use rmac_notifications::banner::{
    CloseCause, Config, Effect, Motion, OutputId, PlacementContext, PlacementPolicy, Schedule,
    Snapshot, Stack,
};
use rmac_notifications::{Notification, NotificationId, PostOutcome, Time};

use crate::{Command, Error, Update};

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
