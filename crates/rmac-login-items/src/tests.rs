use super::*;
use crate::model::{MAX_DISPLAY_FIELD_BYTES, MAX_ERROR_BYTES};

const ENTRY: &str =
    "[Desktop Entry]\nType=Application\nName=Demo\nOnlyShowIn=rmac;GNOME;\nExec=demo\n";

#[test]
fn parser_and_session_filter_follow_xdg_keys() {
    let entry = parse_entry(ENTRY).unwrap();
    assert!(applies_to_session(&entry, &["rmac".into()]));
    assert!(!applies_to_session(&entry, &["KDE".into()]));
    assert!(validate_id("org.example.Demo.desktop").is_ok());
    assert!(validate_id("../Demo.desktop").is_err());
}

#[test]
fn no_display_is_parsed_and_defaults_to_false() {
    assert!(!parse_entry(ENTRY).unwrap().no_display);
    let entry =
        parse_entry("[Desktop Entry]\nType=Application\nName=Demo\nExec=demo\nNoDisplay=true\n")
            .unwrap();
    assert!(entry.no_display);
}

#[test]
fn hidden_update_preserves_entry_and_is_idempotent() {
    let hidden = with_hidden(ENTRY, true, true).unwrap();
    assert!(hidden.contains("Exec=demo"));
    assert!(hidden.contains("Hidden=true"));
    assert!(hidden.contains("X-rmac-ManagedHidden=true"));
    let enabled = with_hidden(&hidden, false, false).unwrap();
    assert_eq!(enabled.matches("Hidden=").count(), 1);
    assert!(!enabled.contains("X-rmac-ManagedHidden"));
}

#[test]
fn malformed_entries_are_rejected_without_rewrite() {
    assert_eq!(
        parse_entry("[Desktop Entry]\nName=No type\n")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidEntry
    );
    assert!(
        parse_entry("[Desktop Entry]\nType=Application\nName=Demo\nExec=bad\tcommand\n").is_err()
    );
    assert!(parse_entry(&format!(
        "[Desktop Entry]\nType=Application\nName=Demo\nExec={}\n",
        "x".repeat(MAX_DISPLAY_FIELD_BYTES + 1)
    ))
    .is_err());
}

#[test]
fn errors_are_bounded_and_control_normalized() {
    let error = Error::new(ErrorKind::Mutation, format!("{}\nsecret", "x".repeat(600)));
    assert!(error.to_string().len() <= MAX_ERROR_BYTES);
    assert!(!error.to_string().contains('\n'));
}

#[test]
fn systemd_states_expose_only_safe_persistent_transitions() {
    let enabled = background_service("example.service", "enabled", true, None)
        .unwrap()
        .unwrap();
    assert!(enabled.enabled);
    assert!(enabled.can_toggle);
    assert!(
        background_service("unused.service", "disabled", false, None)
            .unwrap()
            .is_none()
    );
    assert!(
        background_service("custom.service", "disabled", true, None)
            .unwrap()
            .unwrap()
            .can_toggle
    );
    assert!(
        !background_service("rmac-dock.service", "enabled", true, None)
            .unwrap()
            .unwrap()
            .can_toggle
    );
    assert!(
        !background_service("temporary.service", "enabled-runtime", true, None)
            .unwrap()
            .unwrap()
            .can_toggle
    );
    assert!(validate_service_id("../bad.service").is_err());
}

#[test]
fn system_services_are_read_only_without_user_owned_unit_authority() {
    let system = background_service("system-agent.service", "enabled", false, None)
        .unwrap()
        .unwrap();
    assert!(!system.can_toggle);
    let user = background_service("user-agent.service", "enabled", true, None)
        .unwrap()
        .unwrap();
    assert!(user.can_toggle);
}
