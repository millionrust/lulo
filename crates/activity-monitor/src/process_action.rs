//! Pure process-action identity and feedback policy.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionKind {
    Quit,
    ForceQuit,
}

impl ActionKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::ForceQuit => "Force Quit",
        }
    }

    fn past_tense(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::ForceQuit => "force-quit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) start_time: u64,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) process: ProcessIdentity,
    pub(crate) kind: ActionKind,
}

/// State for the process confirmation shown by the monitor.
#[derive(Default)]
pub(crate) struct Confirmation(Option<Request>);

impl Confirmation {
    pub(crate) fn request(&mut self, request: Request) {
        self.0 = Some(request);
    }

    pub(crate) fn take(&mut self) -> Option<Request> {
        self.0.take()
    }

    pub(crate) fn current(&self) -> Option<&Request> {
        self.0.as_ref()
    }

    pub(crate) fn current_mut(&mut self) -> Option<&mut Request> {
        self.0.as_mut()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Preflight {
    Current,
    Missing,
    Replaced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Delivered,
    Missing,
    Replaced,
    Unsupported,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Feedback {
    pub(crate) success: bool,
    pub(crate) title: String,
    pub(crate) detail: String,
}

pub(crate) fn preflight(request: &Request, observed: Option<&ProcessIdentity>) -> Preflight {
    match observed {
        None => Preflight::Missing,
        Some(observed) if observed != &request.process => Preflight::Replaced,
        Some(_) => Preflight::Current,
    }
}

pub(crate) fn feedback(request: &Request, outcome: Outcome) -> Feedback {
    let label = request.kind.label();
    let process = &request.process;
    match outcome {
        Outcome::Delivered => Feedback {
            success: true,
            title: format!("{label} request sent"),
            detail: format!(
                "The signal was delivered to {} (PID {}). This confirms delivery, not that the process has exited.",
                process.name, process.pid
            ),
        },
        Outcome::Missing => Feedback {
            success: false,
            title: format!("{label} was not sent"),
            detail: format!(
                "{} (PID {}) exited before confirmation.",
                process.name, process.pid
            ),
        },
        Outcome::Replaced => Feedback {
            success: false,
            title: format!("{label} was not sent"),
            detail: format!(
                "PID {} now identifies a different process. No signal was delivered.",
                process.pid
            ),
        },
        Outcome::Unsupported => Feedback {
            success: false,
            title: format!("{label} is unavailable"),
            detail: format!(
                "This system cannot send the requested signal to {} (PID {}).",
                process.name, process.pid
            ),
        },
        Outcome::Rejected => Feedback {
            success: false,
            title: format!("{label} request failed"),
            detail: format!(
                "The operating system rejected the request to {} {} (PID {}). The process may have exited, or your account may not have permission.",
                request.kind.past_tense(),
                process.name,
                process.pid
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: ActionKind) -> Request {
        Request {
            process: ProcessIdentity {
                pid: 42,
                start_time: 1_000,
                name: "worker".into(),
            },
            kind,
        }
    }

    #[test]
    fn quit_confirmation_can_be_reopened_after_cancel() {
        let mut confirmation = Confirmation::default();
        confirmation.request(request(ActionKind::Quit));
        assert_eq!(
            confirmation.current().map(|r| r.kind),
            Some(ActionKind::Quit)
        );
        assert_eq!(confirmation.take().map(|r| r.kind), Some(ActionKind::Quit));
        assert!(confirmation.current().is_none());
        confirmation.request(request(ActionKind::Quit));
        assert_eq!(
            confirmation.current().map(|r| r.kind),
            Some(ActionKind::Quit)
        );
    }

    #[test]
    fn force_quit_confirmation_can_be_reopened_after_cancel() {
        let mut confirmation = Confirmation::default();
        confirmation.request(request(ActionKind::ForceQuit));
        assert_eq!(
            confirmation.current().map(|r| r.kind),
            Some(ActionKind::ForceQuit)
        );
        assert_eq!(
            confirmation.take().map(|r| r.kind),
            Some(ActionKind::ForceQuit)
        );
        assert!(confirmation.current().is_none());
        confirmation.request(request(ActionKind::ForceQuit));
        assert_eq!(
            confirmation.current().map(|r| r.kind),
            Some(ActionKind::ForceQuit)
        );
    }

    #[test]
    fn preflight_requires_the_complete_original_identity() {
        let request = request(ActionKind::Quit);
        assert_eq!(
            preflight(&request, Some(&request.process)),
            Preflight::Current
        );
        assert_eq!(preflight(&request, None), Preflight::Missing);

        let mut reused = request.process.clone();
        reused.start_time += 1;
        assert_eq!(preflight(&request, Some(&reused)), Preflight::Replaced);

        let mut renamed = request.process.clone();
        renamed.name = "replacement".into();
        assert_eq!(preflight(&request, Some(&renamed)), Preflight::Replaced);
    }

    #[test]
    fn delivered_feedback_never_claims_process_exit() {
        let feedback = feedback(&request(ActionKind::ForceQuit), Outcome::Delivered);
        assert!(feedback.success);
        assert!(feedback.title.contains("request sent"));
        assert!(feedback.detail.contains("confirms delivery"));
        assert!(feedback.detail.contains("not that the process has exited"));
    }

    #[test]
    fn every_failure_is_actionable_and_never_claims_delivery() {
        for outcome in [
            Outcome::Missing,
            Outcome::Replaced,
            Outcome::Unsupported,
            Outcome::Rejected,
        ] {
            let feedback = feedback(&request(ActionKind::Quit), outcome);
            assert!(!feedback.success);
            assert!(!feedback.title.is_empty());
            assert!(!feedback.detail.is_empty());
            assert!(!feedback.title.contains("request sent"));
        }
        assert!(feedback(&request(ActionKind::Quit), Outcome::Rejected)
            .detail
            .contains("permission"));
    }
}
