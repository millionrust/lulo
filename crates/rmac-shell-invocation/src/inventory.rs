use std::collections::BTreeSet;
use std::fmt;

pub const MAX_SEATS: usize = 16;
pub const MAX_SEAT_ID_BYTES: usize = 128;

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct SeatId(pub(crate) String);

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
    pub(crate) seats: Vec<SeatId>,
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

    pub(crate) fn exact(&self, seat: &SeatId) -> bool {
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
