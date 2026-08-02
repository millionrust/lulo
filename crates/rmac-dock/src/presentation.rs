//! Renderer-neutral Dock content, semantics, and original embedded assets.

use std::fmt;
use std::path::PathBuf;

use crate::{motion, Item, Model, SpecialItem, SpecialItemKind, SurfaceDescription};

pub const SHELF_AXIS_PADDING: f32 = 8.0;
pub const GROUP_GAP: f32 = 24.0;
pub const MINIMUM_ICON_SIZE: f32 = 36.0;
pub const MINIMUM_ICON_GAP: f32 = 4.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinIcon {
    Application,
    Files,
    Downloads,
    TrashEmpty,
    TrashFull,
    More,
}

impl BuiltinIcon {
    /// A self-contained original vector asset. Keeping these embedded avoids
    /// install-path races while a Dock surface is starting or being restored.
    pub fn svg(self) -> &'static str {
        match self {
            Self::Application => include_str!("../assets/icons/application.svg"),
            Self::Files => include_str!("../assets/icons/files.svg"),
            Self::Downloads => include_str!("../assets/icons/downloads.svg"),
            Self::TrashEmpty => include_str!("../assets/icons/trash-empty.svg"),
            Self::TrashFull => include_str!("../assets/icons/trash-full.svg"),
            Self::More => include_str!("../assets/icons/more.svg"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Icon {
    File(PathBuf),
    Builtin(BuiltinIcon),
}

impl fmt::Debug for Icon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(_) => formatter.write_str("File(<private>)"),
            Self::Builtin(icon) => formatter.debug_tuple("Builtin").field(icon).finish(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryId {
    Application(String),
    Special(SpecialItemKind),
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityIndicator {
    None,
    Running,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub id: EntryId,
    pub label: String,
    /// Complete, path-free state for the renderer's accessible name.
    pub accessible_label: String,
    pub icon: Icon,
    /// False means the item remains visible but cannot be activated.
    pub enabled: bool,
    pub activity: ActivityIndicator,
    pub urgent: bool,
    /// Exact authoritative count. Rendering may visually abbreviate it, but
    /// must preserve the exact count in the accessible label.
    pub badge: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShelfContent {
    pub applications: Vec<Entry>,
    pub places: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub id: EntryId,
    pub center: f32,
    pub size: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShelfLayout {
    pub slots: Vec<Slot>,
    pub start: f32,
    pub end: f32,
    /// Center of the noninteractive separator between applications and places.
    pub separator_axis: Option<f32>,
    /// Stable fitting policy selected before pointer magnification is applied.
    pub effective_magnification: motion::MagnificationConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverflowGroup {
    pub entry: Entry,
    /// Complete hidden tail in original Dock order for an accessible popover.
    pub hidden_applications: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShelfLayoutPlan {
    axis: f32,
    available: f32,
    ids: Vec<EntryId>,
    application_group_len: usize,
    place_count: usize,
    gaps: Vec<f32>,
    desired_shift: f32,
    magnification_enabled: bool,
    reduced_motion: bool,
    pub effective_magnification: motion::MagnificationConfig,
    /// Present only when a narrow output cannot show the complete application
    /// group at the minimum visual size. It is prepared once, not cloned for
    /// every pointer frame.
    pub overflow: Option<OverflowGroup>,
}

impl ShelfLayout {
    pub fn hit_test(&self, axis: f32) -> Option<&EntryId> {
        axis.is_finite().then_some(())?;
        self.slots
            .iter()
            .find(|slot| (axis - slot.center).abs() <= slot.size / 2.0)
            .map(|slot| &slot.id)
    }
}

impl ShelfLayoutPlan {
    pub fn visible_ids(&self) -> &[EntryId] {
        &self.ids
    }

    /// Project one pointer frame without repeating output fitting, overflow
    /// selection, or hidden-entry cloning.
    pub fn layout(&self, pointer_axis: Option<f32>) -> Result<ShelfLayout, LayoutError> {
        if pointer_axis
            .is_some_and(|pointer| !pointer.is_finite() || !(0.0..=self.axis).contains(&pointer))
        {
            return Err(LayoutError::InvalidPointer);
        }
        if self.ids.is_empty() {
            return Ok(ShelfLayout {
                start: self.axis / 2.0,
                end: self.axis / 2.0,
                effective_magnification: self.effective_magnification,
                ..Default::default()
            });
        }
        let relative_pointer = pointer_axis.map(|pointer| pointer - self.desired_shift);
        let layout = motion::magnified_layout_with_gaps(
            self.ids.len(),
            &self.gaps,
            relative_pointer,
            self.magnification_enabled,
            self.reduced_motion,
            self.effective_magnification,
        )?;
        ensure_fits(layout.extent(), self.available)?;

        let minimum_shift = SHELF_AXIS_PADDING - layout.start;
        let maximum_shift = self.axis - SHELF_AXIS_PADDING - layout.end;
        let shift = self.desired_shift.clamp(minimum_shift, maximum_shift);
        let slots = self
            .ids
            .iter()
            .cloned()
            .zip(layout.items)
            .map(|(id, geometry)| Slot {
                id,
                center: geometry.center + shift,
                size: geometry.size,
                scale: geometry.scale,
            })
            .collect::<Vec<_>>();
        let separator_axis = (self.application_group_len > 0 && self.place_count > 0).then(|| {
            let left = &slots[self.application_group_len - 1];
            let right = &slots[self.application_group_len];
            ((left.center + left.size / 2.0) + (right.center - right.size / 2.0)) / 2.0
        });
        Ok(ShelfLayout {
            start: layout.start + shift,
            end: layout.end + shift,
            slots,
            separator_axis,
            effective_magnification: self.effective_magnification,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayoutError {
    InvalidAxis,
    InvalidPointer,
    DoesNotFit { required: f32, available: f32 },
    Magnification(motion::ConfigError),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAxis => formatter.write_str("Dock output axis is invalid"),
            Self::InvalidPointer => formatter.write_str("Dock pointer coordinate is invalid"),
            Self::DoesNotFit {
                required,
                available,
            } => write!(
                formatter,
                "Dock content requires {required} logical pixels but only {available} are available"
            ),
            Self::Magnification(error) => {
                write!(formatter, "invalid Dock magnification: {error:?}")
            }
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<motion::ConfigError> for LayoutError {
    fn from(error: motion::ConfigError) -> Self {
        Self::Magnification(error)
    }
}

impl ShelfContent {
    pub fn project(model: &Model) -> Self {
        Self {
            applications: model.items.iter().map(application_entry).collect(),
            places: model.special_items.iter().map(special_entry).collect(),
        }
    }

    pub fn has_separator(&self) -> bool {
        !self.applications.is_empty() && !self.places.is_empty()
    }

    /// Produce output-axis geometry from stable base centers. Pointer distance
    /// never uses a previously rendered center, including when the final shelf
    /// is shifted just enough to remain inside a physical output edge.
    pub fn layout(
        &self,
        surface: &SurfaceDescription,
        pointer_axis: Option<f32>,
    ) -> Result<ShelfLayout, LayoutError> {
        self.prepare_layout(surface)?.layout(pointer_axis)
    }

    /// Fit content once per coherent content/output policy change. Renderers
    /// retain this plan and call `ShelfLayoutPlan::layout` for pointer frames.
    pub fn prepare_layout(
        &self,
        surface: &SurfaceDescription,
    ) -> Result<ShelfLayoutPlan, LayoutError> {
        if !surface.output_axis_length.is_finite()
            || surface.output_axis_length <= 0.0
            || surface.output_axis_length > f32::MAX as f64
        {
            return Err(LayoutError::InvalidAxis);
        }
        let axis = surface.output_axis_length as f32;
        let requested_magnification = surface.magnification.validate()?;
        if self.applications.is_empty() && self.places.is_empty() {
            return Ok(ShelfLayoutPlan {
                axis,
                available: axis,
                ids: Vec::new(),
                application_group_len: 0,
                place_count: 0,
                gaps: Vec::new(),
                desired_shift: axis / 2.0,
                magnification_enabled: surface.magnification_enabled,
                reduced_motion: !surface.animate,
                effective_magnification: requested_magnification,
                overflow: None,
            });
        }
        let available = axis - 2.0 * SHELF_AXIS_PADDING;
        if available <= 0.0 {
            return Err(LayoutError::InvalidAxis);
        }
        let selection = select_entries(
            self,
            requested_magnification,
            surface.magnification_enabled && surface.animate,
            available,
        )?;
        let gaps = layout_gaps(
            selection.ids.len(),
            selection.application_group_len,
            self.places.len(),
            selection.magnification.gap,
        );
        let base = motion::magnified_layout_with_gaps(
            selection.ids.len(),
            &gaps,
            None,
            false,
            false,
            selection.magnification,
        )?;
        let desired_shift = (axis - base.extent()) / 2.0 - base.start;
        Ok(ShelfLayoutPlan {
            axis,
            available,
            ids: selection.ids,
            application_group_len: selection.application_group_len,
            place_count: self.places.len(),
            gaps,
            desired_shift,
            magnification_enabled: surface.magnification_enabled,
            reduced_motion: !surface.animate,
            effective_magnification: selection.magnification,
            overflow: selection.overflow,
        })
    }
}

struct LayoutSelection {
    ids: Vec<EntryId>,
    application_group_len: usize,
    magnification: motion::MagnificationConfig,
    overflow: Option<OverflowGroup>,
}

fn select_entries(
    content: &ShelfContent,
    requested: motion::MagnificationConfig,
    magnifies: bool,
    available: f32,
) -> Result<LayoutSelection, LayoutError> {
    if let Some(magnification) = fit_magnification(
        content.applications.len(),
        content.places.len(),
        requested,
        magnifies,
        available,
    ) {
        return Ok(LayoutSelection {
            ids: content
                .applications
                .iter()
                .chain(content.places.iter())
                .map(|entry| entry.id.clone())
                .collect(),
            application_group_len: content.applications.len(),
            magnification,
            overflow: None,
        });
    }

    if !content.applications.is_empty() {
        for visible_applications in (0..content.applications.len()).rev() {
            let application_group_len = visible_applications + 1;
            if let Some(magnification) = fit_magnification(
                application_group_len,
                content.places.len(),
                requested,
                magnifies,
                available,
            ) {
                let overflow =
                    overflow_group(content.applications[visible_applications..].to_vec());
                let ids = content.applications[..visible_applications]
                    .iter()
                    .map(|entry| entry.id.clone())
                    .chain(std::iter::once(EntryId::Overflow))
                    .chain(content.places.iter().map(|entry| entry.id.clone()))
                    .collect();
                return Ok(LayoutSelection {
                    ids,
                    application_group_len,
                    magnification,
                    overflow: Some(overflow),
                });
            }
        }
    }

    let minimum = compact_magnification(requested, requested.icon_size.min(MINIMUM_ICON_SIZE));
    let application_group_len = usize::from(!content.applications.is_empty());
    Err(LayoutError::DoesNotFit {
        required: reserved_extent(
            application_group_len,
            content.places.len(),
            minimum,
            magnifies,
        ),
        available,
    })
}

fn fit_magnification(
    application_count: usize,
    place_count: usize,
    requested: motion::MagnificationConfig,
    magnifies: bool,
    available: f32,
) -> Option<motion::MagnificationConfig> {
    if reserved_extent(application_count, place_count, requested, magnifies) <= available {
        return Some(requested);
    }
    let minimum_icon_size = requested.icon_size.min(MINIMUM_ICON_SIZE);
    let minimum = compact_magnification(requested, minimum_icon_size);
    if reserved_extent(application_count, place_count, minimum, magnifies) > available {
        return None;
    }

    let mut lower = minimum_icon_size;
    let mut upper = requested.icon_size;
    let mut best = minimum;
    for _ in 0..24 {
        let candidate_size = (lower + upper) / 2.0;
        let candidate = compact_magnification(requested, candidate_size);
        if reserved_extent(application_count, place_count, candidate, magnifies) <= available {
            best = candidate;
            lower = candidate_size;
        } else {
            upper = candidate_size;
        }
    }
    Some(best)
}

fn compact_magnification(
    requested: motion::MagnificationConfig,
    icon_size: f32,
) -> motion::MagnificationConfig {
    let ratio = icon_size / requested.icon_size;
    let minimum_gap = requested.gap.min(MINIMUM_ICON_GAP);
    motion::MagnificationConfig {
        icon_size,
        gap: (requested.gap * ratio).clamp(minimum_gap, requested.gap),
        influence_radius: requested.influence_radius * ratio,
        maximum_scale: requested.maximum_scale,
    }
}

fn reserved_extent(
    application_count: usize,
    place_count: usize,
    magnification: motion::MagnificationConfig,
    magnifies: bool,
) -> f32 {
    let item_count = application_count + place_count;
    if item_count == 0 {
        return 0.0;
    }
    let mut extent = item_count as f32 * magnification.icon_size
        + item_count.saturating_sub(1) as f32 * magnification.gap;
    if application_count > 0 && place_count > 0 {
        extent += GROUP_GAP.max(magnification.gap) - magnification.gap;
    }
    if magnifies {
        let stride = magnification.icon_size + magnification.gap;
        let affected = (((2.0 * magnification.influence_radius) / stride).floor() as usize + 1)
            .min(item_count);
        extent += affected as f32 * magnification.icon_size * (magnification.maximum_scale - 1.0);
    }
    extent
}

fn layout_gaps(
    item_count: usize,
    application_count: usize,
    place_count: usize,
    regular_gap: f32,
) -> Vec<f32> {
    let mut gaps = vec![regular_gap; item_count.saturating_sub(1)];
    if application_count > 0 && place_count > 0 {
        gaps[application_count - 1] = GROUP_GAP.max(regular_gap);
    }
    gaps
}

pub(crate) fn overflow_group(hidden_applications: Vec<Entry>) -> OverflowGroup {
    let active = hidden_applications
        .iter()
        .any(|entry| entry.activity == ActivityIndicator::Active);
    let running = hidden_applications
        .iter()
        .filter(|entry| entry.activity != ActivityIndicator::None)
        .count();
    let urgent = hidden_applications.iter().any(|entry| entry.urgent);
    let mut accessible = vec![
        "More applications".to_owned(),
        count_label(hidden_applications.len(), "hidden application"),
    ];
    if running > 0 {
        accessible.push(count_label(running, "running application"));
    }
    if active {
        accessible.push("contains active application".into());
    }
    if urgent {
        accessible.push("needs attention".into());
    }
    OverflowGroup {
        entry: Entry {
            id: EntryId::Overflow,
            label: "More".into(),
            accessible_label: accessible.join(", "),
            icon: Icon::Builtin(BuiltinIcon::More),
            enabled: true,
            activity: if active {
                ActivityIndicator::Active
            } else if running > 0 {
                ActivityIndicator::Running
            } else {
                ActivityIndicator::None
            },
            urgent,
            badge: Some(hidden_applications.len()),
        },
        hidden_applications,
    }
}

fn ensure_fits(required: f32, available: f32) -> Result<(), LayoutError> {
    if required <= available {
        Ok(())
    } else {
        Err(LayoutError::DoesNotFit {
            required,
            available,
        })
    }
}

fn application_entry(item: &Item) -> Entry {
    let activity = if item.active {
        ActivityIndicator::Active
    } else if item.running {
        ActivityIndicator::Running
    } else {
        ActivityIndicator::None
    };
    let enabled = item.running || item.launchable;
    Entry {
        id: EntryId::Application(item.id.clone()),
        label: item.name.clone(),
        accessible_label: application_accessible_label(item, enabled),
        icon: item
            .icon
            .clone()
            .map(Icon::File)
            .unwrap_or(Icon::Builtin(BuiltinIcon::Application)),
        enabled,
        activity,
        urgent: item.urgent,
        badge: None,
    }
}

fn application_accessible_label(item: &Item, enabled: bool) -> String {
    let mut parts = vec![item.name.clone()];
    if item.active {
        parts.push("active".into());
    } else if item.running {
        parts.push("running".into());
    }
    if item.urgent {
        parts.push("needs attention".into());
    }
    if !item.windows.is_empty() {
        parts.push(count_label(item.windows.len(), "window"));
    }
    if !enabled {
        parts.push("unavailable".into());
    }
    parts.join(", ")
}

fn special_entry(item: &SpecialItem) -> Entry {
    let (icon, badge, state) = match item.kind {
        SpecialItemKind::Files => (BuiltinIcon::Files, None, None),
        SpecialItemKind::Downloads => (BuiltinIcon::Downloads, None, None),
        SpecialItemKind::Trash => match item.item_count {
            Some(0) => (BuiltinIcon::TrashEmpty, None, Some("empty".into())),
            Some(count) => (
                BuiltinIcon::TrashFull,
                Some(count),
                Some(count_label(count, "item")),
            ),
            None => (BuiltinIcon::TrashEmpty, None, None),
        },
    };
    let mut accessible = vec![item.name.to_owned()];
    if let Some(state) = state {
        accessible.push(state);
    }
    if !item.available {
        accessible.push("unavailable".into());
    }
    Entry {
        id: EntryId::Special(item.kind),
        label: item.name.to_owned(),
        accessible_label: accessible.join(", "),
        icon: Icon::Builtin(icon),
        enabled: item.available,
        activity: ActivityIndicator::None,
        urgent: false,
        badge,
    }
}

fn count_label(count: usize, singular: &str) -> String {
    let suffix = if count == 1 { "" } else { "s" };
    format!("{count} {singular}{suffix}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{SpecialActivation, WindowItem};

    fn item(name: &str) -> Item {
        Item {
            id: format!("{}.desktop", name.to_lowercase()),
            name: name.into(),
            icon: None,
            pinned: true,
            running: false,
            active: false,
            urgent: false,
            launchable: true,
            windows: Vec::new(),
            launch: None,
        }
    }

    fn special(
        kind: SpecialItemKind,
        name: &'static str,
        available: bool,
        item_count: Option<usize>,
    ) -> SpecialItem {
        SpecialItem {
            kind,
            name,
            available,
            item_count,
            activation: SpecialActivation::Unavailable {
                kind,
                detail: "test".into(),
            },
        }
    }

    #[test]
    fn applications_have_exact_visual_and_accessible_state() {
        let mut terminal = item("Terminal");
        terminal.icon = Some(Path::new("/home/alex/.icons/private.svg").into());
        terminal.running = true;
        terminal.active = true;
        terminal.urgent = true;
        terminal.windows = vec![
            WindowItem {
                id: rmac_compositor::WindowId(1),
                title: None,
                focused: true,
                urgent: false,
                focus_timestamp: None,
            },
            WindowItem {
                id: rmac_compositor::WindowId(2),
                title: None,
                focused: false,
                urgent: true,
                focus_timestamp: None,
            },
        ];
        let mut unavailable = item("Missing");
        unavailable.launchable = false;
        let content = ShelfContent::project(&Model {
            items: vec![terminal, unavailable],
            ..Default::default()
        });

        assert_eq!(content.applications[0].activity, ActivityIndicator::Active);
        assert!(content.applications[0].enabled);
        assert!(content.applications[0].urgent);
        assert_eq!(
            content.applications[0].accessible_label,
            "Terminal, active, needs attention, 2 windows"
        );
        assert!(!format!("{:?}", content.applications[0].icon).contains("alex"));
        assert_eq!(
            content.applications[1].icon,
            Icon::Builtin(BuiltinIcon::Application)
        );
        assert!(!content.applications[1].enabled);
        assert_eq!(
            content.applications[1].accessible_label,
            "Missing, unavailable"
        );
    }

    #[test]
    fn places_stay_separate_and_trash_uses_authoritative_count() {
        let content = ShelfContent::project(&Model {
            items: vec![item("Finder")],
            special_items: vec![
                special(SpecialItemKind::Files, "Files", true, None),
                special(SpecialItemKind::Downloads, "Downloads", false, None),
                special(SpecialItemKind::Trash, "Trash", true, Some(7)),
            ],
            ..Default::default()
        });

        assert!(content.has_separator());
        assert_eq!(content.places[0].icon, Icon::Builtin(BuiltinIcon::Files));
        assert!(!content.places[1].enabled);
        assert_eq!(content.places[1].accessible_label, "Downloads, unavailable");
        assert_eq!(content.places[2].badge, Some(7));
        assert_eq!(
            content.places[2].icon,
            Icon::Builtin(BuiltinIcon::TrashFull)
        );
        assert_eq!(content.places[2].accessible_label, "Trash, 7 items");

        let empty = special(SpecialItemKind::Trash, "Trash", true, Some(0));
        let empty = special_entry(&empty);
        assert_eq!(empty.badge, None);
        assert_eq!(empty.icon, Icon::Builtin(BuiltinIcon::TrashEmpty));
        assert_eq!(empty.accessible_label, "Trash, empty");
    }

    #[test]
    fn embedded_icons_are_bounded_self_contained_vectors() {
        let icons = [
            BuiltinIcon::Application,
            BuiltinIcon::Files,
            BuiltinIcon::Downloads,
            BuiltinIcon::TrashEmpty,
            BuiltinIcon::TrashFull,
            BuiltinIcon::More,
        ];
        for icon in icons {
            let svg = icon.svg();
            assert!(svg.starts_with("<svg"));
            assert!(svg.contains("viewBox=\"0 0 64 64\""));
            assert!(svg.len() < 16 * 1024);
            assert!(!svg.contains("<script"));
            assert!(!svg.contains("<image"));
            assert!(!svg.contains("href="));
            assert!(!svg.contains("<text"));
            assert!(!svg.contains("Gradient"));
            assert!(!svg.contains("<filter"));
        }
    }

    fn surface(axis: f64, reduced_motion: bool) -> SurfaceDescription {
        SurfaceDescription {
            output: rmac_compositor::OutputId::from("eDP-1"),
            placement: rmac_shell_settings::DockPlacement::Bottom,
            output_axis_length: axis,
            output_scale: 2.0,
            base_thickness: 64.0,
            maximum_thickness: if reduced_motion { 64.0 } else { 88.0 },
            exclusive_zone: 64.0,
            reveal_edge_thickness: 0.0,
            keyboard_interactive: false,
            autohide: false,
            overview_visible: false,
            magnification_enabled: !reduced_motion,
            animate: !reduced_motion,
            magnification: motion::MagnificationConfig::default(),
        }
    }

    fn grouped_content() -> ShelfContent {
        ShelfContent::project(&Model {
            items: vec![item("Finder"), item("Terminal")],
            special_items: vec![
                special(SpecialItemKind::Downloads, "Downloads", true, None),
                special(SpecialItemKind::Trash, "Trash", true, Some(3)),
            ],
            ..Default::default()
        })
    }

    #[test]
    fn layout_centers_groups_and_preserves_a_real_separator_gap() {
        let content = grouped_content();
        let layout = content.layout(&surface(800.0, false), None).unwrap();

        assert_eq!(layout.start, 284.0);
        assert_eq!(layout.end, 516.0);
        assert_eq!(layout.separator_axis, Some(400.0));
        assert_eq!(layout.slots[0].center, 308.0);
        assert_eq!(layout.slots[1].center, 364.0);
        assert_eq!(layout.slots[2].center, 436.0);
        assert_eq!(layout.slots[3].center, 492.0);
        assert_eq!(
            layout.hit_test(364.0),
            Some(&EntryId::Application("terminal.desktop".into()))
        );
        assert_eq!(layout.hit_test(400.0), None);
    }

    #[test]
    fn pointer_magnification_uses_stable_output_coordinates() {
        let content = grouped_content();
        let surface = surface(800.0, false);
        let first = content.layout(&surface, Some(364.0)).unwrap();
        let replay = content.layout(&surface, Some(364.0)).unwrap();

        assert_eq!(first, replay);
        assert_eq!(first.slots[1].center, 364.0);
        assert_eq!(first.slots[1].scale, 1.5);
        assert!(first.start >= SHELF_AXIS_PADDING);
        assert!(first.end <= 800.0 - SHELF_AXIS_PADDING);
    }

    #[test]
    fn reduced_motion_layout_never_scales_and_invalid_geometry_fails_closed() {
        let content = grouped_content();
        let layout = content.layout(&surface(800.0, true), Some(364.0)).unwrap();
        assert!(layout.slots.iter().all(|slot| slot.scale == 1.0));
        assert!(layout.slots.iter().all(|slot| slot.size == 48.0));

        assert!(matches!(
            content.layout(&surface(100.0, false), None),
            Err(LayoutError::DoesNotFit {
                required,
                available: 84.0,
            }) if required > 84.0
        ));
        assert_eq!(
            content.layout(&surface(800.0, false), Some(f32::NAN)),
            Err(LayoutError::InvalidPointer)
        );
        assert_eq!(
            content.layout(&surface(f64::INFINITY, false), None),
            Err(LayoutError::InvalidAxis)
        );
    }

    #[test]
    fn crowded_layout_selects_one_stable_size_before_hover() {
        let applications = (0..20)
            .map(|index| item(&format!("App {index:02}")))
            .collect();
        let content = ShelfContent::project(&Model {
            items: applications,
            special_items: vec![
                special(SpecialItemKind::Files, "Files", true, None),
                special(SpecialItemKind::Downloads, "Downloads", true, None),
                special(SpecialItemKind::Trash, "Trash", true, Some(2)),
            ],
            ..Default::default()
        });
        let surface = surface(1366.0, false);
        let plan = content.prepare_layout(&surface).unwrap();
        let resting = plan.layout(None).unwrap();

        assert_eq!(resting.slots.len(), 23);
        assert!(plan.overflow.is_none());
        assert!(plan.effective_magnification.icon_size < 48.0);
        assert!(plan.effective_magnification.icon_size >= MINIMUM_ICON_SIZE);
        let pointer = resting.slots[10].center;
        let hovered = plan.layout(Some(pointer)).unwrap();
        let replay = plan.layout(Some(pointer)).unwrap();
        assert_eq!(hovered, replay);
        assert_eq!(
            hovered.effective_magnification,
            plan.effective_magnification
        );
        assert!(hovered.end <= 1366.0 - SHELF_AXIS_PADDING);
        assert!(hovered.start >= SHELF_AXIS_PADDING);
    }

    #[test]
    fn extreme_crowding_keeps_a_stable_prefix_and_aggregates_hidden_state() {
        let mut applications = (0..100)
            .map(|index| item(&format!("App {index:03}")))
            .collect::<Vec<_>>();
        applications[99].running = true;
        applications[99].active = true;
        applications[99].urgent = true;
        let content = ShelfContent::project(&Model {
            items: applications,
            special_items: vec![
                special(SpecialItemKind::Files, "Files", true, None),
                special(SpecialItemKind::Downloads, "Downloads", true, None),
                special(SpecialItemKind::Trash, "Trash", true, Some(1)),
            ],
            ..Default::default()
        });
        let plan = content.prepare_layout(&surface(800.0, false)).unwrap();
        let layout = plan.layout(None).unwrap();
        let overflow = plan.overflow.as_ref().expect("overflow is discoverable");
        let visible_applications = layout.slots.len() - content.places.len() - 1;

        assert!(visible_applications > 0);
        for (index, slot) in layout.slots[..visible_applications].iter().enumerate() {
            assert_eq!(
                slot.id,
                EntryId::Application(format!("app {index:03}.desktop"))
            );
        }
        assert_eq!(layout.slots[visible_applications].id, EntryId::Overflow);
        assert_eq!(
            overflow.hidden_applications.len(),
            content.applications.len() - visible_applications
        );
        assert_eq!(
            overflow.hidden_applications[0].id,
            EntryId::Application(format!("app {visible_applications:03}.desktop"))
        );
        assert_eq!(overflow.entry.icon, Icon::Builtin(BuiltinIcon::More));
        assert_eq!(
            overflow.entry.badge,
            Some(overflow.hidden_applications.len())
        );
        assert_eq!(overflow.entry.activity, ActivityIndicator::Active);
        assert!(overflow.entry.urgent);
        assert!(overflow
            .entry
            .accessible_label
            .contains("contains active application"));
        assert!(overflow.entry.accessible_label.contains("needs attention"));
        assert_eq!(
            layout.hit_test(layout.slots[visible_applications].center),
            Some(&EntryId::Overflow)
        );
        assert_eq!(
            layout.slots[visible_applications + 1].id,
            EntryId::Special(SpecialItemKind::Files)
        );
    }
}
