use std::collections::BTreeMap;
use std::fmt;

use crate::inventory::{InventoryError, SeatId, SeatInventory, MAX_SEATS};

pub const REQUIRED_WL_SEAT_VERSION: u32 = 2;

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
