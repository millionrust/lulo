//! The Security section: update status plus Ubuntu's automatic updates, Pro
//! contract and release support as read-only facts.

use super::*;

impl Settings {
    pub(super) fn append_security_coverage(&self, view: Entity<Self>, cards: &mut Vec<Div>) {
        cards.push(section_header("Security"));
        let security_status = if self.updates_loading && self.updates.is_none() {
            "Checking…".to_string()
        } else if let Some(snapshot) = &self.updates {
            match snapshot.security_count() {
                0 => "None available".to_string(),
                1 => "1 available".to_string(),
                count => format!("{count} available"),
            }
        } else {
            "Unavailable".to_string()
        };
        let mut rows = vec![icon_nav_row(
            "privacy-security-updates",
            tile("icons/refresh-cw.svg", hsl(0x8e8e93), style::ROW_ICON).into_any_element(),
            "Security Updates",
            Some(security_status.into()),
            move |_, cx| {
                view.update(cx, |settings, cx| {
                    settings.push(SubPage::SoftwareUpdate, cx)
                });
            },
        )];
        let mut notes: Vec<String> = Vec::new();
        if let Some(coverage) = &self.security_coverage {
            if let Some(automatic) = &coverage.automatic_updates {
                rows.push(fact_row(
                    "Automatic security updates",
                    if automatic.fully_enabled() {
                        format!("Every {} day(s)", automatic.upgrade_frequency_days)
                    } else {
                        "Off".to_string()
                    },
                ));
                if !automatic.fully_enabled() {
                    if let Some(reason) = &automatic.disabled_reason {
                        notes.push(reason.clone());
                    }
                }
            }
            if let Some(pro) = &coverage.pro {
                rows.push(fact_row(
                    "Ubuntu Pro",
                    if pro.contract_valid {
                        "Attached"
                    } else if pro.attached {
                        "Not valid"
                    } else {
                        "Not attached"
                    },
                ));
            }
            if let Some(release) = &coverage.release_support {
                rows.push(fact_row(
                    format!("Ubuntu {} support", release.series),
                    if release.days_remaining > 0 {
                        format!("{} days remaining", release.days_remaining)
                    } else if release.days_remaining == 0 {
                        "Ends today".to_string()
                    } else {
                        "Ended".to_string()
                    },
                ));
            }
            notes.extend(coverage.issues.iter().map(|issue| issue.to_string()));
        }
        cards.push(card(rows));
        if self.security_coverage_loading && self.security_coverage.is_none() {
            cards.push(footnote("Reading security coverage…"));
        }
        for note in notes {
            cards.push(footnote(note));
        }
    }
}
