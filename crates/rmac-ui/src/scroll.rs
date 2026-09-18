//! Shared scrolling and rubber-band physics (FEEL_SPEC.md §D.4).
//!
//! One implementation is used by every rmac scrollable (Files, Settings, Notes,
//! Control Center, Apps) so momentum and overscroll feel identical everywhere.
//! GPUI delivers raw axis events; a host feeds velocity here and applies the
//! returned delta each frame.

/// Exponential velocity decay time constant, milliseconds.
pub const MOMENTUM_DECAY_MS: f32 = 325.0;
/// A flick is finished once its speed drops below this many logical px/frame.
pub const MOMENTUM_STOP_PX_PER_FRAME: f32 = 0.5;
/// Rubber-band travel is bounded to this fraction of the viewport.
pub const RUBBER_BAND_FRACTION: f32 = 0.25;
/// Spring constants for the snap-back after an overscroll release.
pub const SPRING_STIFFNESS: f32 = 200.0;
pub const SPRING_DAMPING: f32 = 26.0;

/// A momentum flick over a list. Velocity is logical pixels per millisecond.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Momentum {
    velocity: f32,
    active: bool,
}

impl Momentum {
    /// Begin a flick with the velocity measured from the finger at release.
    pub fn flick(&mut self, velocity_px_per_ms: f32) {
        if velocity_px_per_ms.is_finite() && velocity_px_per_ms != 0.0 {
            self.velocity = velocity_px_per_ms;
            self.active = true;
        }
    }

    pub fn cancel(&mut self) {
        self.velocity = 0.0;
        self.active = false;
    }

    pub fn velocity(&self) -> f32 {
        self.velocity
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Advance `dt_ms` and return the scroll delta to apply this frame.
    pub fn step(&mut self, dt_ms: f32) -> f32 {
        if !self.active || !dt_ms.is_finite() || dt_ms <= 0.0 {
            return 0.0;
        }
        self.velocity *= (-dt_ms / MOMENTUM_DECAY_MS).exp();
        let delta = self.velocity * dt_ms;
        if self.velocity.abs() < MOMENTUM_STOP_PX_PER_FRAME {
            self.cancel();
        }
        delta
    }
}

/// The visual offset for dragging past an edge. `overscroll` is the raw
/// distance past the limit; the result approaches (but never reaches)
/// `RUBBER_BAND_FRACTION × viewport`, so the surface resists further pull.
pub fn rubber_band(overscroll: f32, viewport: f32) -> f32 {
    if !overscroll.is_finite() || !viewport.is_finite() || viewport <= 0.0 {
        return 0.0;
    }
    let limit = (viewport * RUBBER_BAND_FRACTION).max(1.0);
    limit * (1.0 - 1.0 / (1.0 + overscroll.abs() / limit)) * overscroll.signum()
}

/// One integration step of the overscroll snap-back spring.
pub fn spring_step(position: f32, velocity: f32, target: f32, dt_s: f32) -> (f32, f32) {
    if !dt_s.is_finite() || dt_s <= 0.0 {
        return (position, velocity);
    }
    let acceleration = -SPRING_STIFFNESS * (position - target) - SPRING_DAMPING * velocity;
    let velocity = velocity + acceleration * dt_s;
    (position + velocity * dt_s, velocity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn momentum_decays_and_then_stops() {
        let mut momentum = Momentum::default();
        assert!(!momentum.is_active());
        momentum.flick(2.0);
        assert!(momentum.is_active());
        let first = momentum.step(16.0);
        assert!(first > 0.0);
        // Velocity strictly decreases over time.
        let before = momentum.velocity();
        momentum.step(16.0);
        assert!(momentum.velocity() < before);
        // Far enough in the future it stops instead of drifting forever.
        for _ in 0..600 {
            momentum.step(16.0);
        }
        assert!(!momentum.is_active());
        assert_eq!(momentum.step(16.0), 0.0);
    }

    #[test]
    fn rubber_band_is_bounded_and_signed() {
        let viewport = 800.0;
        let limit = viewport * RUBBER_BAND_FRACTION;
        assert_eq!(rubber_band(0.0, viewport), 0.0);
        let pull = rubber_band(limit * 10.0, viewport);
        assert!(pull > 0.0 && pull < limit, "pull was {pull}");
        assert!(rubber_band(-limit * 10.0, viewport) < 0.0);
        // Monotonic: more drag never moves you less.
        assert!(rubber_band(100.0, viewport) < rubber_band(200.0, viewport));
    }

    #[test]
    fn spring_converges_to_the_edge() {
        let (mut position, mut velocity) = (120.0_f32, 0.0_f32);
        for _ in 0..600 {
            let (next_position, next_velocity) = spring_step(position, velocity, 0.0, 1.0 / 60.0);
            position = next_position;
            velocity = next_velocity;
        }
        assert!(position.abs() < 0.5, "spring settled at {position}");
        assert!(velocity.abs() < 1.0);
    }
}
