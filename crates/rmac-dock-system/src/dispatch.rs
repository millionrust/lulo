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
    TrashReview(PreparedTrashReview),
}

#[derive(Clone, Eq, PartialEq)]
pub struct PreparedAction {
    target: ActionTarget,
    operation: Operation,
    execution: Execution,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PreparedTrashReview {
    action: rmac_dock::SpecialContextAction,
}

impl fmt::Debug for PreparedTrashReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedTrashReview")
            .field(
                "target",
                &ActionTarget::Special(rmac_dock::SpecialItemKind::Trash),
            )
            .field("operation", &Operation::ReviewTrash)
            .field("action", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
enum Execution {
    Activation(rmac_dock::Activation),
    Context(rmac_dock::ContextAction),
    Reorder(rmac_dock::drag::RevalidatedReorder),
    Special(rmac_dock::SpecialActivation),
    StackActivation(rmac_dock::StackActivation),
    Stack(rmac_dock::StackCommand),
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

impl PreparedTrashReview {
    pub fn begin(self, state: &mut State) -> Result<PendingTrashReview, BeginError> {
        let (ticket, started) = state.begin(
            ActionTarget::Special(rmac_dock::SpecialItemKind::Trash),
            Operation::ReviewTrash,
        )?;
        Ok(PendingTrashReview {
            ticket,
            started,
            action: self.action,
        })
    }
}

pub struct PendingTrashReview {
    ticket: Ticket,
    started: Transition,
    action: rmac_dock::SpecialContextAction,
}

impl PendingTrashReview {
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn started(&self) -> &Transition {
        &self.started
    }

    pub fn cancel(self, state: &mut State) -> Transition {
        state.cancel(self.ticket)
    }

    /// Enumerate and bind the exact Trash identities on a blocking worker.
    pub fn run_blocking(self, backend: &impl rmac_places_system::Backend) -> TrashReviewCompletion {
        let result = crate::prepare_special_context(&self.action, backend)
            .map(|review| ReviewedTrash { review });
        TrashReviewCompletion {
            ticket: self.ticket,
            result,
        }
    }
}

pub struct TrashReviewCompletion {
    ticket: Ticket,
    result: Result<ReviewedTrash, Error>,
}

impl TrashReviewCompletion {
    pub fn apply(self, state: &mut State) -> (Result<ReviewedTrash, Error>, Transition) {
        let Self { ticket, result } = self;
        let status = match &result {
            Ok(_) => Ok(()),
            Err(error) => Err(error),
        };
        let transition = state.finish(ticket, status);
        (result, transition)
    }
}

pub struct ReviewedTrash {
    review: rmac_places_system::EmptyTrashReview,
}

impl fmt::Debug for ReviewedTrash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReviewedTrash")
            .field("item_count", &self.item_count())
            .field("entries", &"<private>")
            .finish()
    }
}

impl ReviewedTrash {
    pub fn item_count(&self) -> usize {
        self.review.item_count()
    }

    /// The destructive capability is created only for an explicit affirmative
    /// response from the confirmation sheet. Decline consumes the review.
    pub fn confirm(self, confirmed: bool) -> Option<ConfirmedTrash> {
        let item_count = self.item_count();
        rmac_places_system::confirm_empty_trash(self.review, confirmed).map(|confirmation| {
            ConfirmedTrash {
                confirmation,
                item_count,
            }
        })
    }
}

pub struct ConfirmedTrash {
    confirmation: rmac_places_system::EmptyTrashConfirmation,
    item_count: usize,
}

impl fmt::Debug for ConfirmedTrash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfirmedTrash")
            .field("item_count", &self.item_count)
            .field("entries", &"<private>")
            .finish()
    }
}

impl ConfirmedTrash {
    pub fn item_count(&self) -> usize {
        self.item_count
    }

    pub fn begin(self, state: &mut State) -> Result<PendingEmptyTrash, BeginError> {
        let (ticket, started) = state.begin(
            ActionTarget::Special(rmac_dock::SpecialItemKind::Trash),
            Operation::EmptyTrash,
        )?;
        Ok(PendingEmptyTrash {
            ticket,
            started,
            confirmation: self.confirmation,
        })
    }
}

pub struct PendingEmptyTrash {
    ticket: Ticket,
    started: Transition,
    confirmation: rmac_places_system::EmptyTrashConfirmation,
}

impl PendingEmptyTrash {
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn started(&self) -> &Transition {
        &self.started
    }

    pub fn cancel(self, state: &mut State) -> Transition {
        state.cancel(self.ticket)
    }

    /// Permanently delete the reviewed identities on a blocking worker.
    pub fn run_blocking(self, backend: &impl rmac_places_system::Backend) -> Completion {
        Completion {
            ticket: self.ticket,
            result: crate::execute_empty_trash(self.confirmation, backend),
        }
    }
}

pub struct PendingAction {
    ticket: Ticket,
    started: Transition,
    action: PreparedAction,
}

impl PendingAction {
    pub fn ticket(&self) -> &Ticket {
        &self.ticket
    }

    pub fn started(&self) -> &Transition {
        &self.started
    }

    pub fn cancel(self, state: &mut State) -> Transition {
        state.cancel(self.ticket)
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
            Execution::Reorder(reorder) => execute_reorder(reorder, backend).await,
            Execution::Special(activation) => crate::execute_special(activation, backend).await,
            Execution::StackActivation(activation) => {
                crate::execute_stack_activation(activation, backend).await
            }
            Execution::Stack(command) => execute_stack(command, backend).await,
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
/// specification, window action, pin command, or private place authority
/// crosses this boundary stale.
pub fn prepare(
    model: &rmac_dock::Model,
    action: rmac_dock::menu::Action,
) -> Result<Preparation, PrepareError> {
    match action {
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Application(
            requested_id,
        )) => prepare_application_activation(model, &requested_id),
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Special(kind)) => {
            Ok(Preparation::Ready(prepare_special_activation(model, kind)))
        }
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Minimized(
            window,
        )) => Ok(Preparation::Ready(prepare_minimized_activation(
            model, window,
        ))),
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Overflow) => {
            Err(PrepareError::UnsupportedEntry)
        }
        rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Stack(kind)) => {
            Ok(Preparation::Ready(prepare_stack_activation(model, kind)))
        }
        rmac_dock::menu::Action::Context(action) => {
            Ok(Preparation::Ready(prepare_context_action(model, &action)))
        }
        rmac_dock::menu::Action::SpecialContext(action) => Ok(prepare_trash_review(model, &action)),
        rmac_dock::menu::Action::StackContext(command) => Ok(Preparation::Ready(
            prepare_stack_context_action(model, &command),
        )),
    }
}

/// Revalidate a completed direct-manipulation intent against the newest Dock
/// model before it can mutate persisted pin order.
pub fn prepare_reorder(
    model: &rmac_dock::Model,
    intent: &rmac_dock::drag::ReorderIntent,
) -> Preparation {
    let Some(reorder) = intent.revalidate(model) else {
        return Preparation::Ready(rejected(
            ActionTarget::Application(intent.app_id().to_owned()),
            Operation::UpdatePins,
            intent.app_id(),
        ));
    };
    Preparation::Ready(PreparedAction {
        target: ActionTarget::Application(reorder.command().app_id().to_owned()),
        operation: Operation::UpdatePins,
        execution: Execution::Reorder(reorder),
    })
}

async fn execute_reorder(
    reorder: &rmac_dock::drag::RevalidatedReorder,
    backend: &impl Backend,
) -> Result<Outcome, Error> {
    backend
        .reorder_pins(reorder)
        .await
        .map(|pinned| Outcome::PinsUpdated { pinned })
        .map_err(|error| {
            Error::new(
                Operation::UpdatePins,
                error.kind,
                reorder.command().app_id(),
                error.detail,
            )
        })
}

/// A path-free identity for error messages and `Operation` context; a
/// stack's real folder never appears here (matches `rmac_dock`'s own
/// `DockStackKind` Debug redaction).
fn stack_item_id(kind: &rmac_shell_settings::DockStackKind) -> String {
    match kind {
        rmac_shell_settings::DockStackKind::Downloads => "Downloads".into(),
        rmac_shell_settings::DockStackKind::Path { .. } => "stack".into(),
    }
}

async fn execute_stack(
    command: &rmac_dock::StackCommand,
    backend: &impl Backend,
) -> Result<Outcome, Error> {
    backend
        .update_stacks(command)
        .await
        .map(|stacks| Outcome::StacksUpdated { stacks })
        .map_err(|error| {
            Error::new(
                Operation::UpdateStacks,
                error.kind,
                stack_item_id(command.kind()),
                error.detail,
            )
        })
}

fn prepare_trash_review(
    model: &rmac_dock::Model,
    requested: &rmac_dock::SpecialContextAction,
) -> Preparation {
    let current = model
        .special_context_menu(rmac_dock::SpecialItemKind::Trash)
        .and_then(|menu| menu.empty_trash);
    match current {
        Some(action) if &action == requested => {
            Preparation::TrashReview(PreparedTrashReview { action })
        }
        _ => Preparation::Ready(rejected(
            ActionTarget::Special(rmac_dock::SpecialItemKind::Trash),
            Operation::ReviewTrash,
            "Trash",
        )),
    }
}

fn prepare_special_activation(
    model: &rmac_dock::Model,
    kind: rmac_dock::SpecialItemKind,
) -> PreparedAction {
    let activation = model.activate_special(kind);
    let operation = match &activation {
        rmac_dock::SpecialActivation::OpenDirectory { .. }
        | rmac_dock::SpecialActivation::OpenTrash => Operation::OpenPlace,
        rmac_dock::SpecialActivation::Unavailable { .. } => Operation::Resolve,
    };
    PreparedAction {
        target: ActionTarget::Special(kind),
        operation,
        execution: Execution::Special(activation),
    }
}

/// A stack's "Open <name>" opens it in Files, exactly like
/// `prepare_special_activation`; the popover a plain click on the stack
/// shows is Dock-local UI state that never reaches this dispatch layer.
fn prepare_stack_activation(
    model: &rmac_dock::Model,
    kind: rmac_shell_settings::DockStackKind,
) -> PreparedAction {
    let activation = model.activate_stack(&kind);
    let operation = match &activation {
        rmac_dock::StackActivation::OpenDirectory { .. } => Operation::OpenPlace,
        rmac_dock::StackActivation::Unavailable { .. } => Operation::Resolve,
    };
    PreparedAction {
        target: ActionTarget::Stack(kind),
        operation,
        execution: Execution::StackActivation(activation),
    }
}

/// Revalidate a Sort By/Display As/View Content As/Remove request against
/// the newest Dock model: the stack must still be kept. The requested
/// choice itself is not re-derived from server state (unlike Pin/Unpin),
/// since every `StackCommand` variant already states the user's full
/// intent, the same way `PinCommand::Move`/`MoveTo` do.
fn prepare_stack_context_action(
    model: &rmac_dock::Model,
    requested: &rmac_dock::StackCommand,
) -> PreparedAction {
    let kind = requested.kind().clone();
    match model.stack_context_menu(&kind) {
        Some(_) => PreparedAction {
            target: ActionTarget::Stack(kind),
            operation: Operation::UpdateStacks,
            execution: Execution::Stack(requested.clone()),
        },
        None => rejected(
            ActionTarget::Stack(kind.clone()),
            Operation::UpdateStacks,
            &stack_item_id(&kind),
        ),
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
        rmac_dock::Activation::FocusWindow(_) | rmac_dock::Activation::FocusApplication { .. } => {
            Operation::Focus
        }
        // An application whose windows are all minimized restores the most
        // recently minimized one, as the macOS Dock does.
        rmac_dock::Activation::RestoreWindow { .. } => Operation::Restore,
        rmac_dock::Activation::NoAction => return Ok(Preparation::NoAction),
        rmac_dock::Activation::Unavailable { .. } => Operation::Resolve,
    };
    Ok(Preparation::Ready(PreparedAction {
        target: ActionTarget::Application(menu.app_id),
        operation,
        execution: Execution::Activation(activation),
    }))
}

/// Resolve a minimized-tile click against the newest Dock projection. The
/// window must still be parked, so a tile left over from a restore fails
/// closed instead of moving an unrelated window.
fn prepare_minimized_activation(
    model: &rmac_dock::Model,
    window: rmac_compositor::WindowId,
) -> PreparedAction {
    let application = model
        .minimized
        .iter()
        .find(|item| item.window == window)
        .and_then(|item| item.app_id.clone())
        .unwrap_or_default();
    let target = ActionTarget::Window {
        application,
        window,
    };
    match model.activate_minimized(window) {
        activation @ rmac_dock::Activation::RestoreWindow { .. } => PreparedAction {
            target,
            operation: Operation::Restore,
            execution: Execution::Activation(activation),
        },
        _ => rejected(target, Operation::Restore, &format!("window {}", window.0)),
    }
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
        rmac_dock::ContextAction::RevealApplication { app_id, .. } => (
            ActionTarget::Application(app_id.clone()),
            Operation::Reveal,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::TerminateApplication { app_id, .. } => (
            ActionTarget::Application(app_id.clone()),
            Operation::Terminate,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::UpdatePins(command) => (
            ActionTarget::Application(command.app_id().to_owned()),
            Operation::UpdatePins,
            command.app_id().to_owned(),
        ),
        rmac_dock::ContextAction::HideApplication { app_id, .. }
        | rmac_dock::ContextAction::HideOthers { app_id, .. } => (
            ActionTarget::Application(app_id.clone()),
            Operation::Hide,
            app_id.clone(),
        ),
        rmac_dock::ContextAction::ShowAllWindows { app_id, .. } => (
            ActionTarget::Application(app_id.clone()),
            Operation::ShowAllWindows,
            app_id.clone(),
        ),
    }
}

fn revalidate_context(
    model: &rmac_dock::Model,
    requested: &rmac_dock::ContextAction,
) -> Option<rmac_dock::ContextAction> {
    match requested {
        rmac_dock::ContextAction::LaunchNew { app_id, .. } => {
            let menu = model.context_menu(app_id)?;
            menu.application_commands
                .into_iter()
                .map(|command| command.action)
                .find(|candidate| candidate == requested)
                .or(menu.open)
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
        rmac_dock::ContextAction::RevealApplication { app_id, .. } => model
            .context_menu(app_id)?
            .show_in_finder
            .filter(|candidate| candidate == requested),
        rmac_dock::ContextAction::TerminateApplication { app_id, kind, .. } => {
            let menu = model.context_menu(app_id)?;
            match kind {
                rmac_dock::TerminationKind::Quit => menu.quit,
                rmac_dock::TerminationKind::ForceQuit => menu.force_quit,
            }
            .filter(|candidate| candidate == requested)
        }
        rmac_dock::ContextAction::HideApplication { app_id, .. } => model
            .context_menu(app_id)?
            .hide
            .filter(|candidate| candidate == requested),
        rmac_dock::ContextAction::HideOthers { app_id, .. } => model
            .context_menu(app_id)?
            .hide_others
            .filter(|candidate| candidate == requested),
        rmac_dock::ContextAction::ShowAllWindows { app_id, .. } => model
            .context_menu(app_id)?
            .show_all_windows
            .filter(|candidate| candidate == requested),
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
                rmac_dock::PinCommand::Move { .. } | rmac_dock::PinCommand::MoveTo { .. } => {
                    let action = rmac_dock::ContextAction::UpdatePins(requested.clone());
                    model.authorizes_context_action(&action).then_some(action)
                }
                rmac_dock::PinCommand::Pin { .. } | rmac_dock::PinCommand::Unpin { .. } => None,
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
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        calls: Mutex<Vec<String>>,
    }

    #[derive(Default)]
    struct FakeTrashBackend {
        entries: Mutex<Vec<rmac_places_system::TrashEntryId>>,
        purged: Mutex<Vec<rmac_places_system::TrashEntryId>>,
    }

    impl rmac_places_system::Backend for FakeTrashBackend {
        fn home(&self) -> Option<PathBuf> {
            Some("/home/alex".into())
        }

        fn config_home(&self) -> Option<PathBuf> {
            None
        }

        fn read_optional(&self, _: &Path) -> io::Result<Option<String>> {
            Ok(None)
        }

        fn exists(&self, _: &Path) -> io::Result<bool> {
            Ok(true)
        }

        fn trash_count(&self) -> Result<usize, String> {
            Ok(self.entries.lock().unwrap().len())
        }

        fn trash_entries(&self) -> Result<Vec<rmac_places_system::TrashEntryId>, String> {
            Ok(self.entries.lock().unwrap().clone())
        }

        fn purge_trash(&self, reviewed: &[rmac_places_system::TrashEntryId]) -> Result<(), String> {
            let mut entries = self.entries.lock().unwrap();
            if reviewed.iter().any(|entry| !entries.contains(entry)) {
                return Err("Trash changed after review".into());
            }
            self.purged.lock().unwrap().extend_from_slice(reviewed);
            entries.retain(|entry| !reviewed.contains(entry));
            Ok(())
        }
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

        fn reorder_pins(
            &self,
            reorder: &rmac_dock::drag::RevalidatedReorder,
        ) -> crate::BackendFuture<'_, Result<Vec<rmac_shell_settings::AppId>, crate::BackendError>>
        {
            let reorder = reorder.clone();
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("reorder {reorder:?}"));
                Ok(reorder.expected_order().to_vec())
            })
        }

        fn open_directory(
            &self,
            path: &Path,
        ) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            let path = path.to_path_buf();
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("directory {}", path.display()));
                Ok(())
            })
        }

        fn open_trash(&self) -> crate::BackendFuture<'_, Result<(), crate::BackendError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("trash".into());
                Ok(())
            })
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
            mime_types: Vec::new(),
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

    fn places(
        downloads: &str,
        downloads_exists: bool,
        trash_count: usize,
    ) -> rmac_places::Snapshot {
        rmac_places::Snapshot {
            home: rmac_places::Place {
                path: PathBuf::from("/home/alex"),
                exists: true,
            },
            downloads: rmac_places::Place {
                path: PathBuf::from(downloads),
                exists: downloads_exists,
            },
            downloads_configured: true,
            trash: rmac_places::TrashSnapshot {
                available: true,
                empty: trash_count == 0,
                item_count: trash_count,
            },
        }
    }

    fn model_with_places(
        downloads: &str,
        downloads_exists: bool,
        trash_count: usize,
    ) -> rmac_dock::Model {
        rmac_dock::Model::build_with_places(
            &[],
            &Default::default(),
            &[application("terminal")],
            &Default::default(),
            &places(downloads, downloads_exists, trash_count),
        )
    }

    fn reorder_model(order: &[&str]) -> rmac_dock::Model {
        let pinned = order
            .iter()
            .map(|id| rmac_shell_settings::AppId((*id).into()))
            .collect::<Vec<_>>();
        let catalog = order
            .iter()
            .map(|id| {
                let mut application = application(id);
                application.id = (*id).into();
                application.name = id.trim_end_matches(".desktop").into();
                application.source = PathBuf::from(format!("/apps/{id}"));
                application
            })
            .collect::<Vec<_>>();
        rmac_dock::Model::build(&pinned, &Default::default(), &catalog, &Default::default())
    }

    fn reorder_intent(
        model: &rmac_dock::Model,
        source_index: usize,
        destination_index: usize,
    ) -> rmac_dock::drag::ReorderIntent {
        let content = rmac_dock::presentation::ShelfContent::project(model);
        let plan = content
            .prepare_layout(&rmac_dock::SurfaceDescription {
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
                magnification: rmac_dock::motion::MagnificationConfig::default(),
            })
            .unwrap();
        let resting = plan.layout(None).unwrap();
        let source = resting.slots[source_index].id.clone();
        let mut drag = rmac_dock::drag::DragSession::begin(
            model,
            &plan,
            &source,
            resting.slots[source_index].center,
        )
        .unwrap();
        drag.update(resting.slots[destination_index].center)
            .unwrap();
        let rmac_dock::drag::DropOutcome::Reorder(intent) = drag.finish() else {
            panic!("test drag produces reorder intent");
        };
        intent
    }

    #[test]
    fn stale_launch_spec_is_replaced_by_the_current_catalog_action() {
        let old = model("old-terminal", Vec::new(), true)
            .context_menu("terminal")
            .unwrap()
            .open
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
        let action = current.context_menu("terminal").unwrap().open.unwrap();
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
    fn renderer_cancellation_makes_a_late_dispatch_completion_inert() {
        let current = model("terminal", Vec::new(), true);
        let action = current.context_menu("terminal").unwrap().open.unwrap();
        let Preparation::Ready(prepared) =
            prepare(&current, rmac_dock::menu::Action::Context(action)).unwrap()
        else {
            panic!("launch is ready");
        };
        let mut state = State::default();
        let backend = FakeBackend::default();
        let pending = prepared.begin(&mut state).unwrap();
        let cancellation = pending.ticket().clone();
        let completion =
            futures_lite::future::block_on(pending.run(rmac_compositor::ActivationId(1), &backend));
        let cancelled = state.cancel(cancellation);
        let (_, late) = completion.apply(&mut state);

        assert!(cancelled.visible);
        assert!(cancelled.snapshot.busy.is_empty());
        assert!(!late.visible);
        assert!(late.snapshot.feedback.is_empty());
    }

    #[test]
    fn current_drag_reorder_uses_ticketed_pin_persistence() {
        let current = reorder_model(&["finder.desktop", "terminal.desktop", "notes.desktop"]);
        let intent = reorder_intent(&current, 0, 2);
        let Preparation::Ready(prepared) = prepare_reorder(&current, &intent) else {
            panic!("current drag is ready");
        };
        assert_eq!(prepared.operation(), Operation::UpdatePins);
        assert_eq!(
            prepared.target(),
            &ActionTarget::Application("finder.desktop".into())
        );
        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            prepared
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, transition) = completion.apply(&mut state);

        assert!(result.is_ok());
        assert!(transition.snapshot.busy.is_empty());
        let calls = backend.calls.into_inner().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("MoveTo { app_id: \"finder.desktop\", index: 2 }"));
        assert!(calls[0].contains("expected_order"));
    }

    #[test]
    fn drag_reorder_is_rejected_if_live_pin_order_changed_before_dispatch() {
        let original = reorder_model(&["finder.desktop", "terminal.desktop", "notes.desktop"]);
        let intent = reorder_intent(&original, 0, 2);
        let changed = reorder_model(&["finder.desktop", "notes.desktop", "terminal.desktop"]);
        let Preparation::Ready(rejected) = prepare_reorder(&changed, &intent) else {
            panic!("stale drag produces feedback");
        };
        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            rejected
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, transition) = completion.apply(&mut state);

        assert!(result.is_err());
        assert_eq!(
            transition.snapshot.feedback[0].reason,
            crate::interaction::FeedbackReason::Rejected
        );
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn downloads_is_not_a_dock_place_until_folder_stacks_are_configured() {
        // The Dock's only permanent place is the Trash (see
        // `rmac_dock::project_special_items`); a stale Downloads activation
        // becomes feedback and never opens the private folder.
        let current = model_with_places("/home/alex/Current Downloads", true, 3);
        let Preparation::Ready(prepared) = prepare(
            &current,
            rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Special(
                rmac_dock::SpecialItemKind::Downloads,
            )),
        )
        .unwrap() else {
            panic!("Downloads produces feedback");
        };
        assert_eq!(prepared.operation(), Operation::Resolve);
        assert_eq!(
            prepared.target(),
            &ActionTarget::Special(rmac_dock::SpecialItemKind::Downloads)
        );
        assert!(!format!("{prepared:?}").contains("Current Downloads"));

        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            prepared
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, transition) = completion.apply(&mut state);
        assert!(matches!(
            result,
            Err(Error {
                kind: FailureKind::Unavailable,
                ..
            })
        ));
        assert!(transition.snapshot.busy.is_empty());
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn unavailable_special_entry_becomes_feedback_without_backend_work() {
        let current = model("terminal", Vec::new(), true);
        let Preparation::Ready(prepared) = prepare(
            &current,
            rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Special(
                rmac_dock::SpecialItemKind::Files,
            )),
        )
        .unwrap() else {
            panic!("unavailable Files produces feedback");
        };
        assert_eq!(prepared.operation(), Operation::Resolve);

        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            prepared
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, transition) = completion.apply(&mut state);
        assert!(matches!(
            result,
            Err(Error {
                kind: FailureKind::Unavailable,
                ..
            })
        ));
        assert_eq!(
            transition.snapshot.feedback[0].reason,
            crate::interaction::FeedbackReason::ServiceUnavailable
        );
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn trash_uses_the_same_ticketed_special_dispatch_path() {
        let current = model_with_places("/home/alex/Downloads", true, 3);
        let Preparation::Ready(prepared) = prepare(
            &current,
            rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Special(
                rmac_dock::SpecialItemKind::Trash,
            )),
        )
        .unwrap() else {
            panic!("Trash is ready");
        };
        let mut state = State::default();
        let backend = FakeBackend::default();
        let pending = prepared.begin(&mut state).unwrap();
        assert!(pending
            .started()
            .snapshot
            .busy
            .contains(&ActionTarget::Special(rmac_dock::SpecialItemKind::Trash)));
        let completion =
            futures_lite::future::block_on(pending.run(rmac_compositor::ActivationId(1), &backend));
        let (result, _) = completion.apply(&mut state);
        assert_eq!(
            result,
            Ok(Outcome::PlaceOpened {
                kind: rmac_dock::SpecialItemKind::Trash,
            })
        );
        assert_eq!(backend.calls.into_inner().unwrap(), ["trash"]);
    }

    #[test]
    fn special_busy_state_is_scoped_by_typed_identity() {
        let current = model_with_places("/home/alex/Downloads", true, 3);
        let action = |kind| {
            let Preparation::Ready(prepared) = prepare(
                &current,
                rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Special(
                    kind,
                )),
            )
            .unwrap() else {
                panic!("special entry is ready");
            };
            prepared
        };
        let mut state = State::default();
        let _downloads = action(rmac_dock::SpecialItemKind::Downloads)
            .begin(&mut state)
            .unwrap();
        let _files = action(rmac_dock::SpecialItemKind::Files)
            .begin(&mut state)
            .unwrap();

        assert!(matches!(
            action(rmac_dock::SpecialItemKind::Downloads).begin(&mut state),
            Err(BeginError::Busy {
                target: ActionTarget::Special(rmac_dock::SpecialItemKind::Downloads),
            })
        ));
        assert_eq!(state.snapshot().busy.len(), 2);
    }

    #[test]
    fn reviewed_trash_requires_confirmation_and_deletes_only_bound_identities() {
        let first = rmac_places_system::TrashEntryId::from_authority_bytes(b"first");
        let second = rmac_places_system::TrashEntryId::from_authority_bytes(b"second");
        let backend = FakeTrashBackend {
            entries: Mutex::new(vec![first, second]),
            ..Default::default()
        };
        let current = model_with_places("/home/alex/Downloads", true, 2);
        let action = current
            .special_context_menu(rmac_dock::SpecialItemKind::Trash)
            .unwrap()
            .empty_trash
            .unwrap();
        let Preparation::TrashReview(prepared) =
            prepare(&current, rmac_dock::menu::Action::SpecialContext(action)).unwrap()
        else {
            panic!("current Trash action requires review");
        };
        assert!(!format!("{prepared:?}").contains("first"));

        let mut state = State::default();
        let pending = prepared.begin(&mut state).unwrap();
        assert!(pending
            .started()
            .snapshot
            .busy
            .contains(&ActionTarget::Special(rmac_dock::SpecialItemKind::Trash)));
        let (review, reviewed) = pending.run_blocking(&backend).apply(&mut state);
        let review = review.unwrap();
        assert_eq!(review.item_count(), 2);
        assert!(reviewed.snapshot.busy.is_empty());
        assert!(!format!("{review:?}").contains("first"));

        let confirmed = review.confirm(true).expect("affirmative confirmation");
        assert_eq!(confirmed.item_count(), 2);
        let later = rmac_places_system::TrashEntryId::from_authority_bytes(b"later");
        backend.entries.lock().unwrap().push(later);
        let deletion = confirmed.begin(&mut state).unwrap();
        assert!(deletion
            .started()
            .snapshot
            .busy
            .contains(&ActionTarget::Special(rmac_dock::SpecialItemKind::Trash)));
        let (outcome, finished) = deletion.run_blocking(&backend).apply(&mut state);

        assert_eq!(outcome, Ok(Outcome::TrashEmptied { remaining_items: 1 }));
        assert!(finished.snapshot.busy.is_empty());
        assert_eq!(*backend.entries.lock().unwrap(), [later]);
        assert_eq!(backend.purged.lock().unwrap().len(), 2);
    }

    #[test]
    fn declining_the_sheet_consumes_review_without_deleting() {
        let entry = rmac_places_system::TrashEntryId::from_authority_bytes(b"kept");
        let backend = FakeTrashBackend {
            entries: Mutex::new(vec![entry]),
            ..Default::default()
        };
        let current = model_with_places("/home/alex/Downloads", true, 1);
        let action = current
            .special_context_menu(rmac_dock::SpecialItemKind::Trash)
            .unwrap()
            .empty_trash
            .unwrap();
        let Preparation::TrashReview(prepared) =
            prepare(&current, rmac_dock::menu::Action::SpecialContext(action)).unwrap()
        else {
            panic!("current Trash action requires review");
        };
        let mut state = State::default();
        let (review, _) = prepared
            .begin(&mut state)
            .unwrap()
            .run_blocking(&backend)
            .apply(&mut state);

        assert!(review.unwrap().confirm(false).is_none());
        assert_eq!(*backend.entries.lock().unwrap(), [entry]);
        assert!(backend.purged.lock().unwrap().is_empty());
        assert!(state.snapshot().busy.is_empty());
    }

    #[test]
    fn stale_trash_menu_count_is_rejected_before_filesystem_review() {
        let current = model_with_places("/home/alex/Downloads", true, 2);
        let Preparation::Ready(rejected) = prepare(
            &current,
            rmac_dock::menu::Action::SpecialContext(rmac_dock::SpecialContextAction::EmptyTrash {
                expected_item_count: 3,
            }),
        )
        .unwrap() else {
            panic!("stale Trash action becomes feedback");
        };
        assert_eq!(rejected.operation(), Operation::ReviewTrash);
        let mut state = State::default();
        let backend = FakeBackend::default();
        let completion = futures_lite::future::block_on(
            rejected
                .begin(&mut state)
                .unwrap()
                .run(rmac_compositor::ActivationId(1), &backend),
        );
        let (result, transition) = completion.apply(&mut state);

        assert!(result.is_err());
        assert_eq!(
            transition.snapshot.feedback[0].reason,
            crate::interaction::FeedbackReason::Rejected
        );
        assert!(backend.calls.into_inner().unwrap().is_empty());
    }

    #[test]
    fn changed_trash_authority_fails_review_with_ticketed_feedback() {
        let only = rmac_places_system::TrashEntryId::from_authority_bytes(b"only");
        let backend = FakeTrashBackend {
            entries: Mutex::new(vec![only]),
            ..Default::default()
        };
        let current = model_with_places("/home/alex/Downloads", true, 2);
        let action = current
            .special_context_menu(rmac_dock::SpecialItemKind::Trash)
            .unwrap()
            .empty_trash
            .unwrap();
        let Preparation::TrashReview(prepared) =
            prepare(&current, rmac_dock::menu::Action::SpecialContext(action)).unwrap()
        else {
            panic!("model accepts review before filesystem revalidation");
        };
        let mut state = State::default();
        let (result, transition) = prepared
            .begin(&mut state)
            .unwrap()
            .run_blocking(&backend)
            .apply(&mut state);

        assert!(matches!(
            result,
            Err(Error {
                operation: Operation::ReviewTrash,
                kind: FailureKind::Rejected,
                ..
            })
        ));
        assert_eq!(
            transition.snapshot.feedback[0].reason,
            crate::interaction::FeedbackReason::Rejected
        );
        assert!(backend.purged.lock().unwrap().is_empty());
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

    #[test]
    fn minimized_tiles_restore_the_parked_window_and_reject_stale_ones() {
        let place = |id: u64, name: Option<&str>| rmac_compositor::Workspace {
            id: rmac_compositor::WorkspaceId(id),
            index: id as u8,
            name: name.map(str::to_owned),
            output: None,
            urgent: false,
            active: false,
            focused: false,
            active_window: None,
        };
        let mut parked = window(9, false);
        parked.workspace = Some(rmac_compositor::WorkspaceId(2));
        let current = rmac_dock::Model::build(
            &[],
            &Default::default(),
            &[application("terminal")],
            &rmac_compositor::Snapshot {
                workspaces: vec![
                    place(1, None),
                    place(2, Some(rmac_compositor::PARKING_WORKSPACE)),
                ],
                windows: vec![parked],
                ..Default::default()
            },
        );

        let Preparation::Ready(prepared) = prepare(
            &current,
            rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Minimized(
                rmac_compositor::WindowId(9),
            )),
        )
        .unwrap() else {
            panic!("a parked window is ready to restore");
        };
        assert_eq!(prepared.operation(), Operation::Restore);
        assert_eq!(
            prepared.target(),
            &ActionTarget::Window {
                application: "terminal".into(),
                window: rmac_compositor::WindowId(9),
            }
        );

        // A tile left over from another window fails closed.
        let Preparation::Ready(stale) = prepare(
            &current,
            rmac_dock::menu::Action::ActivateEntry(rmac_dock::presentation::EntryId::Minimized(
                rmac_compositor::WindowId(404),
            )),
        )
        .unwrap() else {
            panic!("stale minimized tiles produce feedback");
        };
        assert_eq!(stale.operation(), Operation::Restore);
    }
}
