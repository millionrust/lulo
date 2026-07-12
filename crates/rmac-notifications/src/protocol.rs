//! Transport-neutral normalization for the two Linux notification protocols.
//!
//! The eventual zbus service only deserializes D-Bus values into these raw
//! types. Keeping normalization here makes protocol behavior testable on every
//! development host and prevents UI code from interpreting untrusted hints.

use super::{
    Action, ActionTarget, AppId, Content, DisplayHints, LockScreenVisibility, Priority, Request,
    Sound, Source, Timeout, ValidationError,
};

pub const SUPPORTED_BUTTON_PURPOSES: &[&str] = &[
    "system.custom-alert",
    "im.reply-with-text",
    "call.accept",
    "call.decline",
    "call.hang-up",
    "call.enable-speakerphone",
    "call.disable-speakerphone",
];

pub const SUPPORTED_CATEGORIES: &[&str] = &[
    "im.received",
    "alarm.ringing",
    "call.incoming",
    "call.ongoing",
    "call.unanswered",
    "weather.warning.extreme",
    "cellbroadcast.danger.presidential",
    "cellbroadcast.danger.extreme",
    "cellbroadcast.danger.severe",
    "cellbroadcast.public-safety",
    "cellbroadcast.amber-alert",
    "cellbroadcast.test",
    "os.battery.low",
    "browser.web-notification",
];

#[derive(Clone, Default, Eq, PartialEq)]
pub struct PortalButton {
    pub label: Option<String>,
    pub action: String,
    pub target: Option<ActionTarget>,
    pub purpose: Option<String>,
}

impl std::fmt::Debug for PortalButton {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PortalButton")
            .field("label", &self.label.as_ref().map(|_| "<redacted>"))
            .field("action", &"<redacted>")
            .field("target", &self.target)
            .field("purpose", &self.purpose.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct PortalInput {
    pub app_id: String,
    pub id: String,
    pub title: Option<String>,
    pub body: Option<String>,
    /// The D-Bus adapter sanitizes supported markup to plain text first.
    pub markup_body_plain: Option<String>,
    pub priority: Option<String>,
    pub default_action: Option<String>,
    pub default_action_target: Option<ActionTarget>,
    pub buttons: Vec<PortalButton>,
    pub display_hints: Vec<String>,
    pub category: Option<String>,
    pub sound: PortalSound,
}

impl std::fmt::Debug for PortalInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PortalInput")
            .field("app_id", &"<redacted>")
            .field("id", &"<redacted>")
            .field("title", &self.title.as_ref().map(|_| "<redacted>"))
            .field("body", &self.body.as_ref().map(|_| "<redacted>"))
            .field(
                "markup_body_plain",
                &self.markup_body_plain.as_ref().map(|_| "<redacted>"),
            )
            .field("priority", &self.priority.as_ref().map(|_| "<redacted>"))
            .field(
                "default_action",
                &self.default_action.as_ref().map(|_| "<redacted>"),
            )
            .field("default_action_target", &self.default_action_target)
            .field("buttons", &self.buttons)
            .field(
                "display_hints",
                &format_args!("<{} redacted values>", self.display_hints.len()),
            )
            .field("category", &self.category.as_ref().map(|_| "<redacted>"))
            .field("sound", &self.sound)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PortalSound {
    #[default]
    Unspecified,
    Default,
    Silent,
    /// Custom portal sounds are validated by the Linux adapter. The first E1
    /// server treats them as policy-owned sound instead of retaining an fd.
    CustomValidated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    Invalid(ValidationError),
    InvalidPriority,
    InvalidDisplayHints,
    InvalidButton,
    InvalidActions,
}

impl From<ValidationError> for ProtocolError {
    fn from(error: ValidationError) -> Self {
        Self::Invalid(error)
    }
}

pub fn portal(input: PortalInput) -> Result<Request, ProtocolError> {
    let app_id = AppId::parse(input.app_id)?;
    let source = Source::portal(app_id, input.id)?;
    let title = input.title.unwrap_or_default();
    let body = input.body.or(input.markup_body_plain).unwrap_or_default();
    let content = Content::new(title, body)?;
    let priority = match input.priority.as_deref().unwrap_or("normal") {
        "low" => Priority::Low,
        "normal" => Priority::Normal,
        "high" => Priority::High,
        "urgent" => Priority::Urgent,
        _ => return Err(ProtocolError::InvalidPriority),
    };
    let default_action = input
        .default_action
        .map(|id| Action::new(id, "", input.default_action_target))
        .transpose()?;
    let actions = input
        .buttons
        .into_iter()
        .map(|button| {
            let purpose = button
                .purpose
                .filter(|purpose| SUPPORTED_BUTTON_PURPOSES.contains(&purpose.as_str()));
            if button.label.as_deref().unwrap_or_default().is_empty() && purpose.is_none() {
                return Err(ProtocolError::InvalidButton);
            }
            let action = Action::new(
                button.action,
                button.label.unwrap_or_default(),
                button.target,
            )?;
            match purpose {
                Some(purpose) => action.with_purpose(purpose).map_err(ProtocolError::from),
                None => Ok(action),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let display = portal_display_hints(input.display_hints)?;
    let sound = match input.sound {
        PortalSound::Unspecified | PortalSound::CustomValidated => Sound::Policy,
        PortalSound::Default => Sound::Default,
        PortalSound::Silent => Sound::Silent,
    };
    let request = Request {
        source,
        content,
        priority,
        timeout: Timeout::Default,
        default_action,
        actions,
        category: input.category,
        sound,
        display,
        replaces: None,
    };
    request.validate()?;
    Ok(request)
}

fn portal_display_hints(hints: Vec<String>) -> Result<DisplayHints, ProtocolError> {
    let mut display = DisplayHints::default();
    for hint in hints {
        match hint.as_str() {
            "transient" => display.transient = true,
            "tray" => display.tray_only = true,
            "persistent" => display.persistent = true,
            "hide-on-lockscreen" => display.lock_screen = LockScreenVisibility::Hide,
            "hide-content-on-lockscreen" if display.lock_screen != LockScreenVisibility::Hide => {
                display.lock_screen = LockScreenVisibility::HideContent;
            }
            "show-as-new" => display.show_as_new = true,
            // Display hints are extensible. Unknown values must not make an
            // otherwise valid notification disappear.
            _ => {}
        }
    }
    if display.transient && display.tray_only {
        return Err(ProtocolError::InvalidDisplayHints);
    }
    Ok(display)
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct FreedesktopHints {
    pub urgency: Option<u8>,
    pub resident: bool,
    pub transient: bool,
    pub suppress_sound: bool,
    pub category: Option<String>,
}

impl std::fmt::Debug for FreedesktopHints {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FreedesktopHints")
            .field("urgency", &self.urgency)
            .field("resident", &self.resident)
            .field("transient", &self.transient)
            .field("suppress_sound", &self.suppress_sound)
            .field("category", &self.category.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct FreedesktopInput {
    /// Identity authenticated from the unique bus sender or trusted desktop
    /// entry by the Linux adapter; never derived from user-visible app_name.
    pub authenticated_app_id: String,
    pub replaces_id: u32,
    pub summary: String,
    pub body_plain: String,
    /// Alternating action key and localized label, exactly as Notify sends it.
    pub actions: Vec<String>,
    pub hints: FreedesktopHints,
    pub expire_timeout: i32,
}

impl std::fmt::Debug for FreedesktopInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FreedesktopInput")
            .field("authenticated_app_id", &"<redacted>")
            .field("replaces_id", &self.replaces_id)
            .field("summary", &"<redacted>")
            .field("body_plain", &"<redacted>")
            .field(
                "actions",
                &format_args!("<{} redacted values>", self.actions.len()),
            )
            .field("hints", &self.hints)
            .field("expire_timeout", &self.expire_timeout)
            .finish()
    }
}

pub fn freedesktop(input: FreedesktopInput) -> Result<Request, ProtocolError> {
    if !input.actions.len().is_multiple_of(2) || input.actions.len() / 2 > 8 {
        return Err(ProtocolError::InvalidActions);
    }
    let source = Source::Freedesktop {
        app_id: AppId::parse(input.authenticated_app_id)?,
    };
    let priority = Priority::from_freedesktop(input.hints.urgency.unwrap_or(1))?;
    let mut default_action = None;
    let mut actions = Vec::with_capacity(input.actions.len() / 2);
    for pair in input.actions.chunks_exact(2) {
        let action = Action::new(pair[0].clone(), pair[1].clone(), None)?;
        if pair[0] == "default" && default_action.is_none() {
            default_action = Some(action);
        } else {
            actions.push(action);
        }
    }
    let request = Request {
        source,
        content: Content::new(input.summary, input.body_plain)?,
        priority,
        timeout: Timeout::from_freedesktop(input.expire_timeout)?,
        default_action,
        actions,
        category: input.hints.category,
        sound: if input.hints.suppress_sound {
            Sound::Silent
        } else {
            Sound::Policy
        },
        display: DisplayHints {
            transient: input.hints.transient,
            resident: input.hints.resident,
            ..DisplayHints::default()
        },
        replaces: super::NotificationId::from_protocol(input.replaces_id),
    };
    request.validate()?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_v2_fields_map_without_losing_action_targets() {
        let request = portal(PortalInput {
            app_id: "org.example.Chat".into(),
            id: "message-7".into(),
            title: Some("New message".into()),
            body: Some("Hello".into()),
            priority: Some("high".into()),
            default_action: Some("app.open".into()),
            buttons: vec![PortalButton {
                label: Some("Reply".into()),
                action: "app.reply".into(),
                target: Some(ActionTarget::new("s", b"conversation-7".to_vec()).unwrap()),
                purpose: Some("im.reply-with-text".into()),
            }],
            display_hints: vec!["hide-content-on-lockscreen".into(), "future-hint".into()],
            category: Some("im.received".into()),
            sound: PortalSound::Default,
            ..PortalInput::default()
        })
        .unwrap();

        assert_eq!(request.priority, Priority::High);
        assert_eq!(request.content.body(), "Hello");
        assert_eq!(request.actions[0].purpose(), Some("im.reply-with-text"));
        assert_eq!(
            request.actions[0].target().unwrap().bytes(),
            b"conversation-7"
        );
        assert_eq!(
            request.display.lock_screen,
            LockScreenVisibility::HideContent
        );
        assert_eq!(request.sound, Sound::Default);
    }

    #[test]
    fn portal_plain_body_wins_and_conflicting_display_modes_fail() {
        let input = PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            body: Some("Plain".into()),
            markup_body_plain: Some("Markup".into()),
            display_hints: vec!["transient".into(), "tray".into()],
            ..PortalInput::default()
        };
        assert_eq!(portal(input), Err(ProtocolError::InvalidDisplayHints));
    }

    #[test]
    fn portal_button_requires_a_label_or_known_purpose_shape() {
        let missing = PortalInput {
            app_id: "org.example.App".into(),
            id: "one".into(),
            buttons: vec![PortalButton {
                action: "app.open".into(),
                ..PortalButton::default()
            }],
            ..PortalInput::default()
        };
        assert_eq!(portal(missing), Err(ProtocolError::InvalidButton));

        let unknown = PortalInput {
            app_id: "org.example.App".into(),
            id: "two".into(),
            buttons: vec![PortalButton {
                action: "app.open".into(),
                purpose: Some("x-example.unknown".into()),
                ..PortalButton::default()
            }],
            ..PortalInput::default()
        };
        assert_eq!(portal(unknown), Err(ProtocolError::InvalidButton));
    }

    #[test]
    fn freedesktop_default_action_hints_and_timeout_map_exactly() {
        let request = freedesktop(FreedesktopInput {
            authenticated_app_id: ":1.42".into(),
            replaces_id: 17,
            summary: "Transfer complete".into(),
            body_plain: "Private path".into(),
            actions: vec![
                "default".into(),
                "Open".into(),
                "dismiss-later".into(),
                "Later".into(),
            ],
            hints: FreedesktopHints {
                urgency: Some(2),
                resident: true,
                suppress_sound: true,
                category: Some("transfer.complete".into()),
                ..FreedesktopHints::default()
            },
            expire_timeout: 0,
        })
        .unwrap();

        assert_eq!(request.priority, Priority::Urgent);
        assert_eq!(request.timeout, Timeout::Never);
        assert_eq!(request.replaces.unwrap().get(), 17);
        assert_eq!(request.default_action.unwrap().id(), "default");
        assert!(request.display.resident);
        assert_eq!(request.sound, Sound::Silent);
    }

    #[test]
    fn freedesktop_rejects_malformed_pairs_and_unknown_urgency() {
        let malformed = FreedesktopInput {
            authenticated_app_id: ":1.42".into(),
            actions: vec!["open".into()],
            expire_timeout: -1,
            ..FreedesktopInput::default()
        };
        assert_eq!(freedesktop(malformed), Err(ProtocolError::InvalidActions));

        let unknown = FreedesktopInput {
            authenticated_app_id: ":1.42".into(),
            hints: FreedesktopHints {
                urgency: Some(9),
                ..FreedesktopHints::default()
            },
            expire_timeout: -1,
            ..FreedesktopInput::default()
        };
        assert!(matches!(
            freedesktop(unknown),
            Err(ProtocolError::Invalid(_))
        ));
    }

    #[test]
    fn raw_protocol_debug_output_never_exposes_payload_strings() {
        let portal_input = PortalInput {
            app_id: "org.private.Secret8472".into(),
            id: "private-id-8472".into(),
            title: Some("private-title-8472".into()),
            body: Some("private-body-8472".into()),
            priority: Some("private-priority-8472".into()),
            buttons: vec![PortalButton {
                label: Some("private-label-8472".into()),
                action: "private-action-8472".into(),
                purpose: Some("private-purpose-8472".into()),
                ..PortalButton::default()
            }],
            display_hints: vec!["private-hint-8472".into()],
            category: Some("private-category-8472".into()),
            ..PortalInput::default()
        };
        let legacy_input = FreedesktopInput {
            authenticated_app_id: ":private-sender-8472".into(),
            summary: "private-summary-8472".into(),
            body_plain: "private-legacy-body-8472".into(),
            actions: vec!["private-action-8472".into(), "private-label-8472".into()],
            hints: FreedesktopHints {
                category: Some("private-category-8472".into()),
                ..FreedesktopHints::default()
            },
            ..FreedesktopInput::default()
        };
        let debug = format!("{portal_input:?} {legacy_input:?}");
        assert!(!debug.contains("private-"));
        assert!(!debug.contains("Secret8472"));
    }
}
