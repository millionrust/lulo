//! Ubuntu update and security-coverage presentation.

use super::*;

impl Settings {
    pub(super) fn append_security_coverage(&self, view: Entity<Self>, cards: &mut Vec<Div>) {
        let updates_view = view.clone();
        let coverage_view = view;
        cards.push(section_header("Security Updates"));
        let security_status = if self.updates_loading && self.updates.is_none() {
            "Loading cached PackageKit status…".to_string()
        } else if let Some(snapshot) = &self.updates {
            let count = snapshot.security_count();
            if count == 0 {
                "No cached security updates".to_string()
            } else if count == 1 {
                "1 security update available".to_string()
            } else {
                format!("{count} security updates available")
            }
        } else {
            "PackageKit security status unavailable".to_string()
        };
        cards.push(card(vec![row_base()
            .child(tile("icons/refresh-cw.svg", secondary(), 22.0))
            .child(text_block(
                "Available security updates".into(),
                Some(security_status.into()),
            ))
            .child(
                Button::new("privacy-open-updates", "Open").on_click(move |_, _, cx| {
                    updates_view.update(cx, |settings, cx| {
                        settings.push(SubPage::SoftwareUpdate, cx);
                    });
                }),
            )
            .into_any_element()]));

        cards.push(section_header("Ubuntu Security Coverage"));
        cards.push(card(vec![row_base()
            .child(tile("icons/shield.svg", accent(), 22.0))
            .child(text_block(
                "Installed package security".into(),
                Some("Ubuntu Pro Client · local machine-readable authorities".into()),
            ))
            .child(
                Button::new("privacy-refresh-coverage", "Refresh")
                    .busy(self.security_coverage_loading)
                    .disabled(self.security_coverage_loading)
                    .on_click(move |_, _, cx| {
                        coverage_view.update(cx, |settings, cx| {
                            settings.refresh_security_coverage(cx);
                        });
                    }),
            )
            .into_any_element()]));
        if self.security_coverage_loading && self.security_coverage.is_none() {
            cards.push(note_card(
                "Reading package origins, Ubuntu Pro services, and unattended-upgrades status…",
            ));
        } else if let Some(coverage) = &self.security_coverage {
            if let Some(release) = &coverage.release_support {
                let status = if release.days_remaining > 0 {
                    format!(
                        "Standard support · {} days remaining",
                        release.days_remaining
                    )
                } else if release.days_remaining == 0 {
                    "Standard support ends today".to_string()
                } else {
                    format!(
                        "Standard support ended {} days ago",
                        release.days_remaining.unsigned_abs()
                    )
                };
                cards.push(card(vec![value_row(
                    "icons/shield.svg",
                    if release.supported() {
                        accent()
                    } else {
                        secondary()
                    },
                    format!("Ubuntu {} lifecycle", release.series).into(),
                    status.into(),
                )]));
            }
            if let Some(sources) = &coverage.package_sources {
                cards.push(card(vec![
                    value_row(
                        "icons/info.svg",
                        secondary(),
                        "Installed APT packages".into(),
                        sources.installed.to_string().into(),
                    ),
                    value_row(
                        "icons/shield.svg",
                        accent(),
                        "Ubuntu archive".into(),
                        format!(
                            "{} Main/Restricted · {} Universe/Multiverse",
                            sources.main.saturating_add(sources.restricted),
                            sources.universe.saturating_add(sources.multiverse)
                        )
                        .into(),
                    ),
                    value_row(
                        "icons/shield.svg",
                        accent(),
                        "Ubuntu Pro archives".into(),
                        format!(
                            "{} ESM Infra · {} ESM Apps",
                            sources.esm_infra, sources.esm_apps
                        )
                        .into(),
                    ),
                    value_row(
                        "icons/app-window.svg",
                        secondary(),
                        "Other package origins".into(),
                        format!(
                            "{} third-party · {} unknown",
                            sources.third_party, sources.unknown
                        )
                        .into(),
                    ),
                ]));
            }
            if let Some(pro) = &coverage.pro {
                let contract = if pro.contract_valid {
                    format!(
                        "Valid · {} days remaining",
                        pro.contract_remaining_days.max(0)
                    )
                } else if pro.attached {
                    format!(
                        "Attached but not valid{}",
                        pro.contract_status
                            .as_deref()
                            .map(|status| format!(" · {status}"))
                            .unwrap_or_default()
                    )
                } else {
                    "Not attached".to_string()
                };
                let services = if pro.enabled_services.is_empty() {
                    "No Ubuntu Pro services enabled".to_string()
                } else {
                    format!("Enabled: {}", pro.enabled_services.join(", "))
                };
                cards.push(card(vec![value_row(
                    "icons/shield.svg",
                    if pro.contract_valid {
                        accent()
                    } else {
                        secondary()
                    },
                    "Ubuntu Pro contract".into(),
                    format!("{contract} · {services}").into(),
                )]));
            }
            if let Some(automatic) = &coverage.automatic_updates {
                let status = if automatic.fully_enabled() {
                    format!(
                        "Enabled · every {} day(s)",
                        automatic.upgrade_frequency_days
                    )
                } else {
                    automatic
                        .disabled_reason
                        .clone()
                        .unwrap_or_else(|| "Not fully enabled".into())
                };
                cards.push(card(vec![value_row(
                    "icons/refresh-cw.svg",
                    if automatic.fully_enabled() {
                        accent()
                    } else {
                        secondary()
                    },
                    "Automatic security updates".into(),
                    status.into(),
                )]));
                cards.push(note_card(format!(
                    "Allowed unattended-upgrade origins: {}. APT timer: {} · periodic job: {} · package-list refresh: every {} day(s){}.",
                    if automatic.allowed_origins.is_empty() {
                        "none reported".to_string()
                    } else {
                        automatic.allowed_origins.join(", ")
                    },
                    if automatic.apt_timer_enabled { "enabled" } else { "disabled" },
                    if automatic.periodic_job_enabled { "enabled" } else { "disabled" },
                    automatic.package_list_frequency_days,
                    automatic
                        .last_run
                        .as_deref()
                        .map(|last_run| format!(" · last run {last_run}"))
                        .unwrap_or_default()
                )));
            }
            for issue in &coverage.issues {
                cards.push(note_card(issue.clone()));
            }
            if !coverage.pro_client_available {
                cards.push(note_card(
                    "Ubuntu Pro Client is unavailable or does not provide the required offline API endpoints on this system.",
                ));
            }
        }
    }
}
