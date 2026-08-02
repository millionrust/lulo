use std::fmt;

use crate::{Backend, Error, Operation, TrashEntryId};

#[derive(Clone, Eq, PartialEq)]
pub struct EmptyTrashReview {
    entries: Vec<TrashEntryId>,
}

impl EmptyTrashReview {
    pub fn item_count(&self) -> usize {
        self.entries.len()
    }
}

impl fmt::Debug for EmptyTrashReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmptyTrashReview")
            .field("item_count", &self.item_count())
            .field("entries", &"<private>")
            .finish()
    }
}

/// Prepare the exact set of Trash identities represented by the accepted
/// snapshot. A changed count refuses review and asks the caller to refresh.
pub fn prepare_empty_trash(
    snapshot: &rmac_places::TrashSnapshot,
    backend: &impl Backend,
) -> Result<Option<EmptyTrashReview>, Error> {
    if !snapshot.available {
        return Err(Error::message(
            Operation::InspectTrash,
            None,
            "Trash is unavailable",
        ));
    }
    if snapshot.empty || snapshot.item_count == 0 {
        return Ok(None);
    }
    let mut entries = backend
        .trash_entries()
        .map_err(|detail| Error::message(Operation::InspectTrash, None, detail))?;
    entries.sort_unstable();
    let before_deduplication = entries.len();
    entries.dedup();
    if entries.len() != before_deduplication || entries.len() != snapshot.item_count {
        return Err(Error::message(
            Operation::InspectTrash,
            None,
            "Trash changed before the deletion review",
        ));
    }
    Ok(Some(EmptyTrashReview { entries }))
}

#[derive(Eq, PartialEq)]
pub struct EmptyTrashConfirmation(EmptyTrashReview);

impl fmt::Debug for EmptyTrashConfirmation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmptyTrashConfirmation")
            .field("item_count", &self.0.item_count())
            .field("entries", &"<private>")
            .finish()
    }
}

pub fn confirm_empty_trash(
    review: EmptyTrashReview,
    confirmed: bool,
) -> Option<EmptyTrashConfirmation> {
    confirmed.then_some(EmptyTrashConfirmation(review))
}

pub fn empty_trash(
    confirmation: EmptyTrashConfirmation,
    backend: &impl Backend,
) -> Result<rmac_places::TrashSnapshot, Error> {
    backend
        .purge_trash(&confirmation.0.entries)
        .map_err(|detail| Error::message(Operation::EmptyTrash, None, detail))?;
    let item_count = backend
        .trash_count()
        .map_err(|detail| Error::message(Operation::InspectTrash, None, detail))?;
    Ok(rmac_places::TrashSnapshot {
        available: true,
        empty: item_count == 0,
        item_count,
    })
}
