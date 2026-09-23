//! Stable, renderer-neutral direct manipulation for pinned Dock applications.

use std::fmt;

use crate::presentation::{EntryId, LayoutError, ShelfLayoutPlan};
use crate::{Model, PinCommand};

pub const DRAG_THRESHOLD: f32 = 4.0;
pub const MAX_DRAG_PINNED_APPS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragError {
    InvalidCoordinate,
    NotPinnedApplication,
    EntryNotVisible,
    MalformedLayout,
    TooManyPinnedApplications { count: usize },
    Layout(LayoutError),
}

impl fmt::Display for DragError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCoordinate => formatter.write_str("the Dock drag coordinate is invalid"),
            Self::NotPinnedApplication => {
                formatter.write_str("only a pinned Dock application can be reordered")
            }
            Self::EntryNotVisible => {
                formatter.write_str("the dragged Dock application is not visible")
            }
            Self::MalformedLayout => {
                formatter.write_str("the Dock drag layout does not match the pinned order")
            }
            Self::TooManyPinnedApplications { count } => write!(
                formatter,
                "the Dock has {count} pinned applications; drag supports at most {MAX_DRAG_PINNED_APPS}"
            ),
            Self::Layout(error) => write!(formatter, "the Dock drag layout is invalid: {error}"),
        }
    }
}

impl std::error::Error for DragError {}

impl From<LayoutError> for DragError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ReorderIntent {
    app_id: String,
    expected_order: Vec<String>,
    destination_index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevalidatedReorder {
    command: PinCommand,
    expected_order: Vec<rmac_shell_settings::AppId>,
}

impl RevalidatedReorder {
    pub fn command(&self) -> &PinCommand {
        &self.command
    }

    pub fn expected_order(&self) -> &[rmac_shell_settings::AppId] {
        &self.expected_order
    }
}

impl fmt::Debug for ReorderIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReorderIntent")
            .field("app_id", &self.app_id)
            .field("expected_count", &self.expected_order.len())
            .field("destination_index", &self.destination_index)
            .finish()
    }
}

impl ReorderIntent {
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn destination_index(&self) -> usize {
        self.destination_index
    }

    /// Convert the accepted drag into a pin command only while the exact
    /// pinned order that the user manipulated is still authoritative.
    pub fn revalidate(&self, model: &Model) -> Option<RevalidatedReorder> {
        let current = pinned_order(model);
        if current != self.expected_order
            || self.destination_index >= current.len()
            || !current.iter().any(|app_id| app_id == &self.app_id)
        {
            return None;
        }
        Some(RevalidatedReorder {
            command: PinCommand::MoveTo {
                app_id: self.app_id.clone(),
                index: self.destination_index,
            },
            expected_order: current
                .into_iter()
                .map(rmac_shell_settings::AppId)
                .collect(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DropOutcome {
    /// Movement never crossed the drag threshold; route the original primary
    /// click through normal activation.
    Click {
        entry: EntryId,
    },
    /// A real drag returned to the original stable slot.
    NoChange,
    Cancelled,
    Reorder(ReorderIntent),
}

#[derive(Debug)]
pub struct DragUpdate<'a> {
    pub active: bool,
    pub destination_index: usize,
    /// Borrowed cached order; unchanged pointer frames do not clone app IDs.
    pub preview_order: &'a [String],
    pub visual_changed: bool,
}

#[derive(Debug)]
pub struct DragSession {
    app_id: String,
    expected_order: Vec<String>,
    preview_order: Vec<String>,
    source_index: usize,
    destination_index: usize,
    press_axis: f32,
    candidates: Vec<(usize, f32)>,
    active: bool,
}

impl DragSession {
    /// Begin from the retained shelf plan. Candidate centers always come from
    /// its resting layout, so magnified render positions never feed back into
    /// reorder thresholds.
    pub fn begin(
        model: &Model,
        plan: &ShelfLayoutPlan,
        entry: &EntryId,
        press_axis: f32,
    ) -> Result<Self, DragError> {
        if !press_axis.is_finite() {
            return Err(DragError::InvalidCoordinate);
        }
        plan.layout(Some(press_axis))?;
        let EntryId::Application(app_id) = entry else {
            return Err(DragError::NotPinnedApplication);
        };
        let expected_order = pinned_order(model);
        if expected_order.len() > MAX_DRAG_PINNED_APPS {
            return Err(DragError::TooManyPinnedApplications {
                count: expected_order.len(),
            });
        }
        let Some(source_index) = expected_order.iter().position(|current| current == app_id) else {
            return Err(DragError::NotPinnedApplication);
        };
        let resting = plan.layout(None)?;
        let mut candidates = Vec::new();
        let mut previous_index = None;
        for slot in &resting.slots {
            let EntryId::Application(visible_id) = &slot.id else {
                continue;
            };
            let Some(index) = expected_order
                .iter()
                .position(|current| current == visible_id)
            else {
                continue;
            };
            if previous_index.is_some_and(|previous| index <= previous) || !slot.center.is_finite()
            {
                return Err(DragError::MalformedLayout);
            }
            previous_index = Some(index);
            candidates.push((index, slot.center));
        }
        if !candidates.iter().any(|(index, _)| *index == source_index) {
            return Err(DragError::EntryNotVisible);
        }
        Ok(Self {
            app_id: app_id.clone(),
            preview_order: expected_order.clone(),
            expected_order,
            source_index,
            destination_index: source_index,
            press_axis,
            candidates,
            active: false,
        })
    }

    pub fn update(&mut self, pointer_axis: f32) -> Result<DragUpdate<'_>, DragError> {
        if !pointer_axis.is_finite() {
            return Err(DragError::InvalidCoordinate);
        }
        let was_active = self.active;
        if !self.active && (pointer_axis - self.press_axis).abs() >= DRAG_THRESHOLD {
            self.active = true;
        }
        let mut destination_changed = false;
        if self.active {
            let destination_index = self
                .candidates
                .iter()
                .min_by(|(_, left), (_, right)| {
                    (pointer_axis - *left)
                        .abs()
                        .total_cmp(&(pointer_axis - *right).abs())
                })
                .map(|(index, _)| *index)
                .expect("a visible source guarantees one drag candidate");
            if destination_index != self.destination_index {
                self.destination_index = destination_index;
                self.preview_order.clone_from(&self.expected_order);
                let moved = self.preview_order.remove(self.source_index);
                self.preview_order.insert(destination_index, moved);
                destination_changed = true;
            }
        }
        Ok(DragUpdate {
            active: self.active,
            destination_index: self.destination_index,
            preview_order: &self.preview_order,
            visual_changed: was_active != self.active || destination_changed,
        })
    }

    pub fn finish(self) -> DropOutcome {
        if !self.active {
            return DropOutcome::Click {
                entry: EntryId::Application(self.app_id),
            };
        }
        if self.destination_index == self.source_index {
            return DropOutcome::NoChange;
        }
        DropOutcome::Reorder(ReorderIntent {
            app_id: self.app_id,
            expected_order: self.expected_order,
            destination_index: self.destination_index,
        })
    }

    pub fn cancel(self) -> DropOutcome {
        DropOutcome::Cancelled
    }
}

fn pinned_order(model: &Model) -> Vec<String> {
    model
        .items
        .iter()
        .filter(|item| item.pinned)
        .map(|item| item.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::presentation::ShelfContent;

    fn application(id: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: id.trim_end_matches(".desktop").into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    fn model(order: &[&str]) -> Model {
        let pinned = order
            .iter()
            .map(|id| rmac_shell_settings::AppId((*id).into()))
            .collect::<Vec<_>>();
        let catalog = order.iter().map(|id| application(id)).collect::<Vec<_>>();
        Model::build(&pinned, &Default::default(), &catalog, &Default::default())
    }

    fn plan(model: &Model) -> ShelfLayoutPlan {
        ShelfContent::project(model)
            .prepare_layout(&crate::SurfaceDescription {
                output: rmac_compositor::OutputId::from("eDP-1"),
                placement: rmac_shell_settings::DockPlacement::Bottom,
                output_axis_length: 800.0,
                output_scale: 2.0,
                base_thickness: 64.0,
                maximum_thickness: 88.0,
                exclusive_zone: 64.0,
                reveal_edge_thickness: 0.0,
                keyboard_interactive: false,
                autohide: false,
                overview_visible: false,
                magnification_enabled: true,
                animate: true,
                magnification: crate::motion::MagnificationConfig::default(),
            })
            .unwrap()
    }

    #[test]
    fn movement_below_threshold_remains_the_original_primary_click() {
        let model = model(&["finder.desktop", "terminal.desktop"]);
        let plan = plan(&model);
        let resting = plan.layout(None).unwrap();
        let press = resting.slots[0].center;
        let mut drag = DragSession::begin(
            &model,
            &plan,
            &EntryId::Application("finder.desktop".into()),
            press,
        )
        .unwrap();
        let update = drag.update(press + DRAG_THRESHOLD - 0.1).unwrap();

        assert!(!update.active);
        assert!(!update.visual_changed);
        assert_eq!(
            drag.finish(),
            DropOutcome::Click {
                entry: EntryId::Application("finder.desktop".into())
            }
        );
    }

    #[test]
    fn drag_uses_stable_centers_and_caches_the_preview_order() {
        let model = model(&["finder.desktop", "terminal.desktop", "notes.desktop"]);
        let plan = plan(&model);
        let resting = plan.layout(None).unwrap();
        let press = resting.slots[0].center;
        let destination = resting.slots[2].center;
        let mut drag = DragSession::begin(
            &model,
            &plan,
            &EntryId::Application("finder.desktop".into()),
            press,
        )
        .unwrap();
        let first = drag.update(destination).unwrap();
        assert!(first.active);
        assert!(first.visual_changed);
        assert_eq!(
            first.preview_order,
            ["terminal.desktop", "notes.desktop", "finder.desktop"]
        );
        let replay = drag.update(destination + 0.5).unwrap();
        assert!(!replay.visual_changed);
        assert_eq!(replay.destination_index, 2);

        let DropOutcome::Reorder(intent) = drag.finish() else {
            panic!("drag produces a reorder intent");
        };
        assert_eq!(intent.app_id(), "finder.desktop");
        assert_eq!(intent.destination_index(), 2);
        let validated = intent.revalidate(&model).unwrap();
        assert_eq!(
            validated.command(),
            &PinCommand::MoveTo {
                app_id: "finder.desktop".into(),
                index: 2,
            }
        );
        assert_eq!(validated.expected_order().len(), 3);
    }

    #[test]
    fn accepted_intent_fails_closed_after_the_pinned_order_changes() {
        let original = model(&["finder.desktop", "terminal.desktop", "notes.desktop"]);
        let plan = plan(&original);
        let resting = plan.layout(None).unwrap();
        let mut drag = DragSession::begin(
            &original,
            &plan,
            &EntryId::Application("finder.desktop".into()),
            resting.slots[0].center,
        )
        .unwrap();
        drag.update(resting.slots[2].center).unwrap();
        let DropOutcome::Reorder(intent) = drag.finish() else {
            panic!("drag produces an intent");
        };
        let changed = model(&["finder.desktop", "notes.desktop", "terminal.desktop"]);

        assert_eq!(intent.revalidate(&changed), None);
    }

    #[test]
    fn non_applications_unpinned_apps_and_invalid_coordinates_cannot_drag() {
        let pinned = model(&["finder.desktop"]);
        let pinned_plan = plan(&pinned);
        let resting = pinned_plan.layout(None).unwrap();
        assert_eq!(
            DragSession::begin(
                &pinned,
                &pinned_plan,
                &EntryId::Special(crate::SpecialItemKind::Files),
                resting.slots[0].center,
            )
            .unwrap_err(),
            DragError::NotPinnedApplication
        );
        assert_eq!(
            DragSession::begin(
                &pinned,
                &pinned_plan,
                &EntryId::Application("finder.desktop".into()),
                f32::NAN,
            )
            .unwrap_err(),
            DragError::InvalidCoordinate
        );
        let running = Model {
            items: vec![crate::Item {
                id: "terminal.desktop".into(),
                name: "Terminal".into(),
                icon: None,
                pinned: false,
                running: true,
                active: false,
                urgent: false,
                launchable: true,
                windows: Vec::new(),
                launch: None,
                source: None,
                actions: Vec::new(),
                mime_types: Vec::new(),
            }],
            ..Default::default()
        };
        let running_plan = plan(&running);
        assert_eq!(
            DragSession::begin(
                &running,
                &running_plan,
                &EntryId::Application("terminal.desktop".into()),
                running_plan.layout(None).unwrap().slots[0].center,
            )
            .unwrap_err(),
            DragError::NotPinnedApplication
        );
    }
}
