//! Truthful output and Wayland-seat context for shell-surface invocations.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_SEATS: usize = 16;
pub const MAX_SEAT_ID_BYTES: usize = 128;
pub const REQUIRED_WL_SEAT_VERSION: u32 = 2;

#[cfg(target_os = "linux")]
pub mod wayland;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct SeatId(String);

impl SeatId {
    pub fn new(value: impl Into<String>) -> Result<Self, InventoryError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_SEAT_ID_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(InventoryError::InvalidSeat);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SeatId(<redacted>)")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SeatInventory {
    seats: Vec<SeatId>,
}

impl SeatInventory {
    /// Accept only a complete post-registry-roundtrip seat inventory.
    pub fn new(values: impl IntoIterator<Item = String>) -> Result<Self, InventoryError> {
        let values = values.into_iter().collect::<Vec<_>>();
        if values.len() > MAX_SEATS {
            return Err(InventoryError::TooManySeats {
                count: values.len(),
            });
        }
        let mut unique = BTreeSet::new();
        let mut seats = Vec::with_capacity(values.len());
        for value in values {
            let seat = SeatId::new(value)?;
            if !unique.insert(seat.clone()) {
                return Err(InventoryError::DuplicateSeat);
            }
            seats.push(seat);
        }
        seats.sort();
        Ok(Self { seats })
    }

    pub fn len(&self) -> usize {
        self.seats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }

    fn exact(&self, seat: &SeatId) -> bool {
        self.seats.binary_search(seat).is_ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InventoryError {
    InvalidSeat,
    DuplicateSeat,
    TooManySeats { count: usize },
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSeat => formatter.write_str("the Wayland seat identity is invalid"),
            Self::DuplicateSeat => {
                formatter.write_str("the Wayland seat inventory contains a duplicate")
            }
            Self::TooManySeats { count } => {
                write!(formatter, "the Wayland seat inventory has {count} entries")
            }
        }
    }
}

impl std::error::Error for InventoryError {}

/// Platform-neutral reducer for live `wl_registry` and `wl_seat.name` events.
/// A snapshot is published only when every currently bound seat has delivered
/// its immutable name.
#[derive(Debug, Default)]
pub struct SeatRegistry {
    seats: BTreeMap<u32, Option<SeatId>>,
    published: Option<SeatInventory>,
}

impl SeatRegistry {
    pub fn add(&mut self, global: u32, version: u32) -> Result<(), RegistryError> {
        if version < REQUIRED_WL_SEAT_VERSION {
            return Err(RegistryError::SeatVersion {
                advertised: version,
                required: REQUIRED_WL_SEAT_VERSION,
            });
        }
        if self.seats.contains_key(&global) {
            return Err(RegistryError::DuplicateGlobal);
        }
        if self.seats.len() >= MAX_SEATS {
            return Err(RegistryError::TooManySeats {
                count: self.seats.len() + 1,
            });
        }
        self.seats.insert(global, None);
        Ok(())
    }

    pub fn name(
        &mut self,
        global: u32,
        value: impl Into<String>,
    ) -> Result<Option<SeatInventory>, RegistryError> {
        let seat = SeatId::new(value).map_err(RegistryError::Inventory)?;
        let current = self
            .seats
            .get_mut(&global)
            .ok_or(RegistryError::UnknownSeat)?;
        match current {
            Some(existing) if existing != &seat => return Err(RegistryError::SeatRenamed),
            Some(_) => return Ok(None),
            None => *current = Some(seat),
        }
        self.publish_if_complete()
    }

    pub fn remove(&mut self, global: u32) -> Result<Option<SeatInventory>, RegistryError> {
        if self.seats.remove(&global).is_none() {
            return Ok(None);
        }
        self.publish_if_complete()
    }

    pub fn require_complete(&mut self) -> Result<Option<SeatInventory>, RegistryError> {
        if self.seats.values().any(Option::is_none) {
            return Err(RegistryError::IncompleteSeat);
        }
        self.publish_if_complete()
    }

    pub fn snapshot(&self) -> Option<&SeatInventory> {
        self.published.as_ref()
    }

    fn publish_if_complete(&mut self) -> Result<Option<SeatInventory>, RegistryError> {
        if self.seats.values().any(Option::is_none) {
            return Ok(None);
        }
        let inventory = SeatInventory::new(
            self.seats
                .values()
                .filter_map(Clone::clone)
                .map(|seat| seat.0),
        )
        .map_err(RegistryError::Inventory)?;
        if self.published.as_ref() == Some(&inventory) {
            return Ok(None);
        }
        self.published = Some(inventory.clone());
        Ok(Some(inventory))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    SeatVersion { advertised: u32, required: u32 },
    DuplicateGlobal,
    TooManySeats { count: usize },
    UnknownSeat,
    SeatRenamed,
    IncompleteSeat,
    Inventory(InventoryError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeatVersion {
                advertised,
                required,
            } => write!(
                formatter,
                "wl_seat version {advertised} is below required version {required}"
            ),
            Self::DuplicateGlobal => {
                formatter.write_str("the Wayland registry repeated a seat global")
            }
            Self::TooManySeats { count } => {
                write!(formatter, "the Wayland registry has {count} seats")
            }
            Self::UnknownSeat => {
                formatter.write_str("a Wayland seat event referenced an unknown global")
            }
            Self::SeatRenamed => formatter.write_str("a Wayland seat changed its immutable name"),
            Self::IncompleteSeat => formatter.write_str("a Wayland seat did not publish its name"),
            Self::Inventory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RegistryError {}

#[derive(Clone, Eq, PartialEq)]
pub struct Invocation {
    output: rmac_compositor::OutputId,
    seat: SeatId,
    restore_window: Option<rmac_compositor::WindowId>,
}

impl Invocation {
    pub fn output(&self) -> &rmac_compositor::OutputId {
        &self.output
    }

    pub fn seat(&self) -> &SeatId {
        &self.seat
    }

    pub fn restore_window(&self) -> Option<rmac_compositor::WindowId> {
        self.restore_window
    }
}

impl fmt::Debug for Invocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Invocation")
            .field("output", &"<redacted>")
            .field("seat", &"<redacted>")
            .field("restore_window", &self.restore_window.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveError {
    NoFocusedOutput,
    OutputUnavailable,
    NoSeat,
    AmbiguousSeat,
    SeatUnavailable,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoFocusedOutput => "the compositor has no focused output",
            Self::OutputUnavailable => "the invocation output is unavailable",
            Self::NoSeat => "no complete Wayland seat is available",
            Self::AmbiguousSeat => {
                "the shortcut activation does not identify one of multiple seats"
            }
            Self::SeatUnavailable => "the invoking Wayland seat is unavailable",
        })
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a global shortcut only when the compositor has an exact focused
/// output and the Wayland host has exactly one complete seat. Multi-seat
/// sessions fail explicitly because portal shortcut activation carries no seat.
pub fn global_shortcut(
    compositor: &rmac_compositor::Snapshot,
    seats: &SeatInventory,
) -> Result<Invocation, ResolveError> {
    let output = compositor
        .focus
        .output
        .as_ref()
        .ok_or(ResolveError::NoFocusedOutput)?;
    let seat = match seats.seats.as_slice() {
        [] => return Err(ResolveError::NoSeat),
        [seat] => seat.clone(),
        _ => return Err(ResolveError::AmbiguousSeat),
    };
    capture(output, seat, compositor)
}

/// Resolve a pointer invocation whose layer host already knows the exact
/// output and seat. Both identities are revalidated against complete state.
pub fn surface_control(
    output: &rmac_compositor::OutputId,
    seat: &SeatId,
    compositor: &rmac_compositor::Snapshot,
    seats: &SeatInventory,
) -> Result<Invocation, ResolveError> {
    if !seats.exact(seat) {
        return Err(ResolveError::SeatUnavailable);
    }
    capture(output, seat.clone(), compositor)
}

fn capture(
    output: &rmac_compositor::OutputId,
    seat: SeatId,
    compositor: &rmac_compositor::Snapshot,
) -> Result<Invocation, ResolveError> {
    if !compositor
        .outputs
        .iter()
        .any(|candidate| candidate.id == *output && candidate.enabled())
    {
        return Err(ResolveError::OutputUnavailable);
    }
    let restore_window = compositor.focus.window.filter(|focused| {
        compositor
            .windows
            .iter()
            .any(|window| window.id == *focused && window.focused)
    });
    Ok(Invocation {
        output: output.clone(),
        seat,
        restore_window,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, enabled: bool) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "private make".into(),
            model: "private model".into(),
            serial: Some("private serial".into()),
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: enabled.then_some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: enabled.then_some(rmac_compositor::LogicalOutput {
                position: rmac_compositor::LogicalPoint::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    fn window(id: u64, focused: bool) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: rmac_compositor::WindowId(id),
            title: None,
            app_id: None,
            pid: None,
            workspace: None,
            focused,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: rmac_compositor::WindowLayout::default(),
        }
    }

    fn compositor() -> rmac_compositor::Snapshot {
        rmac_compositor::Snapshot {
            outputs: vec![output("private-output-27", true)],
            windows: vec![window(8, true)],
            focus: rmac_compositor::FocusState {
                output: Some("private-output-27".into()),
                window: Some(rmac_compositor::WindowId(8)),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn single_seat_shortcut_resolves_exact_context_privately() {
        let seats = SeatInventory::new(vec!["private-seat-19".into()]).unwrap();
        let invocation = global_shortcut(&compositor(), &seats).unwrap();
        assert_eq!(invocation.output().0, "private-output-27");
        assert_eq!(invocation.seat().as_str(), "private-seat-19");
        assert_eq!(
            invocation.restore_window(),
            Some(rmac_compositor::WindowId(8))
        );
        let diagnostics = format!("{seats:?} {invocation:?}");
        assert!(!diagnostics.contains("private-output-27"));
        assert!(!diagnostics.contains("private-seat-19"));
    }

    #[test]
    fn shortcut_refuses_missing_or_ambiguous_seat() {
        assert_eq!(
            global_shortcut(&compositor(), &SeatInventory::default()),
            Err(ResolveError::NoSeat)
        );
        let seats = SeatInventory::new(vec!["seat-a".into(), "seat-b".into()]).unwrap();
        assert_eq!(
            global_shortcut(&compositor(), &seats),
            Err(ResolveError::AmbiguousSeat)
        );
    }

    #[test]
    fn surface_control_revalidates_exact_seat_and_output() {
        let seats = SeatInventory::new(vec!["seat-a".into(), "seat-b".into()]).unwrap();
        let seat = SeatId::new("seat-b").unwrap();
        let invocation =
            surface_control(&"private-output-27".into(), &seat, &compositor(), &seats).unwrap();
        assert_eq!(invocation.seat(), &seat);
        assert_eq!(
            surface_control(
                &"private-output-27".into(),
                &SeatId::new("seat-c").unwrap(),
                &compositor(),
                &seats,
            ),
            Err(ResolveError::SeatUnavailable)
        );
    }

    #[test]
    fn invalid_inventory_and_unavailable_focus_fail_closed() {
        assert_eq!(
            SeatInventory::new(vec!["seat-a".into(), "seat-a".into()]),
            Err(InventoryError::DuplicateSeat)
        );
        let mut snapshot = compositor();
        snapshot.outputs[0] = output("private-output-27", false);
        assert_eq!(
            global_shortcut(
                &snapshot,
                &SeatInventory::new(vec!["seat-a".into()]).unwrap()
            ),
            Err(ResolveError::OutputUnavailable)
        );
        snapshot.focus.output = None;
        assert_eq!(
            global_shortcut(
                &snapshot,
                &SeatInventory::new(vec!["seat-a".into()]).unwrap()
            ),
            Err(ResolveError::NoFocusedOutput)
        );
    }

    #[test]
    fn registry_publishes_only_complete_hotplug_snapshots() {
        let mut registry = SeatRegistry::default();
        assert_eq!(registry.require_complete().unwrap().unwrap().len(), 0);
        registry.add(8, REQUIRED_WL_SEAT_VERSION).unwrap();
        assert!(registry.snapshot().unwrap().is_empty());
        assert!(registry.name(8, "seat-a").unwrap().is_some());
        registry.add(9, REQUIRED_WL_SEAT_VERSION).unwrap();
        assert!(registry.name(8, "seat-a").unwrap().is_none());
        let two = registry.name(9, "seat-b").unwrap().unwrap();
        assert_eq!(two.len(), 2);
        let one = registry.remove(8).unwrap().unwrap();
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn registry_rejects_incomplete_duplicate_and_renamed_seats() {
        let mut registry = SeatRegistry::default();
        assert_eq!(
            registry.add(1, REQUIRED_WL_SEAT_VERSION - 1),
            Err(RegistryError::SeatVersion {
                advertised: REQUIRED_WL_SEAT_VERSION - 1,
                required: REQUIRED_WL_SEAT_VERSION,
            })
        );
        registry.add(1, REQUIRED_WL_SEAT_VERSION).unwrap();
        assert_eq!(
            registry.require_complete(),
            Err(RegistryError::IncompleteSeat)
        );
        registry.name(1, "seat-a").unwrap();
        assert_eq!(registry.name(1, "seat-b"), Err(RegistryError::SeatRenamed));
        registry.add(2, REQUIRED_WL_SEAT_VERSION).unwrap();
        assert_eq!(
            registry.name(2, "seat-a"),
            Err(RegistryError::Inventory(InventoryError::DuplicateSeat))
        );
    }
}
