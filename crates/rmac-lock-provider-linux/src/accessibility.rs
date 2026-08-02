//! Credential-free accessibility semantics for the secure lock provider.

use std::collections::BTreeSet;
use std::fmt;

use rmac_lock_provider::{OutputId, Phase};
use zeroize::Zeroize as _;

use crate::keyboard::DecodedKey;
use crate::paint::{LockVisualState, PromptVisual};
use crate::pam_conversation::RequestKind;
use crate::prompt_label::{AccountLabel, PromptLabel, PromptText};

pub const LOCK_SCREEN_NAME: &str = "Lock Screen";
pub const MAX_ACCESSIBLE_SURFACES: usize = 32;
pub const MAX_ACCESSIBLE_TEXT_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibleRole {
    Application,
    Window,
    Dialog,
    PasswordTextField,
    TextField,
    Status,
    Alert,
    RadioGroup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LivePoliteness {
    Off,
    Polite,
    Assertive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockAction {
    Submit,
    Cancel,
    SelectPrevious,
    SelectNext,
    ToggleSelection,
}

impl LockAction {
    /// Routes an assistive action through the same bounded input authority as
    /// the physical keyboard and pointer. No action can authorize unlock.
    pub fn into_input(self) -> DecodedKey {
        match self {
            Self::Submit => DecodedKey::Submit,
            Self::Cancel => DecodedKey::Cancel,
            Self::SelectPrevious => DecodedKey::SelectPrevious,
            Self::SelectNext => DecodedKey::SelectNext,
            Self::ToggleSelection => DecodedKey::ToggleSelection,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibleAction {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: LockAction,
    pub enabled: bool,
}

/// Account and PAM guidance text admitted for physical presentation. The value
/// can be scoped into an accessibility adapter, is always redacted from custom
/// diagnostics, and is zeroized with the semantic snapshot.
#[derive(Eq, PartialEq)]
pub struct AccessibleText(String);

impl AccessibleText {
    fn new(value: &str, budget: &mut TextBudget) -> Result<Self, AccessibilityProjectionError> {
        budget.add(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn expose<R>(&self, use_text: impl FnOnce(&str) -> R) -> R {
        use_text(&self.0)
    }
}

impl fmt::Debug for AccessibleText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccessibleText(<redacted>)")
    }
}

impl Drop for AccessibleText {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Eq, PartialEq)]
pub struct AccessibleSurface {
    /// Exact private host target. Custom diagnostics never print it.
    pub output: OutputId,
    pub id: String,
    pub role: AccessibleRole,
    pub name: &'static str,
    pub frame_committed: bool,
    pub position_in_set: usize,
    pub set_size: usize,
}

impl fmt::Debug for AccessibleSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleSurface")
            .field("output", &"<redacted>")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("frame_committed", &self.frame_committed)
            .field("position_in_set", &self.position_in_set)
            .field("set_size", &self.set_size)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
    Password,
    Text,
    Information,
    Error,
    Radio,
    UnsupportedBinary,
}

#[derive(Eq, PartialEq)]
pub struct AccessiblePrompt {
    pub id: &'static str,
    pub role: AccessibleRole,
    pub kind: PromptKind,
    pub name: AccessibleText,
    /// True only for a PAM echo-off field. No value or character count is ever
    /// present in this semantic structure.
    pub protected: bool,
    pub editable: bool,
    /// A single non-sensitive state bit, never the credential length.
    pub has_value: bool,
    pub selected: Option<bool>,
    pub focused: bool,
    pub available: bool,
    pub live_region: LivePoliteness,
    pub actions: Vec<AccessibleAction>,
}

impl fmt::Debug for AccessiblePrompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessiblePrompt")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("protected", &self.protected)
            .field("editable", &self.editable)
            .field("has_value", &self.has_value)
            .field("selected", &self.selected)
            .field("focused", &self.focused)
            .field("available", &self.available)
            .field("live_region", &self.live_region)
            .field("actions", &self.actions)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibleStatus {
    pub id: &'static str,
    pub role: AccessibleRole,
    pub text: &'static str,
    pub live_region: LivePoliteness,
}

#[derive(Eq, PartialEq)]
pub struct AccessibleDialog {
    pub id: &'static str,
    pub role: AccessibleRole,
    pub name: &'static str,
    pub account: AccessibleText,
    pub state: AccessibleStatus,
    pub prompt: Option<AccessiblePrompt>,
    pub caps_lock: Option<AccessibleStatus>,
    pub keyboard_focus_available: bool,
}

impl fmt::Debug for AccessibleDialog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessibleDialog")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("name", &self.name)
            .field("account", &self.account)
            .field("state", &self.state)
            .field("has_prompt", &self.prompt.is_some())
            .field("caps_lock", &self.caps_lock)
            .field("keyboard_focus_available", &self.keyboard_focus_available)
            .finish()
    }
}

#[derive(Eq, PartialEq)]
pub struct LockAccessibilitySnapshot {
    pub role: AccessibleRole,
    pub name: &'static str,
    pub phase: Phase,
    pub surfaces: Vec<AccessibleSurface>,
    /// One logical authentication dialog is shared by the mirrored secure
    /// output surfaces, preventing duplicate Orca reading/action trees.
    pub dialog: AccessibleDialog,
    /// Presentation completeness only. Compositor `locked` remains the sole
    /// security-readiness authority.
    pub all_frames_committed: bool,
}

impl fmt::Debug for LockAccessibilitySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LockAccessibilitySnapshot")
            .field("role", &self.role)
            .field("name", &self.name)
            .field("phase", &self.phase)
            .field("surface_count", &self.surfaces.len())
            .field("dialog", &self.dialog)
            .field("all_frames_committed", &self.all_frames_committed)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessibilityProjectionError {
    InvalidAccount,
    SurfaceLimit,
    DuplicateOutput,
    UnsortedOutput,
    InvalidPresentation,
    InvalidText,
    TextLimit,
}

pub(crate) fn project_lock_accessibility(
    account: &str,
    phase: Phase,
    outputs: &[(OutputId, bool)],
    visual: LockVisualState,
    prompt: Option<PromptText<'_>>,
) -> Result<LockAccessibilitySnapshot, AccessibilityProjectionError> {
    if outputs.len() > MAX_ACCESSIBLE_SURFACES {
        return Err(AccessibilityProjectionError::SurfaceLimit);
    }
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for (output, _) in outputs {
        if !seen.insert(*output) {
            return Err(AccessibilityProjectionError::DuplicateOutput);
        }
        if previous.is_some_and(|previous| previous >= *output) {
            return Err(AccessibilityProjectionError::UnsortedOutput);
        }
        previous = Some(*output);
    }

    let account =
        AccountLabel::new(account).map_err(|_| AccessibilityProjectionError::InvalidAccount)?;
    let mut budget = TextBudget::default();
    let account = account.expose(|value| AccessibleText::new(value, &mut budget))?;
    let prompt = project_prompt(phase, visual, prompt, &mut budget)?;
    let state = state_for(phase, visual, prompt.as_ref())?;
    let caps_lock = visual.caps_lock_active().then_some(AccessibleStatus {
        id: "lock-caps-lock",
        role: AccessibleRole::Alert,
        text: "Caps Lock is on",
        live_region: LivePoliteness::Assertive,
    });

    let surface_count = outputs.len();
    let surfaces = outputs
        .iter()
        .enumerate()
        .map(|(index, (output, committed))| AccessibleSurface {
            output: *output,
            id: format!("lock-surface-{index}"),
            role: AccessibleRole::Window,
            name: LOCK_SCREEN_NAME,
            frame_committed: *committed,
            position_in_set: index + 1,
            set_size: surface_count,
        })
        .collect::<Vec<_>>();
    let all_frames_committed =
        !surfaces.is_empty() && surfaces.iter().all(|surface| surface.frame_committed);

    Ok(LockAccessibilitySnapshot {
        role: AccessibleRole::Application,
        name: LOCK_SCREEN_NAME,
        phase,
        surfaces,
        dialog: AccessibleDialog {
            id: "lock-authentication",
            role: AccessibleRole::Dialog,
            name: LOCK_SCREEN_NAME,
            account,
            state,
            prompt,
            caps_lock,
            keyboard_focus_available: visual.keyboard_focused(),
        },
        all_frames_committed,
    })
}

fn project_prompt(
    phase: Phase,
    visual: LockVisualState,
    prompt: Option<PromptText<'_>>,
    budget: &mut TextBudget,
) -> Result<Option<AccessiblePrompt>, AccessibilityProjectionError> {
    let Some(prompt) = prompt else {
        if !valid_without_prompt(phase, visual) {
            return Err(AccessibilityProjectionError::InvalidPresentation);
        }
        return Ok(None);
    };
    if phase != Phase::Authenticating || visual.authentication_failed() {
        return Err(AccessibilityProjectionError::InvalidPresentation);
    }

    let kind = prompt.kind();
    let (prompt_kind, role, protected, editable, has_value, selected, available, live_region) =
        match (kind, visual.prompt()) {
            (RequestKind::EchoOff, PromptVisual::Secret { dots }) => (
                PromptKind::Password,
                AccessibleRole::PasswordTextField,
                true,
                true,
                dots != 0,
                None,
                true,
                LivePoliteness::Polite,
            ),
            (RequestKind::EchoOn, PromptVisual::Text { dots }) => (
                PromptKind::Text,
                AccessibleRole::TextField,
                false,
                true,
                dots != 0,
                None,
                true,
                LivePoliteness::Polite,
            ),
            (RequestKind::Info, PromptVisual::Notice) => (
                PromptKind::Information,
                AccessibleRole::Status,
                false,
                false,
                false,
                None,
                true,
                LivePoliteness::Polite,
            ),
            (RequestKind::Error, PromptVisual::Notice) => (
                PromptKind::Error,
                AccessibleRole::Alert,
                false,
                false,
                false,
                None,
                true,
                LivePoliteness::Assertive,
            ),
            (RequestKind::Radio, PromptVisual::Radio { selected }) => (
                PromptKind::Radio,
                AccessibleRole::RadioGroup,
                false,
                false,
                false,
                Some(selected),
                true,
                LivePoliteness::Polite,
            ),
            (RequestKind::Binary, PromptVisual::Binary) => (
                PromptKind::UnsupportedBinary,
                AccessibleRole::Alert,
                false,
                false,
                false,
                None,
                false,
                LivePoliteness::Assertive,
            ),
            _ => return Err(AccessibilityProjectionError::InvalidPresentation),
        };
    let label = PromptLabel::from_prompt(prompt);
    let name = label.expose(|value| AccessibleText::new(value, budget))?;
    let actions = actions_for(prompt_kind);

    Ok(Some(AccessiblePrompt {
        id: "lock-prompt",
        role,
        kind: prompt_kind,
        name,
        protected,
        editable,
        has_value,
        selected,
        focused: visual.keyboard_focused(),
        available,
        live_region,
        actions,
    }))
}

fn valid_without_prompt(phase: Phase, visual: LockVisualState) -> bool {
    match phase {
        Phase::Authenticating => {
            visual.prompt() == PromptVisual::Authenticating && !visual.authentication_failed()
        }
        Phase::Locked => visual.prompt() == PromptVisual::Hidden,
        Phase::Acquiring
        | Phase::UnlockAuthorized
        | Phase::Denied
        | Phase::FailedLocked
        | Phase::Finished => {
            visual.prompt() == PromptVisual::Hidden && !visual.authentication_failed()
        }
    }
}

fn state_for(
    phase: Phase,
    visual: LockVisualState,
    prompt: Option<&AccessiblePrompt>,
) -> Result<AccessibleStatus, AccessibilityProjectionError> {
    let state = match phase {
        Phase::Acquiring => AccessibleStatus {
            id: "lock-securing",
            role: AccessibleRole::Status,
            text: "Securing screen…",
            live_region: LivePoliteness::Polite,
        },
        Phase::Locked if visual.authentication_failed() => AccessibleStatus {
            id: "lock-authentication-failed",
            role: AccessibleRole::Alert,
            text: "Authentication failed. Try again.",
            live_region: LivePoliteness::Assertive,
        },
        Phase::Locked => AccessibleStatus {
            id: "lock-locked",
            role: AccessibleRole::Status,
            text: "Screen locked",
            live_region: LivePoliteness::Polite,
        },
        Phase::Authenticating if prompt.is_none() => AccessibleStatus {
            id: "lock-authenticating",
            role: AccessibleRole::Status,
            text: "Authenticating…",
            live_region: LivePoliteness::Polite,
        },
        Phase::Authenticating => AccessibleStatus {
            id: "lock-authentication-required",
            role: AccessibleRole::Status,
            text: "Authentication required",
            live_region: LivePoliteness::Off,
        },
        Phase::UnlockAuthorized => AccessibleStatus {
            id: "lock-unlocking",
            role: AccessibleRole::Status,
            text: "Unlocking…",
            live_region: LivePoliteness::Polite,
        },
        Phase::Denied => AccessibleStatus {
            id: "lock-denied",
            role: AccessibleRole::Alert,
            text: "The compositor denied the lock request",
            live_region: LivePoliteness::Assertive,
        },
        Phase::FailedLocked => AccessibleStatus {
            id: "lock-failed",
            role: AccessibleRole::Alert,
            text: "The secure lock provider stopped. Use session recovery.",
            live_region: LivePoliteness::Assertive,
        },
        Phase::Finished => AccessibleStatus {
            id: "lock-finished",
            role: AccessibleRole::Status,
            text: "Session unlocked",
            live_region: LivePoliteness::Polite,
        },
    };
    if (phase == Phase::Authenticating) != (prompt.is_some() || state.id == "lock-authenticating") {
        return Err(AccessibilityProjectionError::InvalidPresentation);
    }
    Ok(state)
}

fn actions_for(kind: PromptKind) -> Vec<AccessibleAction> {
    let mut actions = Vec::new();
    if kind == PromptKind::Radio {
        actions.extend([
            action(
                "lock-radio-previous",
                "Previous option",
                LockAction::SelectPrevious,
                true,
            ),
            action(
                "lock-radio-next",
                "Next option",
                LockAction::SelectNext,
                true,
            ),
            action(
                "lock-radio-toggle",
                "Toggle option",
                LockAction::ToggleSelection,
                true,
            ),
        ]);
    }
    if kind != PromptKind::UnsupportedBinary {
        actions.push(action(
            "lock-submit",
            if matches!(kind, PromptKind::Password | PromptKind::Text) {
                "Unlock"
            } else {
                "Continue"
            },
            LockAction::Submit,
            true,
        ));
    }
    actions.push(action(
        "lock-cancel",
        "Cancel authentication",
        LockAction::Cancel,
        true,
    ));
    actions
}

const fn action(
    id: &'static str,
    name: &'static str,
    kind: LockAction,
    enabled: bool,
) -> AccessibleAction {
    AccessibleAction {
        id,
        name,
        kind,
        enabled,
    }
}

#[derive(Default)]
struct TextBudget {
    bytes: usize,
}

impl TextBudget {
    fn add(&mut self, value: &str) -> Result<(), AccessibilityProjectionError> {
        if value.trim().is_empty()
            || value.chars().any(char::is_control)
            || value.len() > MAX_ACCESSIBLE_TEXT_BYTES
        {
            return Err(AccessibilityProjectionError::InvalidText);
        }
        self.bytes = self.bytes.saturating_add(value.len());
        if self.bytes > MAX_ACCESSIBLE_TEXT_BYTES {
            return Err(AccessibilityProjectionError::TextLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::DecodedText;
    use crate::pam_broker::conversation_channel;
    use crate::pam_conversation::{Conversation as _, Request};
    use crate::runtime::{Coordinator, RuntimeEvent};
    use std::thread;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(2);

    fn output(value: u64) -> OutputId {
        OutputId::new(value).unwrap()
    }

    fn acquire(coordinator: &mut Coordinator) {
        for value in [1, 2] {
            coordinator
                .apply(RuntimeEvent::OutputAdded(output(value)))
                .unwrap();
            coordinator
                .apply(RuntimeEvent::FrameCommitted(output(value)))
                .unwrap();
        }
        coordinator.apply(RuntimeEvent::LockAcquired).unwrap();
    }

    #[test]
    fn mirrors_outputs_once_and_redacts_account_output_and_prompt_text() {
        let mut coordinator = Coordinator::new();
        acquire(&mut coordinator);
        coordinator
            .apply(RuntimeEvent::KeyboardFocusAvailable(true))
            .unwrap();
        let snapshot = coordinator
            .accessibility_snapshot("private-account-8472")
            .unwrap();
        assert_eq!(snapshot.role, AccessibleRole::Application);
        assert_eq!(snapshot.surfaces.len(), 2);
        assert!(snapshot.all_frames_committed);
        assert_eq!(snapshot.dialog.state.text, "Authenticating…");
        assert!(snapshot.dialog.prompt.is_none());
        assert!(snapshot.dialog.keyboard_focus_available);
        snapshot
            .dialog
            .account
            .expose(|value| assert_eq!(value, "private-account-8472"));

        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("private-account-8472"));
        assert!(!debug.contains("OutputId"));
        assert!(!debug.contains("8472"));
    }

    #[test]
    fn password_semantics_expose_only_protected_presence_and_exact_safe_actions() {
        let mut coordinator = Coordinator::new();
        acquire(&mut coordinator);
        coordinator
            .apply(RuntimeEvent::KeyboardFocusAvailable(true))
            .unwrap();
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        coordinator.apply(RuntimeEvent::Prompt(pending)).unwrap();
        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Text(
                DecodedText::new("secret-credential-8472".into()).unwrap(),
            )))
            .unwrap();

        let snapshot = coordinator.accessibility_snapshot("jacob").unwrap();
        let prompt = snapshot.dialog.prompt.as_ref().unwrap();
        assert_eq!(prompt.role, AccessibleRole::PasswordTextField);
        assert_eq!(prompt.kind, PromptKind::Password);
        assert!(prompt.protected);
        assert!(prompt.editable);
        assert!(prompt.has_value);
        assert!(prompt.focused);
        prompt.name.expose(|name| assert_eq!(name, "Password:"));
        assert_eq!(
            prompt
                .actions
                .iter()
                .map(|action| action.kind)
                .collect::<Vec<_>>(),
            vec![LockAction::Submit, LockAction::Cancel]
        );
        assert!(matches!(
            prompt.actions[0].kind.into_input(),
            DecodedKey::Submit
        ));
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("Password"));
        assert!(!debug.contains("secret-credential"));
        assert!(!debug.contains("8472"));

        coordinator
            .apply(RuntimeEvent::Input(DecodedKey::Cancel))
            .unwrap();
        assert!(worker.join().unwrap().is_err());
    }

    #[test]
    fn malformed_identity_output_and_prompt_state_fail_closed() {
        let visual = LockVisualState::new(PromptVisual::Hidden, false);
        assert!(matches!(
            project_lock_accessibility("bad\naccount", Phase::Acquiring, &[], visual, None),
            Err(AccessibilityProjectionError::InvalidAccount)
        ));

        let outputs = (1..=u64::try_from(MAX_ACCESSIBLE_SURFACES + 1).unwrap())
            .map(|value| (output(value), true))
            .collect::<Vec<_>>();
        assert!(matches!(
            project_lock_accessibility("jacob", Phase::Acquiring, &outputs, visual, None),
            Err(AccessibilityProjectionError::SurfaceLimit)
        ));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let prompt = pending.prompt();
        prompt.text(|text| {
            assert!(matches!(
                project_lock_accessibility(
                    "jacob",
                    Phase::Authenticating,
                    &[(output(1), true)],
                    LockVisualState::new(PromptVisual::text(3), false),
                    Some(PromptText::new(prompt.id(), prompt.kind(), text)),
                ),
                Err(AccessibilityProjectionError::InvalidPresentation)
            ));
        });
        pending.cancel().unwrap();
        assert!(worker.join().unwrap().is_err());

        let failure = project_lock_accessibility(
            "jacob",
            Phase::Locked,
            &[(output(1), true)],
            LockVisualState::new(PromptVisual::Hidden, true).with_caps_lock(true),
            None,
        )
        .unwrap();
        assert_eq!(failure.dialog.state.role, AccessibleRole::Alert);
        assert_eq!(failure.dialog.state.live_region, LivePoliteness::Assertive);
        assert_eq!(failure.dialog.caps_lock.unwrap().text, "Caps Lock is on");

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::Radio(c"Use key?")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let prompt = pending.prompt();
        let radio = prompt
            .text(|text| {
                project_lock_accessibility(
                    "jacob",
                    Phase::Authenticating,
                    &[(output(1), true)],
                    LockVisualState::new(PromptVisual::Radio { selected: true }, false),
                    Some(PromptText::new(prompt.id(), prompt.kind(), text)),
                )
            })
            .unwrap();
        let radio = radio.dialog.prompt.unwrap();
        assert_eq!(radio.kind, PromptKind::Radio);
        assert_eq!(radio.selected, Some(true));
        assert_eq!(
            radio
                .actions
                .iter()
                .map(|action| action.kind)
                .collect::<Vec<_>>(),
            vec![
                LockAction::SelectPrevious,
                LockAction::SelectNext,
                LockAction::ToggleSelection,
                LockAction::Submit,
                LockAction::Cancel,
            ]
        );
        pending.cancel().unwrap();
        assert!(worker.join().unwrap().is_err());
    }
}
