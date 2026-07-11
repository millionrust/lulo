//! Deterministic Dock magnification and event-driven visibility policy.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagnificationConfig {
    pub icon_size: f32,
    pub gap: f32,
    pub influence_radius: f32,
    pub maximum_scale: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    NonFinite,
    IconSize,
    Gap,
    InfluenceRadius,
    MaximumScale,
}

impl MagnificationConfig {
    pub fn validate(self) -> Result<Self, ConfigError> {
        if ![
            self.icon_size,
            self.gap,
            self.influence_radius,
            self.maximum_scale,
        ]
        .into_iter()
        .all(f32::is_finite)
        {
            return Err(ConfigError::NonFinite);
        }
        if !(16.0..=128.0).contains(&self.icon_size) {
            return Err(ConfigError::IconSize);
        }
        if !(0.0..=32.0).contains(&self.gap) {
            return Err(ConfigError::Gap);
        }
        if self.influence_radius < self.icon_size || self.influence_radius > self.icon_size * 6.0 {
            return Err(ConfigError::InfluenceRadius);
        }
        if !(1.0..=2.5).contains(&self.maximum_scale) {
            return Err(ConfigError::MaximumScale);
        }
        Ok(self)
    }
}

impl Default for MagnificationConfig {
    fn default() -> Self {
        Self {
            icon_size: 48.0,
            gap: 8.0,
            influence_radius: 112.0,
            maximum_scale: 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagnifiedItem {
    pub center: f32,
    pub size: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MagnifiedLayout {
    pub items: Vec<MagnifiedItem>,
    pub start: f32,
    pub end: f32,
}

impl MagnifiedLayout {
    pub fn extent(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// Lay out one-dimensional Dock slots. Scale is calculated from stable base
/// centers, then expanded away from the nearest anchor, so recalculated centers
/// never feed back into scale and create pointer oscillation.
pub fn magnified_layout(
    item_count: usize,
    pointer: Option<f32>,
    enabled: bool,
    reduced_motion: bool,
    config: MagnificationConfig,
) -> Result<MagnifiedLayout, ConfigError> {
    let config = config.validate()?;
    if item_count == 0 {
        return Ok(MagnifiedLayout::default());
    }
    let stride = config.icon_size + config.gap;
    let base_centers: Vec<_> = (0..item_count)
        .map(|index| config.icon_size / 2.0 + index as f32 * stride)
        .collect();
    let magnifies = enabled && !reduced_motion;
    let scales: Vec<_> = base_centers
        .iter()
        .map(|center| match (magnifies, pointer) {
            (true, Some(pointer)) => {
                let proximity =
                    (1.0 - (pointer - center).abs() / config.influence_radius).clamp(0.0, 1.0);
                let eased = proximity * proximity * (3.0 - 2.0 * proximity);
                1.0 + (config.maximum_scale - 1.0) * eased
            }
            _ => 1.0,
        })
        .collect();
    let sizes: Vec<_> = scales
        .iter()
        .map(|scale| config.icon_size * scale)
        .collect();
    let anchor = pointer
        .map(|pointer| {
            base_centers
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    (pointer - **left)
                        .abs()
                        .total_cmp(&(pointer - **right).abs())
                })
                .map(|(index, _)| index)
                .expect("a nonempty layout has a nearest item")
        })
        .unwrap_or(item_count / 2);
    let mut centers = vec![0.0; item_count];
    centers[anchor] = base_centers[anchor];
    for index in anchor + 1..item_count {
        centers[index] =
            centers[index - 1] + sizes[index - 1] / 2.0 + config.gap + sizes[index] / 2.0;
    }
    for index in (0..anchor).rev() {
        centers[index] =
            centers[index + 1] - sizes[index + 1] / 2.0 - config.gap - sizes[index] / 2.0;
    }
    let items: Vec<_> = centers
        .into_iter()
        .zip(sizes)
        .zip(scales)
        .map(|((center, size), scale)| MagnifiedItem {
            center,
            size,
            scale,
        })
        .collect();
    let start = items[0].center - items[0].size / 2.0;
    let last = items.last().expect("nonempty items");
    let end = last.center + last.size / 2.0;
    Ok(MagnifiedLayout { items, start, end })
}

pub const HIDE_DELAY_MS: u64 = 500;
pub const REVEAL_PRESSURE_MS: u64 = 150;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Visibility {
    #[default]
    Visible,
    WaitingToHide {
        deadline_ms: u64,
    },
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisibilityEvent {
    Configure {
        autohide: bool,
        reduced_motion: bool,
    },
    PointerEntered,
    PointerLeft {
        now_ms: u64,
    },
    RevealPressure {
        now_ms: u64,
    },
    RevealPressureEnded,
    OverviewChanged {
        visible: bool,
        now_ms: u64,
    },
    FullscreenChanged {
        visible: bool,
        now_ms: u64,
    },
    Deadline {
        now_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VisibilityUpdate {
    pub visibility: Visibility,
    pub state_changed: bool,
    /// True only when the rendered shelf crosses the hidden boundary.
    pub visual_changed: bool,
    pub schedule_deadline_ms: Option<u64>,
    pub animate: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VisibilityMachine {
    visibility: Visibility,
    autohide: bool,
    reduced_motion: bool,
    pointer_inside: bool,
    overview: bool,
    fullscreen: bool,
    pressure_started_ms: Option<u64>,
}

impl VisibilityMachine {
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    pub fn apply(&mut self, event: VisibilityEvent) -> VisibilityUpdate {
        let before = self.visibility;
        let mut schedule = None;
        match event {
            VisibilityEvent::Configure {
                autohide,
                reduced_motion,
            } => {
                self.autohide = autohide;
                self.reduced_motion = reduced_motion;
                if !self.effective_autohide() || self.overview || self.pointer_inside {
                    self.visibility = Visibility::Visible;
                }
            }
            VisibilityEvent::PointerEntered => {
                self.pointer_inside = true;
                self.pressure_started_ms = None;
                self.visibility = Visibility::Visible;
            }
            VisibilityEvent::PointerLeft { now_ms } => {
                self.pointer_inside = false;
                self.pressure_started_ms = None;
                schedule = self.begin_hide(now_ms);
            }
            VisibilityEvent::RevealPressure { now_ms } => {
                if self.visibility == Visibility::Hidden && !self.overview {
                    let started = *self.pressure_started_ms.get_or_insert(now_ms);
                    if now_ms.saturating_sub(started) >= REVEAL_PRESSURE_MS {
                        self.visibility = Visibility::Visible;
                        self.pointer_inside = true;
                        self.pressure_started_ms = None;
                    }
                }
            }
            VisibilityEvent::RevealPressureEnded => self.pressure_started_ms = None,
            VisibilityEvent::OverviewChanged { visible, now_ms } => {
                self.overview = visible;
                self.pressure_started_ms = None;
                if visible {
                    self.visibility = Visibility::Visible;
                } else if !self.pointer_inside {
                    schedule = self.begin_hide(now_ms);
                }
            }
            VisibilityEvent::FullscreenChanged { visible, now_ms } => {
                self.fullscreen = visible;
                self.pressure_started_ms = None;
                if !self.effective_autohide() || self.overview || self.pointer_inside {
                    self.visibility = Visibility::Visible;
                } else {
                    schedule = self.begin_hide(now_ms);
                }
            }
            VisibilityEvent::Deadline { now_ms } => {
                if let Visibility::WaitingToHide { deadline_ms } = self.visibility {
                    if now_ms >= deadline_ms
                        && self.effective_autohide()
                        && !self.pointer_inside
                        && !self.overview
                    {
                        self.visibility = Visibility::Hidden;
                    } else if now_ms < deadline_ms {
                        schedule = Some(deadline_ms);
                    }
                }
            }
        }
        let visual_changed = is_hidden(self.visibility) != is_hidden(before);
        VisibilityUpdate {
            visibility: self.visibility,
            state_changed: self.visibility != before,
            visual_changed,
            schedule_deadline_ms: schedule,
            animate: visual_changed && !self.reduced_motion,
        }
    }

    fn effective_autohide(&self) -> bool {
        self.autohide || self.fullscreen
    }

    fn begin_hide(&mut self, now_ms: u64) -> Option<u64> {
        if self.effective_autohide() && !self.overview && !self.pointer_inside {
            let deadline = now_ms.saturating_add(HIDE_DELAY_MS);
            self.visibility = Visibility::WaitingToHide {
                deadline_ms: deadline,
            };
            Some(deadline)
        } else {
            self.visibility = Visibility::Visible;
            None
        }
    }
}

fn is_hidden(visibility: Visibility) -> bool {
    visibility == Visibility::Hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnification_is_symmetric_bounded_and_non_overlapping() {
        let config = MagnificationConfig::default();
        let pointer = config.icon_size / 2.0 + 2.0 * (config.icon_size + config.gap);
        let layout = magnified_layout(5, Some(pointer), true, false, config).expect("layout");
        assert_eq!(layout.items[2].scale, config.maximum_scale);
        assert_eq!(layout.items[1].scale, layout.items[3].scale);
        assert!(layout
            .items
            .iter()
            .all(|item| (1.0..=config.maximum_scale).contains(&item.scale)));
        for pair in layout.items.windows(2) {
            let left_edge = pair[0].center + pair[0].size / 2.0;
            let right_edge = pair[1].center - pair[1].size / 2.0;
            assert!(right_edge - left_edge >= config.gap - f32::EPSILON);
        }
    }

    #[test]
    fn reduced_motion_and_disabled_magnification_keep_stable_slots() {
        let config = MagnificationConfig::default();
        for (enabled, reduced) in [(false, false), (true, true)] {
            let layout = magnified_layout(3, Some(56.0), enabled, reduced, config).expect("layout");
            assert!(layout.items.iter().all(|item| item.scale == 1.0));
            assert_eq!(layout.extent(), 3.0 * config.icon_size + 2.0 * config.gap);
        }
    }

    #[test]
    fn scale_uses_stable_centers_so_repeated_layout_does_not_oscillate() {
        let config = MagnificationConfig::default();
        let first = magnified_layout(7, Some(143.25), true, false, config).expect("layout");
        let second = magnified_layout(7, Some(143.25), true, false, config).expect("layout");
        assert_eq!(first, second);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_rendering() {
        let invalid = MagnificationConfig {
            maximum_scale: f32::NAN,
            ..Default::default()
        };
        assert_eq!(
            magnified_layout(3, None, true, false, invalid),
            Err(ConfigError::NonFinite)
        );
    }

    #[test]
    fn autohide_uses_one_deadline_and_pressure_dwell() {
        let mut machine = VisibilityMachine::default();
        machine.apply(VisibilityEvent::Configure {
            autohide: true,
            reduced_motion: false,
        });
        let leaving = machine.apply(VisibilityEvent::PointerLeft { now_ms: 100 });
        assert_eq!(leaving.schedule_deadline_ms, Some(600));
        assert!(!leaving.visual_changed);
        assert!(!leaving.animate);
        assert_eq!(
            machine.visibility(),
            Visibility::WaitingToHide { deadline_ms: 600 }
        );
        machine.apply(VisibilityEvent::Deadline { now_ms: 599 });
        assert_ne!(machine.visibility(), Visibility::Hidden);
        let hidden = machine.apply(VisibilityEvent::Deadline { now_ms: 600 });
        assert_eq!(machine.visibility(), Visibility::Hidden);
        assert!(hidden.visual_changed);
        assert!(hidden.animate);

        machine.apply(VisibilityEvent::RevealPressure { now_ms: 1_000 });
        machine.apply(VisibilityEvent::RevealPressure { now_ms: 1_149 });
        assert_eq!(machine.visibility(), Visibility::Hidden);
        machine.apply(VisibilityEvent::RevealPressure { now_ms: 1_150 });
        assert_eq!(machine.visibility(), Visibility::Visible);
    }

    #[test]
    fn overview_is_visible_and_fullscreen_enables_hide_policy() {
        let mut machine = VisibilityMachine::default();
        machine.apply(VisibilityEvent::Configure {
            autohide: false,
            reduced_motion: false,
        });
        machine.apply(VisibilityEvent::PointerLeft { now_ms: 0 });
        assert_eq!(machine.visibility(), Visibility::Visible);
        machine.apply(VisibilityEvent::FullscreenChanged {
            visible: true,
            now_ms: 10,
        });
        assert!(matches!(
            machine.visibility(),
            Visibility::WaitingToHide { .. }
        ));
        machine.apply(VisibilityEvent::OverviewChanged {
            visible: true,
            now_ms: 20,
        });
        assert_eq!(machine.visibility(), Visibility::Visible);
    }

    #[test]
    fn reduced_motion_disables_visibility_animation_not_behavior() {
        let mut machine = VisibilityMachine::default();
        machine.apply(VisibilityEvent::Configure {
            autohide: true,
            reduced_motion: true,
        });
        machine.apply(VisibilityEvent::PointerLeft { now_ms: 0 });
        let hidden = machine.apply(VisibilityEvent::Deadline {
            now_ms: HIDE_DELAY_MS,
        });
        assert_eq!(hidden.visibility, Visibility::Hidden);
        assert!(!hidden.animate);
    }
}
