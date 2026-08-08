//! Privacy & Security settings presentation.

use super::*;

impl Settings {
    pub(in crate::controller) fn render_privacy_security(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let updates_view = view.clone();
        let coverage_view = view.clone();
        let mut cards = vec![card(vec![row_base()
            .child(tile("icons/shield.svg", accent(), 22.0))
            .child(text_block(
                "Portal permission decisions".into(),
                Some("Camera and microphone decisions stored by XDG portals".into()),
            ))
            .child(
                Button::new("privacy-refresh", "Refresh")
                    .busy(self.privacy_loading || self.privacy_stream_refreshing)
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_privacy(cx));
                    }),
            )
            .into_any_element()])];

        if self.privacy_loading {
            cards.push(note_card(
                "Loading decisions from the portal PermissionStore…",
            ));
        } else if let Some(snapshot) = &self.privacy {
            if snapshot.available {
                cards.push(card(vec![value_row(
                    "icons/info.svg",
                    secondary(),
                    "PermissionStore interface".into(),
                    format!("Version {}", snapshot.version).into(),
                )]));
                for resource in [
                    rmac_privacy::PortalResource::Camera,
                    rmac_privacy::PortalResource::Microphone,
                ] {
                    cards.push(section_header(resource.label()));
                    let decisions = snapshot
                        .decisions
                        .iter()
                        .filter(|decision| decision.resource == resource)
                        .cloned()
                        .collect::<Vec<_>>();
                    if decisions.is_empty() {
                        cards.push(note_card(format!(
                            "No stored {} decisions. This does not prove that native or already-running applications lack access.",
                            resource.label().to_lowercase()
                        )));
                        continue;
                    }
                    let rows = decisions
                        .into_iter()
                        .map(|decision| {
                            let identity = self.application_identity(&decision.app_id);
                            let display_name = identity
                                .map(|application| application.name.as_str())
                                .unwrap_or(&decision.app_id)
                                .to_owned();
                            let detail = format!(
                                "{} · Stored tokens: {}",
                                decision.app_id,
                                decision.summary()
                            );
                            let reset_view = view.clone();
                            let reset_decision = decision.clone();
                            let busy = self.privacy_busy.as_ref().is_some_and(
                                |(busy_resource, busy_app)| {
                                    *busy_resource == decision.resource
                                        && busy_app == &decision.app_id
                                },
                            );
                            row_base()
                                .child(tile("icons/app-window.svg", secondary(), 22.0))
                                .child(text_block(display_name.into(), Some(detail.into())))
                                .child(
                                    Button::new(
                                        SharedString::from(format!(
                                            "privacy-reset-{}-{}",
                                            decision.resource.id(),
                                            decision.app_id
                                        )),
                                        "Reset",
                                    )
                                    .busy(busy)
                                    .disabled(
                                        !snapshot.can_reset
                                            || self.privacy_busy.is_some()
                                            || self.privacy_stream_refreshing,
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            reset_view.update(cx, |settings, cx| {
                                                settings.request_privacy_reset(
                                                    reset_decision.clone(),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                )
                                .into_any_element()
                        })
                        .collect();
                    cards.push(card(rows));
                }
                if let Some(detail) = &snapshot.detail {
                    cards.push(note_card(detail.clone()));
                }
            } else {
                cards.push(note_card(snapshot.detail.clone().unwrap_or_else(|| {
                    "The portal PermissionStore is unavailable in this session.".into()
                })));
            }
        }

        if let Some(decision) = &self.privacy_reset_confirmation {
            let cancel_view = view.clone();
            let confirm_view = view.clone();
            cards.push(note_card(format!(
                "Reset the stored {} decision for {}? The next portal request may ask again. This does not terminate active access or change permissions for native applications.",
                decision.resource.label().to_lowercase(),
                decision.app_id
            )));
            cards.push(card(vec![row_base()
                .child(div().flex_1())
                .child(
                    Button::new("privacy-reset-cancel", "Cancel").on_click(move |_, _, cx| {
                        cancel_view.update(cx, |settings, cx| settings.cancel_privacy_reset(cx));
                    }),
                )
                .child(
                    rmac_ui::dialog_button(
                        "privacy-reset-confirm",
                        "Reset Decision",
                        rmac_ui::DialogButtonKind::Destructive,
                    )
                    .disabled(
                        self.privacy_loading
                            || self.privacy_busy.is_some()
                            || self.privacy_stream_refreshing,
                    )
                    .on_click(move |_, _, cx| {
                        confirm_view.update(cx, |settings, cx| settings.confirm_privacy_reset(cx));
                    }),
                )
                .into_any_element()]));
        }

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
        cards.push(section_header("Desktop Application Sources"));
        let application_sources = rmac_apps::source_inventory(&self.app_catalog);
        cards.push(card(vec![
            value_row(
                "icons/app-window.svg",
                accent(),
                "Desktop-visible applications".into(),
                application_sources.total().to_string().into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Sandbox package exports".into(),
                format!(
                    "{} Flatpak · {} Snap",
                    application_sources.flatpak, application_sources.snap
                )
                .into(),
            ),
            value_row(
                "icons/app-window.svg",
                secondary(),
                "Portable applications".into(),
                format!("{} AppImage", application_sources.appimage).into(),
            ),
            value_row(
                "icons/info.svg",
                secondary(),
                "Unattributed desktop entries".into(),
                format!(
                    "{} system · {} user · {} other",
                    application_sources.system_desktop_entries,
                    application_sources.user_desktop_entries,
                    application_sources.other_desktop_entries
                )
                .into(),
            ),
        ]));
        cards.push(note_card(
            "Application source counts cover the live desktop-entry catalog. Flatpak and Snap use their exported desktop-entry paths; AppImage uses integration IDs or the launch executable. System and user desktop entries are not claimed to be APT-owned, and command-line-only packages are outside this inventory.",
        ));
        cards.push(note_card(
            "Reset revalidates the selected version-2 application/resource tokens immediately before DeletePermission and proves absence afterward. PermissionStore has no atomic compare-and-delete operation, so a change after that preflight cannot be excluded. Tokens remain uninterpreted because the store does not define their meaning.",
        ));
        cards.push(note_card(
            "Ubuntu coverage and automatic-update values are read-only until a polkit-aware, rollback-safe APT policy editor is reviewed. Package and application source counts describe provenance signals, not repository trust, vulnerability status, coverage, or the security of an individual application.",
        ));
        self.pane(cards)
    }
}
