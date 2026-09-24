use rmac_locale::{Error, ErrorKind, Service, Snapshot};

use crate::system::{
    system_hour_cycle, system_set_locale, system_set_x11_keyboard, system_snapshot,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemService;

impl Service for SystemService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        system_snapshot()
    }

    fn set_locale(&self, assignments: &[String]) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let desired = rmac_locale::normalize_assignments(assignments.to_vec())?;
        for assignment in &desired {
            let unchanged = current.locale.iter().any(|candidate| {
                candidate.key == assignment.key && candidate.value == assignment.value
            });
            if assignment.key != "LANGUAGE" && !unchanged {
                current.validate_installed(&assignment.value)?;
            }
        }
        apply_complete_locale(current, &desired)
    }

    fn set_x11_keyboard(&self, keyboard: &rmac_locale::X11Keyboard) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        let validated =
            current.preview_x11_keyboard(&keyboard.layout, &keyboard.variant, &keyboard.options)?;
        if keyboard.model != validated.model {
            return Err(Error::new(
                ErrorKind::InvalidKeyboard,
                "this control preserves the current XKB model",
            ));
        }
        if current.x11_keyboard() == validated {
            return Ok(current);
        }
        system_set_x11_keyboard(&validated)?;
        let after = self.snapshot()?;
        if after.x11_keyboard() != validated {
            return Err(Error::new(
                ErrorKind::Mismatch,
                "localed did not confirm the requested keyboard layout",
            ));
        }
        Ok(after)
    }
}

pub fn snapshot() -> Result<Snapshot, Error> {
    SystemService.snapshot()
}

/// Return the system locale's authoritative hour cycle without enumerating
/// installed locales or keyboard layouts.
pub fn hour_cycle() -> Result<rmac_locale::HourCycle, Error> {
    system_hour_cycle()
}

pub fn set_locale(assignments: &[String]) -> Result<Snapshot, Error> {
    SystemService.set_locale(assignments)
}

pub fn set_x11_keyboard(keyboard: &rmac_locale::X11Keyboard) -> Result<Snapshot, Error> {
    SystemService.set_x11_keyboard(keyboard)
}

pub fn restore_locale(rollback: &rmac_locale::LocaleRollback) -> Result<Snapshot, Error> {
    let current = SystemService.snapshot()?;
    if !rmac_locale::locale_assignments_match(&current.locale, rollback.expected()) {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the system locale changed after Lulo OS applied it; refresh before reverting",
        ));
    }
    apply_complete_locale(current, rollback.previous())
}

pub fn restore_x11_keyboard(rollback: &rmac_locale::KeyboardRollback) -> Result<Snapshot, Error> {
    let current = SystemService.snapshot()?;
    if current.x11_keyboard() != *rollback.expected() {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the keyboard layout changed after Lulo OS applied it; refresh before reverting",
        ));
    }
    let previous = rollback.previous();
    let validated =
        current.preview_x11_keyboard(&previous.layout, &previous.variant, &previous.options)?;
    if validated.model != previous.model {
        return Err(Error::new(
            ErrorKind::Conflict,
            "the authoritative keyboard model changed; the previous layout was not restored",
        ));
    }
    system_set_x11_keyboard(previous)?;
    let after = SystemService.snapshot()?;
    if after.x11_keyboard() != *previous {
        return Err(Error::new(
            ErrorKind::Mismatch,
            "localed did not confirm the previous keyboard layout",
        ));
    }
    Ok(after)
}

fn apply_complete_locale(
    before: Snapshot,
    desired: &[rmac_locale::Assignment],
) -> Result<Snapshot, Error> {
    if rmac_locale::locale_assignments_match(&before.locale, desired) {
        return Ok(before);
    }
    let request = rmac_locale::complete_locale_request(&before.locale, desired);
    system_set_locale(&request)?;
    let mut after = SystemService.snapshot()?;

    // When LANG changes, localed may synthesize LANGUAGE from its fallback
    // table. If that is the sole difference from the desired complete state,
    // confirm the intermediate snapshot is still current, then remove only
    // LANGUAGE in a second request without LANG so fallback is not re-triggered.
    let unexpected = after
        .locale
        .iter()
        .filter(|assignment| {
            !desired
                .iter()
                .any(|candidate| candidate.key == assignment.key)
        })
        .collect::<Vec<_>>();
    let requested_values_match = desired.iter().all(|assignment| {
        after
            .locale
            .iter()
            .any(|candidate| candidate.key == assignment.key && candidate.value == assignment.value)
    });
    if requested_values_match && unexpected.len() == 1 && unexpected[0].key == "LANGUAGE" {
        let confirmed = SystemService.snapshot()?;
        if !rmac_locale::locale_assignments_match(&confirmed.locale, &after.locale) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "the system locale changed while localed was applying the request",
            ));
        }
        system_set_locale(&["LANGUAGE=".into()])?;
        after = SystemService.snapshot()?;
    }
    if !rmac_locale::locale_assignments_match(&after.locale, desired) {
        return Err(Error::new(
            ErrorKind::Mismatch,
            "localed did not confirm the requested language and region state",
        ));
    }
    Ok(after)
}
