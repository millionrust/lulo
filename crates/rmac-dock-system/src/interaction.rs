//! Target-scoped Dock busy state and privacy-safe failure presentation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{Error, FailureKind, Operation};

pub const MAX_PENDING_ACTIONS: usize = 64;
pub const MAX_RETAINED_FEEDBACK: usize = 16;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ActionTarget {
    Application(String),
    Window {
        application: String,
        window: rmac_compositor::WindowId,
    },
    Special(rmac_dock::SpecialItemKind),
    Stack(rmac_shell_settings::DockStackKind),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TicketId(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ticket {
    id: TicketId,
    target: ActionTarget,
    operation: Operation,
}

impl Ticket {
    pub fn id(&self) -> TicketId {
        self.id
    }

    pub fn target(&self) -> &ActionTarget {
        &self.target
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedbackReason {
    PermissionDenied,
    ServiceUnavailable,
    ConnectionLost,
    Rejected,
    Unsupported,
    Failed,
}

impl FeedbackReason {
    fn from_failure(kind: FailureKind) -> Self {
        match kind {
            FailureKind::Io(std::io::ErrorKind::PermissionDenied) => Self::PermissionDenied,
            FailureKind::Unavailable => Self::ServiceUnavailable,
            FailureKind::Transport => Self::ConnectionLost,
            FailureKind::Rejected => Self::Rejected,
            FailureKind::Unsupported => Self::Unsupported,
            FailureKind::Io(_) | FailureKind::Protocol | FailureKind::Other => Self::Failed,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::PermissionDenied => "permission was denied",
            Self::ServiceUnavailable => "the required service is unavailable",
            Self::ConnectionLost => "the desktop connection was lost",
            Self::Rejected => "the request was rejected",
            Self::Unsupported => "this action is not supported",
            Self::Failed => "the action failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Feedback {
    pub id: TicketId,
    pub target: ActionTarget,
    pub operation: Operation,
    pub reason: FeedbackReason,
}

impl Feedback {
    /// Path-, command-, window-title-, and backend-detail-free message for a
    /// toast or accessible status node. The renderer may localize the semantic
    /// fields instead of using this English fallback.
    pub fn accessible_message(&self) -> String {
        format!(
            "Could not {}: {}.",
            self.operation,
            self.reason.description()
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub busy: BTreeSet<ActionTarget>,
    pub feedback: Vec<Feedback>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transition {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BeginError {
    Busy { target: ActionTarget },
    TooManyPending,
    TicketExhausted,
}

impl fmt::Display for BeginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { .. } => formatter.write_str("the Dock action is already in progress"),
            Self::TooManyPending => {
                formatter.write_str("too many Dock actions are already in progress")
            }
            Self::TicketExhausted => {
                formatter.write_str("the Dock action ticket space is exhausted")
            }
        }
    }
}

impl std::error::Error for BeginError {}

#[derive(Default)]
pub struct State {
    next_ticket: u64,
    pending: BTreeMap<ActionTarget, (TicketId, Operation)>,
    feedback: BTreeMap<ActionTarget, Feedback>,
}

impl State {
    pub fn snapshot(&self) -> Snapshot {
        let mut feedback = self.feedback.values().cloned().collect::<Vec<_>>();
        feedback.sort_by_key(|feedback| feedback.id);
        Snapshot {
            busy: self.pending.keys().cloned().collect(),
            feedback,
        }
    }

    pub fn begin(
        &mut self,
        target: ActionTarget,
        operation: Operation,
    ) -> Result<(Ticket, Transition), BeginError> {
        if self.pending.contains_key(&target) {
            return Err(BeginError::Busy { target });
        }
        if self.pending.len() >= MAX_PENDING_ACTIONS {
            return Err(BeginError::TooManyPending);
        }
        let next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or(BeginError::TicketExhausted)?;
        self.next_ticket = next_ticket;
        let id = TicketId(next_ticket);
        self.feedback.remove(&target);
        self.pending.insert(target.clone(), (id, operation));
        let ticket = Ticket {
            id,
            target,
            operation,
        };
        Ok((
            ticket,
            Transition {
                snapshot: self.snapshot(),
                visible: true,
            },
        ))
    }

    pub fn finish(&mut self, ticket: Ticket, result: Result<(), &Error>) -> Transition {
        if self.pending.get(&ticket.target) != Some(&(ticket.id, ticket.operation)) {
            return Transition {
                snapshot: self.snapshot(),
                visible: false,
            };
        }
        self.pending.remove(&ticket.target);
        if let Err(error) = result {
            if !self.feedback.contains_key(&ticket.target)
                && self.feedback.len() >= MAX_RETAINED_FEEDBACK
            {
                let oldest = self
                    .feedback
                    .iter()
                    .min_by_key(|(_, feedback)| feedback.id)
                    .map(|(target, _)| target.clone())
                    .expect("a full feedback map has an oldest entry");
                self.feedback.remove(&oldest);
            }
            self.feedback.insert(
                ticket.target.clone(),
                Feedback {
                    id: ticket.id,
                    target: ticket.target,
                    operation: ticket.operation,
                    reason: FeedbackReason::from_failure(error.kind),
                },
            );
        }
        Transition {
            snapshot: self.snapshot(),
            visible: true,
        }
    }

    /// Cancel presentation-owned work (for example when a surface disappears)
    /// without claiming that the platform operation failed.
    pub fn cancel(&mut self, ticket: Ticket) -> Transition {
        if self.pending.get(&ticket.target) == Some(&(ticket.id, ticket.operation)) {
            self.pending.remove(&ticket.target);
            Transition {
                snapshot: self.snapshot(),
                visible: true,
            }
        } else {
            Transition {
                snapshot: self.snapshot(),
                visible: false,
            }
        }
    }

    pub fn dismiss(&mut self, target: &ActionTarget, id: TicketId) -> Transition {
        let removed = self
            .feedback
            .get(target)
            .is_some_and(|feedback| feedback.id == id)
            .then(|| self.feedback.remove(target))
            .flatten()
            .is_some();
        Transition {
            snapshot: self.snapshot(),
            visible: removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> ActionTarget {
        ActionTarget::Application(format!("{name}.desktop"))
    }

    fn error(kind: FailureKind, private_detail: &str) -> Error {
        Error::new(Operation::Launch, kind, "terminal.desktop", private_detail)
    }

    #[test]
    fn duplicate_target_is_rejected_while_other_targets_can_progress() {
        let mut state = State::default();
        let (terminal, started) = state.begin(app("terminal"), Operation::Launch).unwrap();
        assert!(started.visible);
        assert_eq!(started.snapshot.busy, BTreeSet::from([app("terminal")]));
        assert_eq!(
            state.begin(app("terminal"), Operation::Launch),
            Err(BeginError::Busy {
                target: app("terminal")
            })
        );

        let (finder, parallel) = state.begin(app("finder"), Operation::Launch).unwrap();
        assert_eq!(parallel.snapshot.busy.len(), 2);
        let finished = state.finish(terminal, Ok(()));
        assert_eq!(finished.snapshot.busy, BTreeSet::from([app("finder")]));
        assert!(finished.snapshot.feedback.is_empty());
        assert!(state.finish(finder, Ok(())).snapshot.busy.is_empty());
    }

    #[test]
    fn failure_feedback_is_semantic_dismissible_and_private_safe() {
        let mut state = State::default();
        let target = app("terminal");
        let (ticket, _) = state.begin(target.clone(), Operation::Launch).unwrap();
        let private = "/home/alex/private/project failed";
        let failure = error(
            FailureKind::Io(std::io::ErrorKind::PermissionDenied),
            private,
        );
        let failed = state.finish(ticket, Err(&failure));

        assert!(failed.snapshot.busy.is_empty());
        assert_eq!(failed.snapshot.feedback.len(), 1);
        let feedback = &failed.snapshot.feedback[0];
        assert_eq!(feedback.reason, FeedbackReason::PermissionDenied);
        assert!(!feedback.accessible_message().contains("alex"));
        assert!(!format!("{:?}", failed.snapshot).contains("alex"));

        let stale = state.dismiss(&target, TicketId(feedback.id.0 + 1));
        assert!(!stale.visible);
        let dismissed = state.dismiss(&target, feedback.id);
        assert!(dismissed.visible);
        assert!(dismissed.snapshot.feedback.is_empty());
    }

    #[test]
    fn retry_clears_old_feedback_and_stale_completion_cannot_clear_new_work() {
        let mut state = State::default();
        let target = app("terminal");
        let (first, _) = state.begin(target.clone(), Operation::Launch).unwrap();
        let failure = error(FailureKind::Transport, "private socket path");
        state.finish(first, Err(&failure));
        assert_eq!(state.snapshot().feedback.len(), 1);

        let (cancelled, retried) = state.begin(target.clone(), Operation::Launch).unwrap();
        assert!(retried.snapshot.feedback.is_empty());
        let stale_copy = cancelled.clone();
        assert!(state.cancel(cancelled).visible);
        let (current, _) = state.begin(target.clone(), Operation::Launch).unwrap();

        let stale = state.finish(stale_copy, Ok(()));
        assert!(!stale.visible);
        assert_eq!(stale.snapshot.busy, BTreeSet::from([target]));
        assert!(state.finish(current, Ok(())).snapshot.busy.is_empty());
    }

    #[test]
    fn every_platform_failure_maps_without_retaining_raw_detail() {
        let cases = [
            (FailureKind::Unavailable, FeedbackReason::ServiceUnavailable),
            (FailureKind::Transport, FeedbackReason::ConnectionLost),
            (FailureKind::Rejected, FeedbackReason::Rejected),
            (FailureKind::Unsupported, FeedbackReason::Unsupported),
            (FailureKind::Protocol, FeedbackReason::Failed),
            (FailureKind::Other, FeedbackReason::Failed),
        ];
        for (index, (kind, expected)) in cases.into_iter().enumerate() {
            let mut state = State::default();
            let (ticket, _) = state
                .begin(app(&format!("app-{index}")), Operation::Launch)
                .unwrap();
            let failure = error(kind, "secret backend diagnostic");
            let transition = state.finish(ticket, Err(&failure));
            assert_eq!(transition.snapshot.feedback[0].reason, expected);
            assert!(!format!("{transition:?}").contains("secret"));
        }
    }

    #[test]
    fn ticket_exhaustion_fails_without_mutating_state() {
        let mut state = State {
            next_ticket: u64::MAX,
            ..Default::default()
        };
        assert_eq!(
            state.begin(app("terminal"), Operation::Launch),
            Err(BeginError::TicketExhausted)
        );
        assert_eq!(state.snapshot(), Snapshot::default());
    }

    #[test]
    fn pending_and_feedback_state_remain_bounded_and_feedback_is_chronological() {
        let mut pending = State::default();
        let mut tickets = Vec::new();
        for index in 0..MAX_PENDING_ACTIONS {
            tickets.push(
                pending
                    .begin(app(&format!("pending-{index:02}")), Operation::Launch)
                    .unwrap()
                    .0,
            );
        }
        assert_eq!(pending.snapshot().busy.len(), MAX_PENDING_ACTIONS);
        assert_eq!(
            pending.begin(app("one-too-many"), Operation::Launch),
            Err(BeginError::TooManyPending)
        );
        for ticket in tickets {
            pending.cancel(ticket);
        }

        let mut feedback = State::default();
        for index in 0..MAX_RETAINED_FEEDBACK + 4 {
            let (ticket, _) = feedback
                .begin(app(&format!("failed-{index:02}")), Operation::Launch)
                .unwrap();
            let failure = error(FailureKind::Rejected, "private rejection detail");
            feedback.finish(ticket, Err(&failure));
        }
        let snapshot = feedback.snapshot();
        assert_eq!(snapshot.feedback.len(), MAX_RETAINED_FEEDBACK);
        assert!(snapshot
            .feedback
            .windows(2)
            .all(|pair| pair[0].id < pair[1].id));
        assert_eq!(snapshot.feedback[0].id, TicketId(5));
        assert_eq!(snapshot.feedback.last().unwrap().id, TicketId(20));
    }
}
