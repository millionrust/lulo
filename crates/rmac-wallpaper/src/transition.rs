//! Event-driven wallpaper crossfade state with reduced-motion handling.

use std::time::Duration;

const DEFAULT_DURATION: Duration = Duration::from_millis(300);
const MAX_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    pub reduced_motion: bool,
    pub duration: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            duration: DEFAULT_DURATION,
        }
    }
}

impl Policy {
    fn effective_duration(self) -> Duration {
        if self.reduced_motion {
            Duration::ZERO
        } else {
            self.duration.min(MAX_DURATION)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub frame: FrameId,
    pub opacity: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    pub layers: Vec<Layer>,
    /// True only while another presentation callback can change opacity.
    pub needs_frame: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    None,
    PresentImmediately,
    Animate,
    /// Capture the frozen `sample()` into one renderer-owned frame, then call
    /// `capture_ready`. This prevents a jump when a crossfade is interrupted.
    Capture {
        id: CaptureId,
    },
}

#[derive(Clone, Debug)]
enum Phase {
    Empty,
    Stable {
        frame: FrameId,
    },
    Fading {
        from: FrameId,
        to: FrameId,
        started: Duration,
        duration: Duration,
    },
    AwaitingCapture {
        id: CaptureId,
        target: FrameId,
        frozen: Sample,
        duration: Duration,
    },
}

#[derive(Clone, Debug)]
pub struct Transition {
    phase: Phase,
    policy: Policy,
    next_capture: u64,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            phase: Phase::Empty,
            policy: Policy::default(),
            next_capture: 0,
        }
    }
}

impl Transition {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn replace(&mut self, frame: FrameId, now: Duration) -> Effect {
        let duration = self.policy.effective_duration();
        match &self.phase {
            Phase::Empty => {
                self.phase = Phase::Stable { frame };
                Effect::PresentImmediately
            }
            Phase::Stable { frame: current } if *current == frame => Effect::None,
            Phase::Stable { frame: current } if duration.is_zero() => {
                let _ = current;
                self.phase = Phase::Stable { frame };
                Effect::PresentImmediately
            }
            Phase::Stable { frame: current } => {
                self.phase = Phase::Fading {
                    from: *current,
                    to: frame,
                    started: now,
                    duration,
                };
                Effect::Animate
            }
            Phase::Fading { to, .. } if *to == frame => Effect::None,
            Phase::Fading { .. } if duration.is_zero() => {
                self.phase = Phase::Stable { frame };
                Effect::PresentImmediately
            }
            Phase::Fading { .. } => {
                let frozen = self.sample_at(now, false);
                self.next_capture = self.next_capture.wrapping_add(1).max(1);
                let id = CaptureId(self.next_capture);
                self.phase = Phase::AwaitingCapture {
                    id,
                    target: frame,
                    frozen,
                    duration,
                };
                Effect::Capture { id }
            }
            Phase::AwaitingCapture { target, .. } if *target == frame => Effect::None,
            Phase::AwaitingCapture { .. } if duration.is_zero() => {
                self.phase = Phase::Stable { frame };
                Effect::PresentImmediately
            }
            Phase::AwaitingCapture {
                id,
                frozen,
                duration: pending_duration,
                ..
            } => {
                self.phase = Phase::AwaitingCapture {
                    id: *id,
                    target: frame,
                    frozen: frozen.clone(),
                    duration: (*pending_duration).min(duration),
                };
                Effect::None
            }
        }
    }

    pub fn capture_ready(&mut self, id: CaptureId, captured: FrameId, now: Duration) -> Effect {
        let Phase::AwaitingCapture {
            id: pending,
            target,
            duration,
            ..
        } = &self.phase
        else {
            return Effect::None;
        };
        if *pending != id {
            return Effect::None;
        }
        let target = *target;
        let duration = *duration;
        if duration.is_zero() || captured == target {
            self.phase = Phase::Stable { frame: target };
            Effect::PresentImmediately
        } else {
            self.phase = Phase::Fading {
                from: captured,
                to: target,
                started: now,
                duration,
            };
            Effect::Animate
        }
    }

    pub fn set_policy(&mut self, policy: Policy) -> Effect {
        self.policy = policy;
        if !policy.reduced_motion {
            return Effect::None;
        }
        let target = match &self.phase {
            Phase::Fading { to, .. } => Some(*to),
            Phase::AwaitingCapture { target, .. } => Some(*target),
            Phase::Empty | Phase::Stable { .. } => None,
        };
        if let Some(frame) = target {
            self.phase = Phase::Stable { frame };
            Effect::PresentImmediately
        } else {
            Effect::None
        }
    }

    pub fn sample(&mut self, now: Duration) -> Sample {
        let sample = self.sample_at(now, true);
        if let Phase::Fading {
            to,
            started,
            duration,
            ..
        } = self.phase
        {
            if now.saturating_sub(started) >= duration {
                self.phase = Phase::Stable { frame: to };
            }
        }
        sample
    }

    pub fn active(&self) -> bool {
        matches!(self.phase, Phase::Fading { .. })
    }

    fn sample_at(&self, now: Duration, settle_end: bool) -> Sample {
        match &self.phase {
            Phase::Empty => Sample::default(),
            Phase::Stable { frame } => Sample {
                layers: vec![Layer {
                    frame: *frame,
                    opacity: 1.0,
                }],
                needs_frame: false,
            },
            Phase::Fading {
                from,
                to,
                started,
                duration,
            } => {
                let elapsed = now.saturating_sub(*started);
                let linear = if duration.is_zero() {
                    1.0
                } else {
                    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
                };
                // Smoothstep avoids a mechanical linear start/stop while
                // remaining deterministic and overshoot-free.
                let progress = linear * linear * (3.0 - 2.0 * linear);
                if settle_end && progress >= 1.0 {
                    Sample {
                        layers: vec![Layer {
                            frame: *to,
                            opacity: 1.0,
                        }],
                        needs_frame: false,
                    }
                } else {
                    Sample {
                        layers: vec![
                            Layer {
                                frame: *from,
                                opacity: 1.0 - progress,
                            },
                            Layer {
                                frame: *to,
                                opacity: progress,
                            },
                        ],
                        needs_frame: progress < 1.0,
                    }
                }
            }
            Phase::AwaitingCapture { frozen, .. } => frozen.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_requests_frames_only_until_settled() {
        let mut transition = Transition::default();
        assert_eq!(
            transition.replace(FrameId(1), Duration::ZERO),
            Effect::PresentImmediately
        );
        assert!(!transition.sample(Duration::ZERO).needs_frame);
        assert_eq!(
            transition.replace(FrameId(2), Duration::from_secs(1)),
            Effect::Animate
        );
        let halfway = transition.sample(Duration::from_millis(1150));
        assert_eq!(halfway.layers[0].opacity, 0.5);
        assert_eq!(halfway.layers[1].opacity, 0.5);
        assert!(halfway.needs_frame);
        let done = transition.sample(Duration::from_millis(1300));
        assert_eq!(
            done.layers,
            vec![Layer {
                frame: FrameId(2),
                opacity: 1.0
            }]
        );
        assert!(!done.needs_frame);
        assert!(!transition.active());
    }

    #[test]
    fn interruption_freezes_current_composite_then_uses_latest_target() {
        let mut transition = Transition::default();
        transition.replace(FrameId(1), Duration::ZERO);
        transition.replace(FrameId(2), Duration::ZERO);
        let before = transition.sample(Duration::from_millis(150));
        let Effect::Capture { id } = transition.replace(FrameId(3), Duration::from_millis(150))
        else {
            panic!("interruption requests one capture");
        };
        assert_eq!(transition.sample(Duration::from_millis(250)), before);
        assert_eq!(
            transition.replace(FrameId(4), Duration::from_millis(250)),
            Effect::None
        );
        assert_eq!(
            transition.capture_ready(CaptureId(id.0 + 1), FrameId(99), Duration::from_millis(250)),
            Effect::None
        );
        assert_eq!(
            transition.capture_ready(id, FrameId(99), Duration::from_millis(250)),
            Effect::Animate
        );
        let sample = transition.sample(Duration::from_millis(250));
        assert_eq!(sample.layers[0].frame, FrameId(99));
        assert_eq!(sample.layers[1].frame, FrameId(4));
    }

    #[test]
    fn reduced_motion_is_immediate_even_when_enabled_mid_transition() {
        let mut transition = Transition::new(Policy {
            reduced_motion: true,
            duration: Duration::from_secs(1),
        });
        transition.replace(FrameId(1), Duration::ZERO);
        assert_eq!(
            transition.replace(FrameId(2), Duration::ZERO),
            Effect::PresentImmediately
        );
        assert!(!transition.active());

        transition.set_policy(Policy::default());
        transition.replace(FrameId(3), Duration::ZERO);
        assert!(transition.active());
        assert_eq!(
            transition.set_policy(Policy {
                reduced_motion: true,
                duration: DEFAULT_DURATION,
            }),
            Effect::PresentImmediately
        );
        assert_eq!(
            transition.sample(Duration::ZERO).layers,
            vec![Layer {
                frame: FrameId(3),
                opacity: 1.0,
            }]
        );
    }

    #[test]
    fn duration_is_bounded_and_replacing_same_frame_is_a_noop() {
        let mut transition = Transition::new(Policy {
            reduced_motion: false,
            duration: Duration::from_secs(60),
        });
        transition.replace(FrameId(1), Duration::ZERO);
        assert_eq!(transition.replace(FrameId(1), Duration::ZERO), Effect::None);
        transition.replace(FrameId(2), Duration::ZERO);
        assert!(transition.sample(Duration::from_millis(1999)).needs_frame);
        assert!(!transition.sample(Duration::from_secs(2)).needs_frame);
    }
}
