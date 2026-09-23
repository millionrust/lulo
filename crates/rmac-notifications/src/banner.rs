//! Deterministic banner-stack behavior for the E2 presentation surface.
//!
//! This module stores notification IDs and motion state, never notification
//! content. GPUI and Wayland adapters render [`Snapshot`] and execute
//! [`Effect`] values without inventing their own timeouts or focus policy.

use std::fmt;

use super::{NotificationId, Time};

const MAX_OUTPUT_ID_BYTES: usize = 256;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct OutputId(String);

impl OutputId {
    pub fn parse(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.trim().is_empty()
            || value.len() > MAX_OUTPUT_ID_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(Error::InvalidOutput);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OutputId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OutputId(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlacementPolicy {
    ActiveOutput,
    PointerOutput,
    PrimaryOutput,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlacementContext {
    pub active: Option<OutputId>,
    pub pointer: Option<OutputId>,
    pub primary: Option<OutputId>,
    pub connected: Vec<OutputId>,
}

impl PlacementContext {
    pub fn resolve(&self, policy: PlacementPolicy) -> Result<OutputId, Error> {
        let preferred = match policy {
            PlacementPolicy::ActiveOutput => [&self.active, &self.pointer, &self.primary],
            PlacementPolicy::PointerOutput => [&self.pointer, &self.active, &self.primary],
            PlacementPolicy::PrimaryOutput => [&self.primary, &self.active, &self.pointer],
        };
        preferred
            .into_iter()
            .flatten()
            .find(|output| self.connected.contains(output))
            .cloned()
            .or_else(|| self.connected.first().cloned())
            .ok_or(Error::NoOutput)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub max_visible_per_output: usize,
    pub enter_ms: u64,
    pub exit_ms: u64,
    pub top_inset_px: u16,
    pub trailing_inset_px: u16,
    pub gap_px: u16,
    pub width_px: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_visible_per_output: 3,
            enter_ms: 220,
            exit_ms: 180,
            // macOS 26 banner: a 344 pt card, 8 pt under the menu bar and
            // from the screen edge, 8 pt between stacked banners
            // (design-lab/notifications.html).
            top_inset_px: 8,
            trailing_inset_px: 8,
            gap_px: 8,
            width_px: 344,
        }
    }
}

impl Config {
    pub fn validate(self) -> Result<Self, Error> {
        if self.max_visible_per_output == 0
            || self.max_visible_per_output > 8
            || self.enter_ms > 2_000
            || self.exit_ms > 2_000
            || !(280..=480).contains(&self.width_px)
        {
            return Err(Error::InvalidConfig);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
    Full,
    Reduced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseCause {
    Expired,
    Dismissed,
    Action,
    /// The notification authority already closed or suppressed this record.
    /// Finishing its visual exit must not call the authority again.
    Authority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseSnapshot {
    Entering,
    Visible,
    Exiting(CloseCause),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PauseState {
    pub hovered: bool,
    pub keyboard_focused: bool,
}

impl PauseState {
    fn active(self) -> bool {
        self.hovered || self.keyboard_focused
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BannerSnapshot {
    pub id: NotificationId,
    pub output: OutputId,
    pub phase: PhaseSnapshot,
    pub pause: PauseState,
    /// Zero is the topmost/newest banner on this output.
    pub stack_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub banners: Vec<BannerSnapshot>,
    pub top_inset_px: u16,
    pub trailing_inset_px: u16,
    pub gap_px: u16,
    pub width_px: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    VisualChanged,
    CapturePreviousFocus,
    RestorePreviousFocus,
    Close {
        id: NotificationId,
        cause: CloseCause,
    },
    /// A flood or output move removed only the visual banner. The notification
    /// authority and policy-allowed Center history remain intact.
    RetireVisual {
        id: NotificationId,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Schedule {
    /// Request another frame only while an enter/exit transition is active.
    pub frame: bool,
    /// Exact monotonic wakeup for the next timeout or transition boundary.
    pub wake_at: Option<Time>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidOutput,
    NoOutput,
    InvalidConfig,
    UnknownBanner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Entering { ends_at: Time },
    Visible,
    Exiting { cause: CloseCause, ends_at: Time },
}

#[derive(Clone, Debug)]
struct Banner {
    id: NotificationId,
    output: OutputId,
    phase: Phase,
    pause: PauseState,
    remaining_ms: Option<u64>,
    deadline: Option<Time>,
    sequence: u64,
}

#[derive(Debug)]
pub struct Stack {
    config: Config,
    motion: Motion,
    next_sequence: u64,
    focused: Option<NotificationId>,
    banners: Vec<Banner>,
}

impl Stack {
    pub fn new(config: Config, motion: Motion) -> Result<Self, Error> {
        Ok(Self {
            config: config.validate()?,
            motion,
            next_sequence: 1,
            focused: None,
            banners: Vec::new(),
        })
    }

    pub fn set_motion(&mut self, motion: Motion, now: Time) -> Vec<Effect> {
        if self.motion == motion {
            return Vec::new();
        }
        self.motion = motion;
        if motion == Motion::Reduced {
            let mut effects = Vec::new();
            let mut index = 0;
            while index < self.banners.len() {
                match self.banners[index].phase {
                    Phase::Entering { .. } => {
                        self.banners[index].phase = Phase::Visible;
                        arm_deadline(&mut self.banners[index], now);
                        index += 1;
                    }
                    Phase::Exiting { cause, .. } => {
                        let banner = self.banners.remove(index);
                        effects.push(Effect::Close {
                            id: banner.id,
                            cause,
                        });
                        if self.focused == Some(banner.id) {
                            self.focused = None;
                            effects.push(Effect::RestorePreviousFocus);
                        }
                    }
                    Phase::Visible => index += 1,
                }
            }
            effects.insert(0, Effect::VisualChanged);
            effects
        } else {
            vec![Effect::VisualChanged]
        }
    }

    pub fn post(
        &mut self,
        id: NotificationId,
        output: OutputId,
        timeout_ms: Option<u64>,
        announce_as_new: bool,
        now: Time,
    ) -> Vec<Effect> {
        let sequence = self.take_sequence();
        if let Some(index) = self.banners.iter().position(|banner| banner.id == id) {
            let banner = &mut self.banners[index];
            if announce_as_new {
                banner.output = output.clone();
                banner.phase = entering_phase(self.motion, self.config.enter_ms, now);
            } else if matches!(banner.phase, Phase::Exiting { .. }) {
                banner.phase = Phase::Visible;
            }
            banner.sequence = sequence;
            banner.remaining_ms = timeout_ms;
            banner.deadline = None;
            if matches!(banner.phase, Phase::Visible) {
                arm_deadline(banner, now);
            }
        } else {
            let mut banner = Banner {
                id,
                output: output.clone(),
                phase: entering_phase(self.motion, self.config.enter_ms, now),
                pause: PauseState {
                    hovered: false,
                    keyboard_focused: false,
                },
                remaining_ms: timeout_ms,
                deadline: None,
                sequence,
            };
            if matches!(banner.phase, Phase::Visible) {
                arm_deadline(&mut banner, now);
            }
            self.banners.push(banner);
        }
        let mut effects = vec![Effect::VisualChanged];
        self.enforce_limit(&output, id, &mut effects);
        effects
    }

    pub fn set_hovered(
        &mut self,
        id: NotificationId,
        hovered: bool,
        now: Time,
    ) -> Result<Vec<Effect>, Error> {
        let banner = self
            .banners
            .iter_mut()
            .find(|banner| banner.id == id)
            .ok_or(Error::UnknownBanner)?;
        let was_paused = banner.pause.active();
        banner.pause.hovered = hovered;
        update_pause(banner, was_paused, now);
        Ok(vec![Effect::VisualChanged])
    }

    pub fn focus(&mut self, id: Option<NotificationId>, now: Time) -> Result<Vec<Effect>, Error> {
        if self.focused == id {
            return Ok(Vec::new());
        }
        if id.is_some_and(|id| !self.banners.iter().any(|banner| banner.id == id)) {
            return Err(Error::UnknownBanner);
        }
        let mut effects = vec![Effect::VisualChanged];
        if let Some(previous) = self.focused {
            if let Some(banner) = self.banners.iter_mut().find(|banner| banner.id == previous) {
                let was_paused = banner.pause.active();
                banner.pause.keyboard_focused = false;
                update_pause(banner, was_paused, now);
            }
        } else if id.is_some() {
            effects.push(Effect::CapturePreviousFocus);
        }
        if let Some(id) = id {
            let banner = self
                .banners
                .iter_mut()
                .find(|banner| banner.id == id)
                .ok_or(Error::UnknownBanner)?;
            let was_paused = banner.pause.active();
            banner.pause.keyboard_focused = true;
            update_pause(banner, was_paused, now);
        } else {
            effects.push(Effect::RestorePreviousFocus);
        }
        self.focused = id;
        Ok(effects)
    }

    pub fn close(
        &mut self,
        id: NotificationId,
        cause: CloseCause,
        now: Time,
    ) -> Result<Vec<Effect>, Error> {
        let index = self
            .banners
            .iter()
            .position(|banner| banner.id == id)
            .ok_or(Error::UnknownBanner)?;
        Ok(self.begin_exit(index, cause, now))
    }

    /// Reconciles a close already completed by the notification authority.
    /// Unknown/retired banners are intentionally inert. If a local exit is in
    /// progress, preserve its visual deadline but disarm its eventual service
    /// command so the close cannot be dispatched twice.
    pub fn reconcile_closed(&mut self, id: NotificationId, now: Time) -> Vec<Effect> {
        let Some(index) = self.banners.iter().position(|banner| banner.id == id) else {
            return Vec::new();
        };
        if let Phase::Exiting { cause, .. } = &mut self.banners[index].phase {
            if *cause != CloseCause::Authority {
                *cause = CloseCause::Authority;
            }
            return Vec::new();
        }
        self.begin_exit(index, CloseCause::Authority, now)
    }

    pub fn outputs_changed(
        &mut self,
        connected: &[OutputId],
        fallback: &OutputId,
    ) -> Result<Vec<Effect>, Error> {
        if !connected.contains(fallback) {
            return Err(Error::NoOutput);
        }
        let mut moved_outputs = Vec::new();
        for banner in &mut self.banners {
            if !connected.contains(&banner.output) {
                banner.output = fallback.clone();
                moved_outputs.push(banner.id);
            }
        }
        if moved_outputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut effects = vec![Effect::VisualChanged];
        if let Some(keep) = moved_outputs.last().copied() {
            self.enforce_limit(fallback, keep, &mut effects);
        }
        Ok(effects)
    }

    pub fn advance(&mut self, now: Time) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut index = 0;
        while index < self.banners.len() {
            match self.banners[index].phase {
                Phase::Entering { ends_at } if ends_at.0 <= now.0 => {
                    self.banners[index].phase = Phase::Visible;
                    arm_deadline(&mut self.banners[index], now);
                    effects.push(Effect::VisualChanged);
                    index += 1;
                }
                Phase::Visible
                    if self.banners[index]
                        .deadline
                        .is_some_and(|deadline| deadline.0 <= now.0) =>
                {
                    effects.extend(self.begin_exit(index, CloseCause::Expired, now));
                    if self.motion == Motion::Full {
                        index += 1;
                    }
                }
                Phase::Exiting { cause, ends_at } if ends_at.0 <= now.0 => {
                    let banner = self.banners.remove(index);
                    effects.push(Effect::VisualChanged);
                    effects.push(Effect::Close {
                        id: banner.id,
                        cause,
                    });
                    if self.focused == Some(banner.id) {
                        self.focused = None;
                        effects.push(Effect::RestorePreviousFocus);
                    }
                }
                _ => index += 1,
            }
        }
        effects
    }

    pub fn schedule(&self, now: Time) -> Schedule {
        let frame = self.banners.iter().any(|banner| match banner.phase {
            Phase::Entering { ends_at } | Phase::Exiting { ends_at, .. } => ends_at.0 > now.0,
            Phase::Visible => false,
        });
        let wake_at = self
            .banners
            .iter()
            .filter_map(|banner| match banner.phase {
                Phase::Entering { ends_at } | Phase::Exiting { ends_at, .. } => Some(ends_at),
                Phase::Visible => banner.deadline,
            })
            .filter(|time| time.0 > now.0)
            .min_by_key(|time| time.0);
        Schedule { frame, wake_at }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut banners = self.banners.clone();
        banners.sort_by(|left, right| {
            left.output
                .cmp(&right.output)
                .then_with(|| right.sequence.cmp(&left.sequence))
        });
        let snapshots = banners
            .iter()
            .enumerate()
            .map(|(index, banner)| {
                let stack_index = banners[..index]
                    .iter()
                    .filter(|candidate| candidate.output == banner.output)
                    .count();
                BannerSnapshot {
                    id: banner.id,
                    output: banner.output.clone(),
                    phase: phase_snapshot(banner.phase),
                    pause: banner.pause,
                    stack_index,
                }
            })
            .collect();
        Snapshot {
            banners: snapshots,
            top_inset_px: self.config.top_inset_px,
            trailing_inset_px: self.config.trailing_inset_px,
            gap_px: self.config.gap_px,
            width_px: self.config.width_px,
        }
    }

    fn begin_exit(&mut self, index: usize, cause: CloseCause, now: Time) -> Vec<Effect> {
        let id = self.banners[index].id;
        if self.motion == Motion::Reduced || self.config.exit_ms == 0 {
            self.banners.remove(index);
            let mut effects = vec![Effect::VisualChanged, Effect::Close { id, cause }];
            if self.focused == Some(id) {
                self.focused = None;
                effects.push(Effect::RestorePreviousFocus);
            }
            effects
        } else {
            self.banners[index].phase = Phase::Exiting {
                cause,
                ends_at: Time(now.0.saturating_add(self.config.exit_ms)),
            };
            self.banners[index].deadline = None;
            vec![Effect::VisualChanged]
        }
    }

    fn enforce_limit(
        &mut self,
        output: &OutputId,
        newest: NotificationId,
        effects: &mut Vec<Effect>,
    ) {
        while self
            .banners
            .iter()
            .filter(|banner| &banner.output == output)
            .count()
            > self.config.max_visible_per_output
        {
            let retire = self
                .banners
                .iter()
                .enumerate()
                .filter(|(_, banner)| &banner.output == output)
                .filter(|(_, banner)| self.focused != Some(banner.id))
                .min_by_key(|(_, banner)| banner.sequence)
                .map(|(index, _)| index)
                .or_else(|| self.banners.iter().position(|banner| banner.id == newest));
            let Some(index) = retire else {
                break;
            };
            let banner = self.banners.remove(index);
            effects.push(Effect::RetireVisual { id: banner.id });
        }
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }
}

fn entering_phase(motion: Motion, duration_ms: u64, now: Time) -> Phase {
    if motion == Motion::Reduced || duration_ms == 0 {
        Phase::Visible
    } else {
        Phase::Entering {
            ends_at: Time(now.0.saturating_add(duration_ms)),
        }
    }
}

fn phase_snapshot(phase: Phase) -> PhaseSnapshot {
    match phase {
        Phase::Entering { .. } => PhaseSnapshot::Entering,
        Phase::Visible => PhaseSnapshot::Visible,
        Phase::Exiting { cause, .. } => PhaseSnapshot::Exiting(cause),
    }
}

fn arm_deadline(banner: &mut Banner, now: Time) {
    if !banner.pause.active() {
        banner.deadline = banner
            .remaining_ms
            .map(|duration| Time(now.0.saturating_add(duration)));
    }
}

fn update_pause(banner: &mut Banner, was_paused: bool, now: Time) {
    let is_paused = banner.pause.active();
    if !was_paused && is_paused {
        if let Some(deadline) = banner.deadline.take() {
            banner.remaining_ms = Some(deadline.0.saturating_sub(now.0));
        }
    } else if was_paused && !is_paused && matches!(banner.phase, Phase::Visible) {
        arm_deadline(banner, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str) -> OutputId {
        OutputId::parse(name).unwrap()
    }

    fn id(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    #[test]
    fn placement_prefers_active_then_connected_fallbacks() {
        let context = PlacementContext {
            active: Some(output("DP-2-private")),
            pointer: Some(output("HDMI-A-1-private")),
            primary: Some(output("eDP-1-private")),
            connected: vec![output("eDP-1-private"), output("HDMI-A-1-private")],
        };
        assert_eq!(
            context.resolve(PlacementPolicy::ActiveOutput).unwrap(),
            output("HDMI-A-1-private")
        );
        assert!(!format!("{:?}", context.connected[0]).contains("eDP"));
    }

    #[test]
    fn hover_and_keyboard_focus_pause_exact_remaining_time() {
        let mut stack = Stack::new(Config::default(), Motion::Reduced).unwrap();
        stack.post(id(1), output("one"), Some(1_000), false, Time(100));
        stack.set_hovered(id(1), true, Time(400)).unwrap();
        assert_eq!(stack.schedule(Time(900)).wake_at, None);
        stack.focus(Some(id(1)), Time(500)).unwrap();
        stack.set_hovered(id(1), false, Time(600)).unwrap();
        assert_eq!(stack.schedule(Time(900)).wake_at, None);
        stack.focus(None, Time(1_000)).unwrap();
        assert_eq!(stack.schedule(Time(1_000)).wake_at, Some(Time(1_700)));
        assert!(stack.advance(Time(1_699)).is_empty());
        assert!(stack.advance(Time(1_700)).contains(&Effect::Close {
            id: id(1),
            cause: CloseCause::Expired
        }));
    }

    #[test]
    fn replacement_updates_in_place_unless_show_as_new_is_requested() {
        let mut stack = Stack::new(Config::default(), Motion::Full).unwrap();
        stack.post(id(1), output("one"), Some(1_000), true, Time(0));
        stack.advance(Time(220));
        stack.post(id(1), output("two"), Some(2_000), false, Time(300));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.banners.len(), 1);
        assert_eq!(snapshot.banners[0].output, output("one"));
        assert_eq!(snapshot.banners[0].phase, PhaseSnapshot::Visible);

        stack.post(id(1), output("two"), Some(2_000), true, Time(400));
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.banners[0].output, output("two"));
        assert_eq!(snapshot.banners[0].phase, PhaseSnapshot::Entering);
    }

    #[test]
    fn full_motion_requests_frames_only_during_transitions() {
        let mut stack = Stack::new(Config::default(), Motion::Full).unwrap();
        stack.post(id(1), output("one"), Some(1_000), true, Time(0));
        assert_eq!(
            stack.schedule(Time(0)),
            Schedule {
                frame: true,
                wake_at: Some(Time(220))
            }
        );
        stack.advance(Time(220));
        assert_eq!(
            stack.schedule(Time(220)),
            Schedule {
                frame: false,
                wake_at: Some(Time(1_220))
            }
        );
        stack.advance(Time(1_220));
        assert_eq!(
            stack.schedule(Time(1_220)),
            Schedule {
                frame: true,
                wake_at: Some(Time(1_400))
            }
        );
    }

    #[test]
    fn reduced_motion_finishes_active_transitions_immediately() {
        let mut stack = Stack::new(Config::default(), Motion::Full).unwrap();
        stack.post(id(1), output("one"), None, true, Time(0));
        stack.close(id(1), CloseCause::Dismissed, Time(10)).unwrap();
        let effects = stack.set_motion(Motion::Reduced, Time(20));
        assert!(effects.contains(&Effect::Close {
            id: id(1),
            cause: CloseCause::Dismissed
        }));
        assert!(stack.snapshot().banners.is_empty());
    }

    #[test]
    fn flood_keeps_newest_banners_without_closing_history_records() {
        let config = Config {
            max_visible_per_output: 2,
            ..Config::default()
        };
        let mut stack = Stack::new(config, Motion::Reduced).unwrap();
        stack.post(id(1), output("one"), None, false, Time(0));
        stack.post(id(2), output("one"), None, false, Time(1));
        let effects = stack.post(id(3), output("one"), None, false, Time(2));
        assert!(effects.contains(&Effect::RetireVisual { id: id(1) }));
        assert_eq!(
            stack
                .snapshot()
                .banners
                .iter()
                .map(|banner| banner.id)
                .collect::<Vec<_>>(),
            vec![id(3), id(2)]
        );
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, Effect::Close { .. })));
    }

    #[test]
    fn focused_banner_survives_flood_and_restores_focus_after_close() {
        let config = Config {
            max_visible_per_output: 1,
            ..Config::default()
        };
        let mut stack = Stack::new(config, Motion::Reduced).unwrap();
        stack.post(id(1), output("one"), None, false, Time(0));
        let focus = stack.focus(Some(id(1)), Time(0)).unwrap();
        assert!(focus.contains(&Effect::CapturePreviousFocus));
        let effects = stack.post(id(2), output("one"), None, false, Time(1));
        assert!(effects.contains(&Effect::RetireVisual { id: id(2) }));
        let effects = stack.close(id(1), CloseCause::Action, Time(2)).unwrap();
        assert!(effects.contains(&Effect::RestorePreviousFocus));
    }

    #[test]
    fn disconnected_outputs_move_to_fallback_and_reapply_bound() {
        let config = Config {
            max_visible_per_output: 2,
            ..Config::default()
        };
        let mut stack = Stack::new(config, Motion::Reduced).unwrap();
        stack.post(id(1), output("gone"), None, false, Time(0));
        stack.post(id(2), output("fallback"), None, false, Time(1));
        stack.post(id(3), output("fallback"), None, false, Time(2));
        let effects = stack
            .outputs_changed(&[output("fallback")], &output("fallback"))
            .unwrap();
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::RetireVisual { .. })));
        assert_eq!(stack.snapshot().banners.len(), 2);
    }

    #[test]
    fn authoritative_close_is_idempotent_and_disarms_local_close_dispatch() {
        let mut stack = Stack::new(Config::default(), Motion::Full).unwrap();
        stack.post(id(1), output("one"), None, false, Time(0));
        stack.advance(Time(220));
        stack
            .close(id(1), CloseCause::Dismissed, Time(300))
            .unwrap();
        assert_eq!(stack.schedule(Time(300)).wake_at, Some(Time(480)));

        assert!(stack.reconcile_closed(id(1), Time(350)).is_empty());
        assert_eq!(stack.schedule(Time(350)).wake_at, Some(Time(480)));
        let effects = stack.advance(Time(480));
        assert!(effects.contains(&Effect::Close {
            id: id(1),
            cause: CloseCause::Authority,
        }));
        assert!(!effects.contains(&Effect::Close {
            id: id(1),
            cause: CloseCause::Dismissed,
        }));
        assert!(stack.reconcile_closed(id(1), Time(500)).is_empty());
    }
}
