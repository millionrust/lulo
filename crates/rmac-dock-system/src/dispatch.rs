//! Revalidated Dock menu dispatch with target-scoped interaction tickets.

use std::fmt;

use crate::interaction::{ActionTarget, BeginError, State, Ticket, Transition};
use crate::{Backend, Error, FailureKind, Operation, Outcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareError {
    UnsupportedEntry,
}

impl fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEntry => {
                formatter.write_str("the Dock menu entry cannot be dispatched")
            }
        }
    }
}

impl std::error::Error for PrepareError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Preparation {
    NoAction,
    Ready(PreparedAction),
}

#[derive(Clone, Eq, PartialEq)]
pub struct PreparedAction {
    target: ActionTarget,
    operation: Operation,
    execution: Execution,
}

#[derive(Clone, Eq, PartialEq)]
enum Execution {
    Activation(rmac_dock::Activation),
    Context(rmac_dock::ContextAction),
    Rejected(Error),
}

impl fmt::Debug for PreparedAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedAction")
            .field("target", &self.target)
            .field("operation", &self.operation)
            .field("execution", &"<redacted>")
            .finish()
    }
}

impl PreparedAction {
    pub fn target(&self) -> &ActionTarget {
        &self.target
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn begin(self, state: &mut State) -> Result<PendingAction, BeginError> {
        let (ticket, started) = state.begin(self.target.clone(), self.operation)?;
        Ok(PendingAction {
            ticket,
            started,
            action: self,
        })
    }
}

pub struct PendingAction {
    ticket: Ticket,
    started: Transition,
    action: PreparedAction,
}

impl PendingAction {
    pub fn started(&self) -> &Transition {
        &self.started
    }

    pub async fn run(
        self,
        request_id: rmac_compositor::ActivationId,
        backend: &impl Backend,
    ) -> Completion {
        let result = match &self.action.execution {
            Execution::Activation(activation) => {
                crate::execute(activation, request_id, backend).await
            }
            Execution::Context(action) => crate::execute_context(action, request_id, backend).await,
            Execution::Rejected(error) => Err(error.clone()),
        };
        Completion {
            ticket: self.ticket,
            result,
        }
    }
}

pub struct Completion {
    ticket: Ticket,
    result: Result<Outcome, Error>,
}

impl Completion {
    pub fn apply(self, state: &mut State) -> (Result<Outcome, Error>, Transition) {
        let Self { ticket, result } = self;
        let status = match &result {
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
        let transition = state.finish(ticket, status);
        (result, transition)
    }
}

/// Resolve a menu intent against the newest coherent Dock model. The menu may
/// have been open across catalog, compositor, or settings changes; no launch
/// specification, window action, or pin command crosses this boundary stale.
pub fn prepare(
    model: &rmac_dock::Model,
    action: rmac_dock::menu::Action,
) -> Result<Preparation, PrepareError> {
    match action {
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Application(
            requested_id,
        )) => prepare_application_activation(model, &requested_id),
        rmac_dock::menu::Action::ActivateEntry(
            rmac_dock::presentation::EntryId::Special(_)
            | rmac_dock::presentation::EntryId::Overflow,
        ) => Err(PrepareError::UnsupportedEntry),
        rmac_dock::menu::Action::Context(action) => {
            Ok(Preparation::Ready(prepare_context_action(model, &action)))
        }
    }
}

fn prepare_application_activation(
    model: &rmac_dock::Model,
    requested_id: &str,
) -> Result<Preparation, PrepareError> {
    let Some(menu) = model.context_menu(requested_id) else {
        return Ok(Preparation::Ready(rejected(
            ActionTarget::Application(requested_id.to_owned()),
            Operation::Resolve,
            requested_id,
        )));
    };
    let activation = model.activate(&menu.app_id);
    let operation = match &activation {
        rmac_dock::Activation::Launch { .. } => Operation::Launch,
        rmac_dock::Activation::FocusWindow(_) => Operation::Focus,
        rmac_dock::Activation::NoAction => return Ok(Preparation::NoAction),
        rmac_dock::Activation::Unavailable { .. } => Operation::Resolve,
    };
    Ok(Preparation::Ready(PreparedAction {
        target: ActionTarget::Application(menu.app_id),
        operation,
        execution: Execution::Activation(activation),
    }))
}

fn prepare_context_action(
    model: &rmac_dock::Model,
    requested: &rmac_dock::ContextAction,
) -> PreparedAction {
    match revalidate_context(model, requested) {
        Some(action) => {
            let (target, operation, _) = context_identity(&action);
            PreparedAction {
                target,
                operation,
                execution: Execution::Context(action),
            }
        }
        None => {
            let (target, operation, public_id) = context_identity(requested);
            rejected(target, operation, &public_id)
        }
    }
}

fn context_identity(action: &rmac_dock::ContextAction) -> (ActionTarget, Operation, String) {
    match action {
        rmac_dock::ContextAction::LaunchNew { app_id, .. } => (
            ActionTarget::Application(app_id.clone()),
            Operation::Launch,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::FocusWindow { app_id, window } => (
            ActionTarget::Window {
                application: app_id.clone(),
                window: *window,
            },
            Operation::Focus,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::CloseWindow { app_id, window } => (
            ActionTarget::Window {
                application: app_id.clone(),
                window: *window,
            },
            Operation::Close,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::UpdatePins(command) => (
            ActionTarget::Application(command.app_id().to_owned()),
            Operation::UpdatePins,
            command.app_id().to_owned(),
        ),
    }
}

fn revalidate_context(
    model: &rmac_dock::Model,
    requested: &rmac_dock::ContextAction,
) -> Option<rmac_dock::ContextAction> {
    match requested {
        rmac_dock::ContextAction::LaunchNew { app_id, .. } => {
            model.context_menu(app_id)?.launch_new
        }
        rmac_dock::ContextAction::FocusWindow { app_id, window } => model
            .context_menu(app_id)?
            .windows
            .into_iter()
            .find(|candidate| candidate.id == *window)
            .map(|candidate| candidate.focus),
        rmac_dock::ContextAction::CloseWindow { app_id, window } => model
            .context_menu(app_id)?
            .windows
            .into_iter()
            .find(|candidate| candidate.id == *window)
            .map(|candidate| candidate.close),
        rmac_dock::ContextAction::UpdatePins(requested) => {
            let menu = model.context_menu(requested.app_id())?;
            match requested {
                rmac_dock::PinCommand::Pin { .. }
                    if matches!(&menu.pin, rmac_dock::PinCommand::Pin { .. }) =>
                {
                    Some(rmac_dock::ContextAction::UpdatePins(menu.pin))
                }
                rmac_dock::PinCommand::Unpin { .. }
                    if matches!(&menu.pin, rmac_dock::PinCommand::Unpin { .. }) =>
                {
                    Some(rmac_dock::ContextAction::UpdatePins(menu.pin))
                }
                rmac_dock::PinCommand::Move {
                    direction: rmac_dock::MoveDirection::Left,
                    ..
                } => menu.move_left.map(rmac_dock::ContextAction::UpdatePins),
                rmac_dock::PinCommand::Move {
                    direction: rmac_dock::MoveDirection::Right,
                    ..
                } => menu.move_right.map(rmac_dock::ContextAction::UpdatePins),
                rmac_dock::PinCommand::MoveTo { .. }
                | rmac_dock::PinCommand::Pin { .. }
                | rmac_dock::PinCommand::Unpin { .. } => None,
            }
        }
    }
}

fn rejected(target: ActionTarget, operation: Operation, public_id: &str) -> PreparedAction {
    PreparedAction {
        target,
        operation,
        execution: Execution::Rejected(Error::new(
            operation,
            FailureKind::Rejected,
            public_id,
            "the Dock action no longer matches current state",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
    }

    impl Backend for FakeBackend {
        fn launch<'a>(
            &'a self,
            spec: &'a rmac_apps::LaunchSpec,
        ) -> crate::BackendFuture<'a, Result<rmac_app_launch::Outcome, crate::BackendError>>
        {
            Box::pin(async move {
                self.calls.lock().unwrap().push(format!("launch {spec:?}"));
                Ok(rmac_app_launch::Outcome {
                    process_id: Some(7),
                    delivery: rmac_app_launch::Delivery::DirectFallback,
                })
            })
        }

        fn focus_window(
            &self,
            request_id: rmac_compositor::ActivationId,
            window: rmac_compositor::WindowId,
        ) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("focus {} request {}", window.0, request_id.0));
                Ok(())
            })
        }

        fn close_window(
            &self,
            request_id: rmac_compositor::ActivationId,
            window: rmac_compositor::WindowId,
        ) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("close {} request {}", window.0, request_id.0));
                Ok(())
            })
        }

        fn update_pins(
            &self,
            command: &rmac_dock::PinCommand,
        ) -> crate::BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, crate::BackendError>>
        {
            let command = command.clone();
            Box::pin(async move {
                self.calls.lock().unwrap().push(format!("pins {command:?}"));
                Ok(vec![rmac_shell_settings::AppId(
                    command.app_id().to_owned(),
                )])
            })
        }

        fn open_directory(
            &self,
            _: &Path,
        ) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            Box::pin(async { Ok(()) })
        }

        fn open_trash(&self) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn application(program: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: "terminal.desktop".into(),
            name: "Terminal".into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from("/apps/terminal.desktop"),
            icon: None,
            categories: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: program.into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    fn window(id: u64, focused: bool) -> rmac_compositor::Window {
        rmac_compositor::Window {
            id: rmac_compositor::WindowId(id),
            title: Some("private shell title".into()),
            app_id: Some("terminal".into()),
            pid: None,
            workspace: None,
            focused,
            floating: false,
            urgent: false,
            focus_timestamp: None,
            layout: Default::default(),
        }
    }

    fn model(
        program: &str,
        windows: Vec<rmac_compositor::Window>,
        pinned: bool,
    ) -> rmac_dock::Model {
        let pinned_apps = pinned
            .then(|| rmac_shell_settings::AppId("terminal.desktop".into()))
            .into_iter()
            .collect::<Vec<_>>();
        rmac_dock::Model::build(
            &pinned_apps,
            &Default::default(),
            &[application(program)],
            &rmac_compositor::Snapshot {
                windows,
                ..Default::default()
            },
        )
    }

    #[test]
    fn stale_launch_spec_is_replaced_by_the_current_catalog_action() {
        let old = model("old-terminal", Vec::new(), true)
            .context_menu("terminal")
            .unwrap()
            .launch_new
            .unwrap();
        let current = model("current-terminal", Vec::new(), true);
        let Preparation::Ready(prepared) =
            prepare(&current, rmac_dock::menu::Action::Context(old)).unwrap()
        else {
            panic!("launch is ready");
        };
        assert_eq!(prepared.operation(), Operation::Launch);
        assert!(!format!("{prepared:?}").contains("current-terminal"));

        let mut state = State::default();
        let pending = prepared.begin(&mut state).unwrap();
        assert_eq!(pending.started().snapshot.busy.len(), 1);
        let backend = FakeBackend::default();
        let completion =
            futures_lite::future::block_on(pending.run(rmac_compositor::ActivationId(3), &backend));
        let (result, finished) = completion.apply(&mut state);
        assert!(result.is_ok());
        assert!(finished.snapshot.busy.is_empty());
        let calls = backend.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("current-terminal"));
        assert!(!calls[0].contains("old-terminal"));
    }

    #[test]
    fn stale_window_becomes_rejected_feedback_without_touching_backend() {
        let stale = rmac_dock::menu::Action::Context(rmac_dock::ContextAction::FocusWindow {
            app_id: "terminal.desktop".into(),
            window: rmac_compositor::WindowId(9),
        });
        let current = model("terminal", Vec::new(), true);
        let Preparation::Ready(prepared) = prepare(&current, stale).unwrap() else {
            panic!("stale action produces feedback");
        };
        assert_eq!(prepared.operation(), Operation::Focus);

        let mut state = State::default();
        let backend = FakeBackend::default();
        let pending = prepared.begin(&mut state).unwrap();
        let completion =
            futures_lite::future::block_on(pending.run(rmac_compositor::ActivationId(4), &backend));
        let (result, finished) = completion.apply(&mut state);
        assert!(result.is_err());
        assert_eq!(finished.snapshot.feedback.len(), 1);
        assert_eq!(
            finished.snapshot.feedback[0].reason,
            crate::interaction::FeedbackReason::Rejected
        );
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn current_window_action_keeps_exact_target_request_and_completion() {
        let current = model("terminal", vec![window(7, false)], true);
        let mut menu = current.context_menu("terminal").unwrap();
        let action = menu.windows.remove(0).close;
        let Preparation::Ready(prepared) =
            prepare(&current, rmac_dock::menu::Action::Context(action)).unwrap()
        else {
            panic!("window action is ready");
        };
        assert_eq!(
            prepared.target(),
            &ActionTarget::Window {
                application: "terminal.desktop".into(),
                window: rmac_compositor::WindowId(7),
            }
        );
        assert_eq!(prepared.operation(), Operation::Close);

        let mut state = State::default();
        let backend = FakeBackend::default();
        let pending = prepared.begin(&mut state).unwrap();
        let completion = futures_lite::future::block_on(
            pending.run(rmac_compositor::ActivationId(12), &backend),
        );
        let (result, transition) = completion.apply(&mut state);
        assert_eq!(
            result,
            Ok(Outcome::CloseRequested {
                window: rmac_compositor::WindowId(7)
            })
        );
        assert!(transition.snapshot.busy.is_empty());
        assert_eq!(backend.calls.into_inner().unwrap(), ["close 7 request 12"]);
    }

    #[test]
    fn stale_pin_intent_is_rejected_instead_of_becoming_its_opposite() {
        let stale = rmac_dock::menu::Action::Context(rmac_dock::ContextAction::UpdatePins(
            rmac_dock::PinCommand::Unpin {
                app_id: "terminal.desktop".into(),
            },
        ));
        let current = model("terminal", Vec::new(), false);
        let Preparation::Ready(prepared) = prepare(&current, stale).unwrap() else {
            panic!("stale pin produces feedback");
        };
        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            prepared
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, _) = completion.apply(&mut state);
        assert!(result.is_err());
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn a_second_action_for_the_same_target_cannot_start_while_busy() {
        let current = model("terminal", Vec::new(), true);
        let action = current
            .context_menu("terminal")
            .unwrap()
            .launch_new
            .unwrap();
        let Preparation::Ready(first) =
            prepare(&current, rmac_dock::menu::Action::Context(action.clone())).unwrap()
        else {
            panic!("launch is ready");
        };
        let Preparation::Ready(second) =
            prepare(&current, rmac_dock::menu::Action::Context(action)).unwrap()
        else {
            panic!("launch is ready");
        };
        let mut state = State::default();
        let _pending = first.begin(&mut state).unwrap();

        assert!(matches!(
            second.begin(&mut state),
            Err(BeginError::Busy {
                target: ActionTarget::Application(app_id),
            }) if app_id == "terminal.desktop"
        ));
    }

    #[test]
    fn focused_single_window_is_noop_and_malformed_entry_never_tickets() {
        let current = model("terminal", vec![window(7, true)], true);
        assert_eq!(
            prepare(
                &current,
                rmac_dock::menu::Action::ActivateEntry(
                    rmac_dock::presentation::EntryId::Application("terminal.desktop".into())
                )
            ),
            Ok(Preparation::NoAction)
        );
        assert_eq!(
            prepare(
                &current,
                rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Overflow)
            ),
            Err(PrepareError::UnsupportedEntry)
        );
    }
}
