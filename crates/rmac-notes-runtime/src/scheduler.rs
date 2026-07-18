use std::fmt;
use std::time::Duration;

use rmac_notes_store::{NoteChanges, NoteId};

pub const DEFAULT_EDIT_DEBOUNCE: Duration = Duration::from_millis(500);
pub const MAX_EDIT_DEBOUNCE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EditGeneration(u64);

impl EditGeneration {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScheduledEdit {
    request_id: u64,
    generation: EditGeneration,
    note_id: NoteId,
    expected_note_revision: u64,
    changes: NoteChanges,
}

impl ScheduledEdit {
    pub fn new(
        request_id: u64,
        generation: EditGeneration,
        note_id: NoteId,
        expected_note_revision: u64,
        changes: NoteChanges,
    ) -> Result<Self, SchedulerError> {
        if request_id == 0 || expected_note_revision == 0 {
            return Err(SchedulerError::InvalidRequest);
        }
        Ok(Self {
            request_id,
            generation,
            note_id,
            expected_note_revision,
            changes,
        })
    }

    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn generation(&self) -> EditGeneration {
        self.generation
    }

    pub fn note_id(&self) -> NoteId {
        self.note_id
    }

    pub fn expected_note_revision(&self) -> u64 {
        self.expected_note_revision
    }

    pub fn changes(&self) -> &NoteChanges {
        &self.changes
    }

    pub fn into_changes(self) -> NoteChanges {
        self.changes
    }
}

impl fmt::Debug for ScheduledEdit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScheduledEdit")
            .field("request_id", &self.request_id)
            .field("generation", &self.generation)
            .field("note_id", &self.note_id)
            .field("expected_note_revision", &self.expected_note_revision)
            .field("changes", &"[private]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleOutcome {
    /// A different note's pending edit that must be committed before the new
    /// note becomes the active pending edit.
    pub displaced: Option<ScheduledEdit>,
    /// The same note's older generation that was safely coalesced away.
    pub replaced_generation: Option<EditGeneration>,
    pub deadline_millis: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerError {
    InvalidDebounce,
    InvalidRequest,
    StaleGeneration,
    TimeOverflow,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDebounce => "the Notes edit debounce is outside its safety bounds",
            Self::InvalidRequest => "the Notes edit request is invalid",
            Self::StaleGeneration => "an older Notes edit cannot replace a newer edit",
            Self::TimeOverflow => "the Notes edit deadline cannot be represented",
        })
    }
}

impl std::error::Error for SchedulerError {}

struct PendingEdit {
    edit: ScheduledEdit,
    deadline_millis: u64,
}

/// One-active-note debounce state with no permanent timer.
///
/// [`next_wait`](Self::next_wait) returns `None` while idle. Scheduling a newer
/// generation for the same note replaces the older full edit; scheduling a
/// different note returns the old edit for immediate flush rather than losing
/// it. Generations are globally strict so delayed UI messages fail closed.
pub struct EditScheduler {
    debounce_millis: u64,
    highest_generation: Option<EditGeneration>,
    pending: Option<PendingEdit>,
}

impl EditScheduler {
    pub fn new(debounce: Duration) -> Result<Self, SchedulerError> {
        let millis = u64::try_from(debounce.as_millis())
            .ok()
            .filter(|millis| *millis > 0)
            .filter(|millis| *millis <= MAX_EDIT_DEBOUNCE.as_millis() as u64)
            .ok_or(SchedulerError::InvalidDebounce)?;
        Ok(Self {
            debounce_millis: millis,
            highest_generation: None,
            pending: None,
        })
    }

    pub fn schedule(
        &mut self,
        now_millis: u64,
        edit: ScheduledEdit,
    ) -> Result<ScheduleOutcome, SchedulerError> {
        if self
            .highest_generation
            .is_some_and(|highest| edit.generation() <= highest)
        {
            return Err(SchedulerError::StaleGeneration);
        }
        let deadline_millis = now_millis
            .checked_add(self.debounce_millis)
            .ok_or(SchedulerError::TimeOverflow)?;
        self.highest_generation = Some(edit.generation());
        let previous = self.pending.take();
        let (displaced, replaced_generation) = match previous {
            Some(previous) if previous.edit.note_id() == edit.note_id() => {
                (None, Some(previous.edit.generation()))
            }
            Some(previous) => (Some(previous.edit), None),
            None => (None, None),
        };
        self.pending = Some(PendingEdit {
            edit,
            deadline_millis,
        });
        Ok(ScheduleOutcome {
            displaced,
            replaced_generation,
            deadline_millis,
        })
    }

    pub fn take_due(&mut self, now_millis: u64) -> Option<ScheduledEdit> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now_millis >= pending.deadline_millis)
        {
            return self.pending.take().map(|pending| pending.edit);
        }
        None
    }

    pub fn flush(&mut self) -> Option<ScheduledEdit> {
        self.pending.take().map(|pending| pending.edit)
    }

    pub fn cancel(&mut self, generation: EditGeneration) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.edit.generation() == generation)
        {
            self.pending = None;
            true
        } else {
            false
        }
    }

    pub fn next_wait(&self, now_millis: u64) -> Option<Duration> {
        self.pending.as_ref().map(|pending| {
            Duration::from_millis(pending.deadline_millis.saturating_sub(now_millis))
        })
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
}

impl Default for EditScheduler {
    fn default() -> Self {
        Self::new(DEFAULT_EDIT_DEBOUNCE).expect("the default debounce is within fixed bounds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(request: u64, generation: u64, note: u64, body: &str) -> ScheduledEdit {
        ScheduledEdit::new(
            request,
            EditGeneration::new(generation).unwrap(),
            NoteId::new(note).unwrap(),
            1,
            NoteChanges {
                modified_unix_ms: generation,
                title: "Private title".into(),
                body: body.into(),
                tags: Vec::new(),
            },
        )
        .unwrap()
    }

    #[test]
    fn idle_scheduler_has_no_timer_or_synthetic_save() {
        let mut scheduler = EditScheduler::default();

        assert_eq!(scheduler.next_wait(1_000), None);
        assert_eq!(scheduler.take_due(u64::MAX), None);
        assert!(!scheduler.has_pending());
    }

    #[test]
    fn same_note_edits_coalesce_to_the_newest_complete_generation() {
        let mut scheduler = EditScheduler::default();
        scheduler.schedule(1_000, edit(1, 1, 7, "first")).unwrap();

        let outcome = scheduler.schedule(1_200, edit(2, 2, 7, "latest")).unwrap();

        assert_eq!(outcome.displaced, None);
        assert_eq!(outcome.replaced_generation, EditGeneration::new(1));
        assert_eq!(outcome.deadline_millis, 1_700);
        assert_eq!(scheduler.take_due(1_699), None);
        let due = scheduler.take_due(1_700).unwrap();
        assert_eq!(due.request_id(), 2);
        assert_eq!(due.changes().body, "latest");
        assert_eq!(scheduler.next_wait(1_700), None);
    }

    #[test]
    fn switching_notes_returns_the_previous_edit_for_ordered_flush() {
        let mut scheduler = EditScheduler::default();
        scheduler.schedule(0, edit(1, 1, 7, "first note")).unwrap();

        let outcome = scheduler
            .schedule(100, edit(2, 2, 8, "second note"))
            .unwrap();

        let displaced = outcome.displaced.unwrap();
        assert_eq!(displaced.note_id(), NoteId::new(7).unwrap());
        assert_eq!(displaced.changes().body, "first note");
        assert_eq!(
            scheduler.flush().unwrap().note_id(),
            NoteId::new(8).unwrap()
        );
    }

    #[test]
    fn stale_or_overflowing_schedule_does_not_replace_pending_content() {
        let mut scheduler = EditScheduler::default();
        scheduler.schedule(10, edit(2, 2, 7, "newer")).unwrap();

        assert_eq!(
            scheduler.schedule(20, edit(1, 1, 7, "stale")),
            Err(SchedulerError::StaleGeneration)
        );
        assert_eq!(
            scheduler.schedule(u64::MAX, edit(3, 3, 7, "overflow")),
            Err(SchedulerError::TimeOverflow)
        );
        assert_eq!(scheduler.flush().unwrap().changes().body, "newer");
    }

    #[test]
    fn only_the_exact_pending_generation_can_be_cancelled() {
        let mut scheduler = EditScheduler::default();
        scheduler.schedule(0, edit(1, 1, 7, "draft")).unwrap();

        assert!(!scheduler.cancel(EditGeneration::new(2).unwrap()));
        assert!(scheduler.has_pending());
        assert!(scheduler.cancel(EditGeneration::new(1).unwrap()));
        assert!(!scheduler.has_pending());
    }

    #[test]
    fn debounce_and_request_identifiers_are_bounded() {
        assert_eq!(
            EditScheduler::new(Duration::ZERO).err(),
            Some(SchedulerError::InvalidDebounce)
        );
        assert_eq!(
            EditScheduler::new(MAX_EDIT_DEBOUNCE + Duration::from_millis(1)).err(),
            Some(SchedulerError::InvalidDebounce)
        );
        assert_eq!(
            ScheduledEdit::new(
                0,
                EditGeneration::new(1).unwrap(),
                NoteId::new(1).unwrap(),
                1,
                NoteChanges {
                    modified_unix_ms: 1,
                    title: String::new(),
                    body: String::new(),
                    tags: Vec::new(),
                }
            )
            .err(),
            Some(SchedulerError::InvalidRequest)
        );
    }

    #[test]
    fn debug_output_redacts_private_editor_content() {
        let edit = edit(1, 1, 7, "secret body");
        let debug = format!("{edit:?}");

        assert!(!debug.contains("Private title"));
        assert!(!debug.contains("secret body"));
        assert!(debug.contains("[private]"));
    }
}
