use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn provider(id: &str, category: Category, privacy: Privacy) -> ProviderDescriptor {
    ProviderDescriptor {
        id: rmac_shell_settings::ProviderId(id.into()),
        category,
        privacy,
    }
}

fn result(provider: &str, local: &str, category: Category, title: &str) -> SearchResult {
    let primary = match category {
        Category::Applications => Action::LaunchApplication {
            app_id: local.into(),
            spec: rmac_apps::LaunchSpec::OpenPath("/Applications/Test.app".into()),
        },
        Category::Settings => Action::OpenSetting {
            pane_id: local.into(),
        },
        Category::Calculator | Category::Other => Action::CopyText { text: title.into() },
        Category::Files => Action::OpenFile {
            path: format!("/home/alex/{local}").into(),
        },
    };
    SearchResult {
        id: ResultId {
            provider: rmac_shell_settings::ProviderId(provider.into()),
            local: local.into(),
        },
        category,
        title: title.into(),
        subtitle: None,
        icon: None,
        primary,
        alternate: None,
        recency_rank: 0,
    }
}

fn private_files() -> Privacy {
    Privacy {
        private_content: true,
        network: false,
    }
}

#[test]
fn private_and_network_providers_require_explicit_policy() {
    let descriptors = vec![
        provider("apps", Category::Applications, Privacy::default()),
        provider(
            "files",
            Category::Files,
            Privacy {
                private_content: true,
                network: false,
            },
        ),
        provider(
            "web",
            Category::Other,
            Privacy {
                private_content: false,
                network: true,
            },
        ),
    ];
    assert_eq!(
        enabled_providers(&descriptors, &BTreeMap::new())
            .iter()
            .map(|provider| provider.id.0.as_str())
            .collect::<Vec<_>>(),
        ["apps"]
    );
    let policies = BTreeMap::from([
        (
            rmac_shell_settings::ProviderId("files".into()),
            rmac_shell_settings::ProviderPolicy {
                allow_private_content: true,
                ..Default::default()
            },
        ),
        (
            rmac_shell_settings::ProviderId("web".into()),
            rmac_shell_settings::ProviderPolicy {
                allow_network: true,
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(enabled_providers(&descriptors, &policies).len(), 3);
}

#[test]
fn duplicate_descriptors_and_spoofed_results_are_rejected() {
    let apps = provider("apps", Category::Applications, Privacy::default());
    assert_eq!(
        enabled_providers(&[apps.clone(), apps.clone()], &BTreeMap::new()).len(),
        1
    );
    let mut session = Session::default();
    let request = session.begin("terminal", vec![apps]);
    let spoofed = result("files", "terminal", Category::Files, "Terminal");
    assert!(session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![spoofed]),
    ));
    assert!(session.results().is_empty());
    assert_eq!(session.errors().len(), 1);

    let apps = provider("apps", Category::Applications, Privacy::default());
    let request = session.begin("report", vec![apps]);
    let mut smuggled = result("apps", "report", Category::Applications, "Report");
    smuggled.primary = Action::OpenFile {
        path: "/home/alex/private-report.txt".into(),
    };
    assert!(session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![smuggled]),
    ));
    assert!(session.results().is_empty());
    assert_eq!(session.errors().len(), 1);
}

#[test]
fn exact_and_prefix_matches_rank_deterministically_across_categories() {
    let apps = provider("apps", Category::Applications, Privacy::default());
    let files = provider("files", Category::Files, private_files());
    let mut session = Session::default();
    let request = session.begin("term", vec![apps, files]);
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("files".into()),
        Ok(vec![result(
            "files",
            "1",
            Category::Files,
            "old terminal notes",
        )]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![
            result("apps", "terminal", Category::Applications, "Terminal"),
            result("apps", "term", Category::Applications, "Term"),
        ]),
    );
    assert_eq!(
        session
            .results()
            .iter()
            .map(|result| result.result.title.as_str())
            .collect::<Vec<_>>(),
        ["Term", "Terminal", "old terminal notes"]
    );
}

#[test]
fn newer_query_cancels_old_work_and_rejects_stale_batches() {
    let descriptor = provider("apps", Category::Applications, Privacy::default());
    let mut session = Session::default();
    let first = session.begin("term", vec![descriptor.clone()]);
    let second = session.begin("notes", vec![descriptor]);
    assert!(first.cancellation.is_cancelled());
    assert!(!session.apply(
        first.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result(
            "apps",
            "terminal",
            Category::Applications,
            "Terminal"
        )]),
    ));
    assert!(session.apply(
        second.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result(
            "apps",
            "notes",
            Category::Applications,
            "Notes"
        )]),
    ));
    assert_eq!(session.results()[0].result.title, "Notes");
}

#[test]
fn provider_failure_preserves_other_results_and_exposes_error() {
    let mut session = Session::default();
    let request = session.begin(
        "term",
        vec![
            provider("apps", Category::Applications, Privacy::default()),
            provider("files", Category::Files, private_files()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result(
            "apps",
            "terminal",
            Category::Applications,
            "Terminal",
        )]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("files".into()),
        Err(ProviderError {
            detail: "search cancelled by mount loss".into(),
        }),
    );
    assert_eq!(session.results().len(), 1);
    assert_eq!(session.errors().len(), 1);
    assert!(session.pending().is_empty());
}

#[test]
fn selection_wraps_and_survives_later_provider_batches() {
    let mut session = Session::default();
    let request = session.begin(
        "",
        vec![
            provider("apps", Category::Applications, Privacy::default()),
            provider("settings", Category::Settings, Privacy::default()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result(
            "apps",
            "terminal",
            Category::Applications,
            "Terminal",
        )]),
    );
    let selected = session.selected().cloned();
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("settings".into()),
        Ok(vec![result(
            "settings",
            "sound",
            Category::Settings,
            "Sound",
        )]),
    );
    assert_eq!(session.selected(), selected.as_ref());
    session.move_selection(MoveSelection::Previous);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("sound")
    );
    session.move_selection(MoveSelection::Next);
    assert_eq!(session.selected(), selected.as_ref());
}

#[test]
fn empty_query_selection_order_matches_application_grid_then_suggestions() {
    let mut session = Session::default();
    let request = session.begin(
        "",
        vec![
            provider("apps", Category::Applications, Privacy::default()),
            provider("settings", Category::Settings, Privacy::default()),
        ],
    );
    let mut high_recency_setting = result("settings", "sound", Category::Settings, "Sound");
    high_recency_setting.recency_rank = 100;
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("settings".into()),
        Ok(vec![high_recency_setting]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result(
            "apps",
            "terminal",
            Category::Applications,
            "Terminal",
        )]),
    );

    assert_eq!(session.results()[0].result.category, Category::Applications);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("sound")
    );
    session.move_selection(MoveSelection::Next);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("terminal")
    );
    session.move_selection(MoveSelection::Next);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("sound")
    );
}

#[test]
fn pointer_selection_accepts_only_a_visible_result() {
    let mut session = Session::default();
    let request = session.begin(
        "",
        vec![provider("apps", Category::Applications, Privacy::default())],
    );
    let visible = result("apps", "terminal", Category::Applications, "Terminal");
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![visible.clone()]),
    );
    assert!(!session.select(&visible.id));
    assert!(!session.select(&ResultId {
        provider: rmac_shell_settings::ProviderId("apps".into()),
        local: "not-visible".into(),
    }));
    assert_eq!(session.selected(), Some(&visible.id));
}

#[test]
fn category_cap_prevents_one_provider_from_crowding_out_peers() {
    let mut session = Session::with_limits(10, 2);
    let request = session.begin(
        "",
        vec![
            provider("apps", Category::Applications, Privacy::default()),
            provider("settings", Category::Settings, Privacy::default()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok((0..6)
            .map(|index| {
                result(
                    "apps",
                    &format!("app-{index}"),
                    Category::Applications,
                    &format!("App {index}"),
                )
            })
            .collect()),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("settings".into()),
        Ok(vec![result(
            "settings",
            "sound",
            Category::Settings,
            "Sound",
        )]),
    );
    assert_eq!(session.results().len(), 3);
    assert!(session
        .results()
        .iter()
        .any(|result| result.result.category == Category::Settings));
}

#[test]
fn alternate_activation_is_explicit_and_never_falls_back() {
    let mut file = result("files", "report", Category::Files, "Report");
    file.primary = Action::OpenFile {
        path: PathBuf::from("/home/alex/Report.txt"),
    };
    file.alternate = Some(Action::RevealFile {
        path: PathBuf::from("/home/alex/Report.txt"),
    });
    let descriptor = provider("files", Category::Files, private_files());
    let mut session = Session::default();
    let request = session.begin("report", vec![descriptor]);
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("files".into()),
        Ok(vec![file]),
    );
    assert!(matches!(
        session.activation(ActivationMode::Alternate),
        Some(Action::RevealFile { .. })
    ));
}

#[test]
fn escape_closes_overlay_and_cancels_provider_work() {
    let descriptors = [provider("apps", Category::Applications, Privacy::default())];
    let mut launcher = Launcher::default();
    let request = launcher.open(&descriptors, &BTreeMap::new());
    assert!(launcher.is_open());
    assert!(launcher.escape());
    assert!(!launcher.is_open());
    assert!(request.cancellation.is_cancelled());
    assert!(!launcher.escape());
}
