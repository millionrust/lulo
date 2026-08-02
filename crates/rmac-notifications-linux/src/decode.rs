use std::collections::HashMap;

use rmac_notifications::protocol::{
    self, FreedesktopHints, FreedesktopInput, PortalButton, PortalInput, PortalSound,
};
use rmac_notifications::{ActionTarget, Request};
use zbus::zvariant::{serialized::Context, to_bytes, Endian, OwnedValue};

use crate::{media, Error, ErrorKind};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortalDecoded {
    pub request: Request,
    pub media: media::NotificationMedia,
}

pub fn portal(
    app_id: String,
    id: String,
    notification: HashMap<String, OwnedValue>,
) -> Result<Request, Error> {
    portal_with_media(app_id, id, notification).map(|decoded| decoded.request)
}

pub fn portal_with_media(
    app_id: String,
    id: String,
    mut notification: HashMap<String, OwnedValue>,
) -> Result<PortalDecoded, Error> {
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
    let icon = notification
        .remove("icon")
        .map(media::take_icon)
        .transpose()
        .map_err(media_error)?
        .flatten();
    let sound = notification
        .remove("sound")
        .map(media::take_sound)
        .transpose()
        .map_err(media_error)?
        .unwrap_or_default();
    let (sound, custom_sound) = match sound {
        media::SoundValue::Unspecified => (PortalSound::Unspecified, None),
        media::SoundValue::Default => (PortalSound::Default, None),
        media::SoundValue::Silent => (PortalSound::Silent, None),
        media::SoundValue::Custom(sound) => (PortalSound::CustomValidated, Some(sound)),
    };
    let request = protocol::portal(PortalInput {
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
    .map_err(|_| Error::new("notification", ErrorKind::Domain))?;
    Ok(PortalDecoded {
        request,
        media: media::NotificationMedia {
            icon,
            sound: custom_sound,
        },
    })
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

fn media_error(error: media::Error) -> Error {
    Error::new(error.field, ErrorKind::InvalidMedia(error.kind))
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
