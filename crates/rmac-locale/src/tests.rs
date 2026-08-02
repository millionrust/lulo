use super::*;
use crate::model::REGION_KEYS;
use std::cell::RefCell;

struct FakeService {
    snapshot: RefCell<Snapshot>,
    mutation_error: Option<Error>,
}

impl Service for FakeService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(self.snapshot.borrow().clone())
    }

    fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error> {
        if let Some(error) = &self.mutation_error {
            return Err(error.clone());
        }
        let mut next = self.snapshot.borrow().clone();
        next.locale = normalize_assignments(assignments.to_vec())?;
        *self.snapshot.borrow_mut() = next.clone();
        Ok(next)
    }

    fn set_x11_keyboard(&self, keyboard: &X11Keyboard) -> Result<Snapshot, Error> {
        if let Some(error) = &self.mutation_error {
            return Err(error.clone());
        }
        let mut next = self.snapshot.borrow().clone();
        next.x11_layout = keyboard.layout.clone();
        next.x11_model = keyboard.model.clone();
        next.x11_variant = keyboard.variant.clone();
        next.x11_options = keyboard.options.clone();
        *self.snapshot.borrow_mut() = next.clone();
        Ok(next)
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        locale: normalize_assignments(vec![
            "LANG=en_GB.UTF-8".into(),
            "LC_TIME=en_DK.UTF-8".into(),
        ])
        .unwrap(),
        installed_locales: vec!["C".into(), "en_GB.utf8".into(), "fr_FR.utf8".into()],
        installed_x11_layouts: vec!["de".into(), "us".into()],
        ..Snapshot::default()
    }
}

#[test]
fn language_preview_preserves_format_overrides() {
    let preview = snapshot().preview_language("fr_FR.UTF-8").unwrap();
    assert_eq!(preview[0].encoded(), "LANG=fr_FR.UTF-8");
    assert!(preview
        .iter()
        .any(|assignment| assignment.encoded() == "LC_TIME=en_DK.UTF-8"));
}

#[test]
fn region_preview_changes_only_format_categories() {
    let original = snapshot();
    let preview = original.preview_region("fr_FR.UTF-8").unwrap();
    assert!(REGION_KEYS.iter().all(|key| preview
        .iter()
        .any(|assignment| assignment.key == *key && assignment.value == "fr_FR.UTF-8")));
    assert!(preview
        .iter()
        .any(|assignment| assignment.encoded() == "LANG=en_GB.UTF-8"));
}

#[test]
fn canonical_state_elides_redundant_overrides_and_builds_unsets() {
    let current = normalize_assignments(vec![
        "LANG=en_GB.UTF-8".into(),
        "LC_TIME=en_DK.UTF-8".into(),
        "LANGUAGE=en:en_GB".into(),
    ])
    .unwrap();
    let desired = normalize_assignments(vec!["LANG=en_GB.UTF-8".into()]).unwrap();
    assert_eq!(
        complete_locale_request(&current, &desired),
        [
            "LANG=en_GB.UTF-8".to_string(),
            "LC_TIME=".to_string(),
            "LANGUAGE=".to_string(),
        ]
    );
    let redundant = normalize_assignments(vec![
        "LANG=en_GB.UTF-8".into(),
        "LC_TIME=en_GB.UTF-8".into(),
    ])
    .unwrap();
    assert_eq!(redundant.len(), 1);
    assert!(locale_assignments_match(&desired, &redundant));
}

#[test]
fn public_errors_are_bounded_and_control_free() {
    let error = Error::new(ErrorKind::Protocol, "private\n".repeat(200));
    assert!(error.to_string().len() <= MAX_ERROR_BYTES);
    assert!(!error.to_string().chars().any(char::is_control));
}

#[test]
fn installed_validation_accepts_utf8_alias_spelling() {
    let snapshot = snapshot();
    assert!(snapshot.validate_installed("en_GB.UTF-8").is_ok());
    assert_eq!(
        snapshot
            .validate_installed("xx_YY.UTF-8")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidLocale
    );
    assert!(validate_locale_syntax("../../etc/passwd").is_err());
}

#[test]
fn assignment_decoder_rejects_unknown_and_malformed_keys() {
    assert!(normalize_assignments(vec!["LANG=en_US.UTF-8".into()]).is_ok());
    assert!(normalize_assignments(vec!["LC_ALL=en_US.UTF-8".into()]).is_err());
    assert!(normalize_assignments(vec!["PATH=/tmp".into()]).is_err());
}

#[test]
fn installed_inventory_is_sorted_deduplicated_and_bounded() {
    let mut values = (0..=MAX_INSTALLED_LOCALES)
        .map(|index| format!("x_{index}.UTF-8"))
        .collect::<Vec<_>>();
    values.push("C".into());
    values.push("C".into());
    let (values, truncated) = normalize_installed_locales(values);
    assert_eq!(values.len(), MAX_INSTALLED_LOCALES);
    assert!(truncated);
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn service_boundary_returns_authoritative_snapshot_after_mutation() {
    let service = FakeService {
        snapshot: RefCell::new(snapshot()),
        mutation_error: None,
    };
    let assignments = service
        .snapshot()
        .unwrap()
        .preview_language("fr_FR.UTF-8")
        .unwrap()
        .iter()
        .map(Assignment::encoded)
        .collect::<Vec<_>>();
    let changed = service.set_locale(&assignments).unwrap();
    assert_eq!(changed.language(), "fr_FR.UTF-8");
    assert_eq!(service.snapshot().unwrap(), changed);

    let keyboard = changed
        .preview_x11_keyboard("us,de", ",nodeadkeys", "grp:ctrl_space_toggle")
        .unwrap();
    let changed = service.set_x11_keyboard(&keyboard).unwrap();
    assert_eq!(changed.x11_layout, "us,de");
    assert_eq!(changed.x11_variant, ",nodeadkeys");
}

#[test]
fn failed_service_mutation_preserves_last_authoritative_snapshot() {
    let original = snapshot();
    let service = FakeService {
        snapshot: RefCell::new(original.clone()),
        mutation_error: Some(Error::new(ErrorKind::Authorization, "cancelled")),
    };
    assert!(service.set_locale(&["LANG=fr_FR.UTF-8".into()]).is_err());
    assert_eq!(service.snapshot().unwrap(), original);
}

#[test]
fn keyboard_preview_validates_installed_layouts_and_variant_count() {
    let snapshot = snapshot();
    let keyboard = snapshot
        .preview_x11_keyboard("us,de", ",nodeadkeys", "grp:ctrl_space_toggle")
        .unwrap();
    assert_eq!(keyboard.layout, "us,de");
    assert_eq!(keyboard.variant, ",nodeadkeys");
    assert_eq!(
        snapshot
            .preview_x11_keyboard("us,xx", "", "")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidKeyboard
    );
    assert!(snapshot
        .preview_x11_keyboard("us", "basic,nodeadkeys", "")
        .is_err());
    assert!(snapshot.preview_x11_keyboard("us,de", "", "").is_err());
}

#[test]
fn x11_layout_inventory_is_sorted_deduplicated_and_bounded() {
    let mut values = (0..=MAX_INSTALLED_X11_LAYOUTS)
        .map(|index| format!("layout_{index}"))
        .collect::<Vec<_>>();
    values.extend(["us".into(), "us".into(), "../../bad".into()]);
    let (values, truncated) = normalize_installed_x11_layouts(values);
    assert_eq!(values.len(), MAX_INSTALLED_X11_LAYOUTS);
    assert!(truncated);
    assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
}
