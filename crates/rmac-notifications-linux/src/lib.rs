//! Linux D-Bus wire decoding for the rmac notification authority.
//!
//! This crate consumes untrusted `a{sv}` values and emits the portable protocol
//! inputs from `rmac-notifications`. It owns no D-Bus name and performs no UI
//! work; those runtime concerns can therefore share one audited decoder.

use std::collections::HashMap;
use std::fmt;

use rmac_notifications::protocol::{
    self, FreedesktopHints, FreedesktopInput, PortalButton, PortalInput, PortalSound,
};
use rmac_notifications::{ActionTarget, Request};
use zbus::zvariant::{serialized::Context, to_bytes, Endian, OwnedValue};

pub mod service;

pub const FREEDESKTOP_CAPABILITIES: &[&str] = &["actions", "body", "persistence"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    WrongType,
    InvalidValue,
    InvalidMarkup,
    InvalidTarget,
    Domain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub field: &'static str,
    pub kind: ErrorKind,
}

impl Error {
    fn new(field: &'static str, kind: ErrorKind) -> Self {
        Self { field, kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid notification field {} ({:?})",
            self.field, self.kind
        )
    }
}

impl std::error::Error for Error {}

pub fn freedesktop(
    authenticated_app_id: String,
    replaces_id: u32,
    summary: String,
    body: String,
    actions: Vec<String>,
    mut hints: HashMap<String, OwnedValue>,
    expire_timeout: i32,
) -> Result<Request, Error> {
    let urgency = take_optional::<u8>(&mut hints, "urgency")?;
    let resident = take_optional::<bool>(&mut hints, "resident")?.unwrap_or(false);
    let transient = take_optional::<bool>(&mut hints, "transient")?.unwrap_or(false);
    let suppress_sound = take_optional::<bool>(&mut hints, "suppress-sound")?.unwrap_or(false);
    let category = take_string(&mut hints, "category")?;
    protocol::freedesktop(FreedesktopInput {
        authenticated_app_id,
        replaces_id,
        summary,
        // rmac does not advertise body-markup, so the body is always rendered
        // as literal plain text and can never become executable markup.
        body_plain: body,
        actions,
        hints: FreedesktopHints {
            urgency,
            resident,
            transient,
            suppress_sound,
            category,
        },
        expire_timeout,
    })
    .map_err(|_| Error::new("request", ErrorKind::Domain))
}

pub fn portal(
    app_id: String,
    id: String,
    mut notification: HashMap<String, OwnedValue>,
) -> Result<Request, Error> {
    let title = take_string(&mut notification, "title")?;
    let body = take_string(&mut notification, "body")?;
    let markup_body_plain = take_string(&mut notification, "markup-body")?
        .map(|markup| sanitize_portal_markup(&markup))
        .transpose()?;
    let priority = take_string(&mut notification, "priority")?;
    let default_action = take_string(&mut notification, "default-action")?;
    let default_action_target = notification
        .remove("default-action-target")
        .map(|value| target("default-action-target", value))
        .transpose()?;
    let buttons = take_buttons(&mut notification)?;
    let display_hints =
        take_optional::<Vec<String>>(&mut notification, "display-hint")?.unwrap_or_default();
    let category = take_string(&mut notification, "category")?;
    let sound = take_portal_sound(&mut notification)?;
    // Icon and validated custom media are presentation resources, not reducer
    // state. The runtime adapter will validate sealed fds before publishing a
    // separate bounded media handle; unknown optional keys are ignored.
    protocol::portal(PortalInput {
        app_id,
        id,
        title,
        body,
        markup_body_plain,
        priority,
        default_action,
        default_action_target,
        buttons,
        display_hints,
        category,
        sound,
    })
    .map_err(|_| Error::new("notification", ErrorKind::Domain))
}

fn take_buttons(
    notification: &mut HashMap<String, OwnedValue>,
) -> Result<Vec<PortalButton>, Error> {
    let Some(value) = notification.remove("buttons") else {
        return Ok(Vec::new());
    };
    let buttons = Vec::<HashMap<String, OwnedValue>>::try_from(value)
        .map_err(|_| Error::new("buttons", ErrorKind::WrongType))?;
    buttons
        .into_iter()
        .map(|mut button| {
            let label = take_string(&mut button, "label")?;
            let action = take_string(&mut button, "action")?
                .ok_or_else(|| Error::new("buttons.action", ErrorKind::InvalidValue))?;
            let target = button
                .remove("target")
                .map(|value| target("buttons.target", value))
                .transpose()?;
            let purpose = take_string(&mut button, "purpose")?;
            Ok(PortalButton {
                label,
                action,
                target,
                purpose,
            })
        })
        .collect()
}

fn take_portal_sound(notification: &mut HashMap<String, OwnedValue>) -> Result<PortalSound, Error> {
    let Some(value) = notification.remove("sound") else {
        return Ok(PortalSound::Unspecified);
    };
    if let Ok(name) = <&str>::try_from(&value) {
        return match name {
            "default" => Ok(PortalSound::Default),
            "silent" => Ok(PortalSound::Silent),
            _ => Err(Error::new("sound", ErrorKind::InvalidValue)),
        };
    }
    // Custom sound fd tuples require the media validator, which is not part of
    // this state-only slice. Ignoring them is permitted by the portal contract.
    Ok(PortalSound::Unspecified)
}

fn target(field: &'static str, value: OwnedValue) -> Result<ActionTarget, Error> {
    let context = Context::new_dbus(Endian::Little, 0);
    let serialized =
        to_bytes(context, &value).map_err(|_| Error::new(field, ErrorKind::InvalidTarget))?;
    #[cfg(unix)]
    if !serialized.fds().is_empty() {
        return Err(Error::new(field, ErrorKind::InvalidTarget));
    }
    ActionTarget::new("v", serialized.bytes().to_vec())
        .map_err(|_| Error::new(field, ErrorKind::InvalidTarget))
}

fn take_string(
    values: &mut HashMap<String, OwnedValue>,
    key: &'static str,
) -> Result<Option<String>, Error> {
    let Some(value) = values.remove(key) else {
        return Ok(None);
    };
    String::try_from(value)
        .map(Some)
        .map_err(|_| Error::new(key, ErrorKind::WrongType))
}

fn take_optional<T>(
    values: &mut HashMap<String, OwnedValue>,
    key: &'static str,
) -> Result<Option<T>, Error>
where
    T: TryFrom<OwnedValue>,
{
    let Some(value) = values.remove(key) else {
        return Ok(None);
    };
    T::try_from(value)
        .map(Some)
        .map_err(|_| Error::new(key, ErrorKind::WrongType))
}

/// Converts the portal's deliberately small markup language to inert text.
/// Only b/i/a tags affect presentation, so all tags are discarded, the five
/// XML named entities are decoded, and line breaks are normalized to spaces.
fn sanitize_portal_markup(markup: &str) -> Result<String, Error> {
    if markup.len() > 64 * 1024 {
        return Err(Error::new("markup-body", ErrorKind::InvalidMarkup));
    }
    let mut output = String::with_capacity(markup.len());
    let mut open_tags = Vec::new();
    let mut cursor = 0;
    while cursor < markup.len() {
        let remainder = &markup[cursor..];
        if remainder.starts_with('<') {
            let end = remainder
                .find('>')
                .ok_or_else(|| Error::new("markup-body", ErrorKind::InvalidMarkup))?;
            let tag = remainder[1..end].trim();
            let closing = tag.starts_with('/');
            let self_closing = tag.ends_with('/');
            let name = tag
                .trim_start_matches('/')
                .split(|character: char| character.is_ascii_whitespace() || character == '/')
                .next()
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '-')
                })
                .ok_or_else(|| Error::new("markup-body", ErrorKind::InvalidMarkup))?;
            if closing {
                if open_tags.pop() != Some(name) {
                    return Err(Error::new("markup-body", ErrorKind::InvalidMarkup));
                }
            } else if !self_closing {
                if open_tags.len() >= 32 {
                    return Err(Error::new("markup-body", ErrorKind::InvalidMarkup));
                }
                open_tags.push(name);
            }
            cursor += end + 1;
            continue;
        }
        if remainder.starts_with('&') {
            let end = remainder
                .find(';')
                .ok_or_else(|| Error::new("markup-body", ErrorKind::InvalidMarkup))?;
            let entity = &remainder[..=end];
            let decoded = match entity {
                "&amp;" => '&',
                "&lt;" => '<',
                "&gt;" => '>',
                "&quot;" => '"',
                "&apos;" => '\'',
                _ => decode_numeric_entity(entity)
                    .ok_or_else(|| Error::new("markup-body", ErrorKind::InvalidMarkup))?,
            };
            output.push(if matches!(decoded, '\n' | '\r') {
                ' '
            } else {
                decoded
            });
            cursor += end + 1;
            continue;
        }
        let character = remainder
            .chars()
            .next()
            .ok_or_else(|| Error::new("markup-body", ErrorKind::InvalidMarkup))?;
        output.push(if matches!(character, '\n' | '\r') {
            ' '
        } else {
            character
        });
        cursor += character.len_utf8();
    }
    if !open_tags.is_empty() {
        return Err(Error::new("markup-body", ErrorKind::InvalidMarkup));
    }
    Ok(output)
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix("&#")?.strip_suffix(';')?;
    let (digits, radix) = digits
        .strip_prefix('x')
        .or_else(|| digits.strip_prefix('X'))
        .map_or((digits, 10), |digits| (digits, 16));
    let character = char::from_u32(u32::from_str_radix(digits, radix).ok()?)?;
    (!character.is_control() || matches!(character, '\n' | '\r' | '\t')).then_some(character)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{DynamicType, OwnedValue, Str, Value};

    fn string(value: &str) -> OwnedValue {
        OwnedValue::from(Str::from(value.to_owned()))
    }

    fn owned<T>(value: T) -> OwnedValue
    where
        T: Into<Value<'static>> + DynamicType,
    {
        OwnedValue::try_from(Value::new(value)).unwrap()
    }

    #[test]
    fn freedesktop_wire_hints_map_and_unknown_values_are_ignored() {
        let hints = HashMap::from([
            ("urgency".into(), owned(2_u8)),
            ("resident".into(), owned(true)),
            ("transient".into(), owned(true)),
            ("suppress-sound".into(), owned(true)),
            ("category".into(), string("transfer.complete")),
            ("x-example-private".into(), string("ignored")),
        ]);
        let request = freedesktop(
            ":1.42".into(),
            0,
            "Done".into(),
            "Private path".into(),
            vec!["default".into(), "Open".into()],
            hints,
            -1,
        )
        .unwrap();
        assert_eq!(request.priority, rmac_notifications::Priority::Urgent);
        assert!(request.display.resident);
        assert!(request.display.transient);
        assert_eq!(request.sound, rmac_notifications::Sound::Silent);
    }

    #[test]
    fn wrong_known_hint_type_fails_without_echoing_payload() {
        let hints = HashMap::from([("urgency".into(), string("secret-value-8472"))]);
        let error = freedesktop(
            ":1.42".into(),
            0,
            "Secret".into(),
            "Secret".into(),
            Vec::new(),
            hints,
            -1,
        )
        .unwrap_err();
        assert_eq!(error, Error::new("urgency", ErrorKind::WrongType));
        assert!(!format!("{error:?}").contains("secret-value-8472"));
    }

    #[test]
    fn portal_wire_decodes_markup_display_hints_and_opaque_targets() {
        let button: HashMap<String, OwnedValue> = HashMap::from([
            ("label".into(), string("Reply")),
            ("action".into(), string("app.reply")),
            ("target".into(), string("conversation-7")),
            ("purpose".into(), string("im.reply-with-text")),
        ]);
        let notification = HashMap::from([
            ("title".into(), string("Message")),
            (
                "markup-body".into(),
                string("Hello <b>Ada</b> &amp; <i>Lin</i> &#33;"),
            ),
            ("priority".into(), string("high")),
            ("buttons".into(), owned(vec![button])),
            (
                "display-hint".into(),
                owned(vec![
                    "hide-content-on-lockscreen".to_owned(),
                    "show-as-new".to_owned(),
                ]),
            ),
            ("sound".into(), string("silent")),
        ]);
        let request = portal("org.example.Chat".into(), "message-7".into(), notification).unwrap();
        assert_eq!(request.content.body(), "Hello Ada & Lin !");
        assert_eq!(request.actions[0].target().unwrap().signature(), "v");
        assert!(!request.actions[0].target().unwrap().bytes().is_empty());
        assert!(request.display.show_as_new);
        assert_eq!(request.sound, rmac_notifications::Sound::Silent);
    }

    #[test]
    fn malformed_portal_markup_and_button_shapes_fail_closed() {
        let malformed_markup = HashMap::from([("markup-body".into(), string("<b>unfinished"))]);
        assert_eq!(
            portal("org.example.App".into(), "one".into(), malformed_markup).unwrap_err(),
            Error::new("markup-body", ErrorKind::InvalidMarkup)
        );

        let button: HashMap<String, OwnedValue> =
            HashMap::from([("label".into(), string("Missing action"))]);
        let malformed_button = HashMap::from([("buttons".into(), owned(vec![button]))]);
        assert_eq!(
            portal("org.example.App".into(), "two".into(), malformed_button).unwrap_err(),
            Error::new("buttons.action", ErrorKind::InvalidValue)
        );
    }

    #[test]
    fn claimed_capabilities_match_implemented_wire_behavior() {
        assert_eq!(FREEDESKTOP_CAPABILITIES, ["actions", "body", "persistence"]);
        assert!(protocol::SUPPORTED_BUTTON_PURPOSES.contains(&"im.reply-with-text"));
        assert!(protocol::SUPPORTED_CATEGORIES.contains(&"os.battery.low"));
    }
}
