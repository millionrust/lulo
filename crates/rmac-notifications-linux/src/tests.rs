use super::*;
use rmac_notifications::protocol;
use std::collections::HashMap;
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
fn portal_wire_separates_validated_media_from_reducer_state() {
    let icon = owned((
        "themed".to_owned(),
        owned(vec![
            "mail-unread-symbolic".to_owned(),
            "mail-unread".to_owned(),
        ]),
    ));
    let notification = HashMap::from([
        ("title".into(), string("Message")),
        ("icon".into(), icon),
        ("sound".into(), string("default")),
    ]);
    let decoded =
        portal_with_media("org.example.Chat".into(), "message-8".into(), notification).unwrap();
    assert_eq!(decoded.request.sound, rmac_notifications::Sound::Default);
    assert!(matches!(
        decoded.media.icon,
        Some(media::Icon::Themed(ref names))
            if names.len() == 2
                && names[0] == "mail-unread-symbolic"
                && names[1] == "mail-unread"
    ));
    assert!(decoded.media.sound.is_none());
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
