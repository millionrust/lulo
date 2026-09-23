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
        Category::Calculator | Category::Clock | Category::Dictionary | Category::Other => {
            Action::CopyText { text: title.into() }
        }
        Category::Files => Action::OpenFile {
            path: format!("/home/alex/{local}").into(),
        },
        Category::SearchIn => Action::SearchFiles {
            query: title.into(),
        },
    };
    SearchResult {
        id: ResultId {
            provider: rmac_shell_settings::ProviderId(provider.into()),
            local: local.into(),
        },
        category,
        application_group: None,
        title: title.into(),
        subtitle: None,
        detail: None,
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
fn browse_mode_selection_stays_inside_its_category() {
    let mut session = Session::default();
    let request = session.begin(
        "",
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
        Ok(vec![
            result("files", "alpha", Category::Files, "Alpha"),
            result("files", "beta", Category::Files, "Beta"),
        ]),
    );

    session.move_selection_in_category(Category::Files, MoveSelection::Next);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("alpha")
    );
    session.move_selection_in_category(Category::Files, MoveSelection::Next);
    assert_eq!(session.selected().map(|id| id.local.as_str()), Some("beta"));
    session.move_selection_in_category(Category::Files, MoveSelection::Next);
    assert_eq!(
        session.selected().map(|id| id.local.as_str()),
        Some("alpha")
    );
    session.move_selection_in_category(Category::Files, MoveSelection::Previous);
    assert_eq!(session.selected().map(|id| id.local.as_str()), Some("beta"));
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

fn id(provider: &str, local: &str) -> ResultId {
    ResultId {
        provider: rmac_shell_settings::ProviderId(provider.into()),
        local: local.into(),
    }
}

fn titles(session: &Session) -> Vec<&str> {
    session
        .results()
        .iter()
        .map(|ranked| ranked.result.title.as_str())
        .collect()
}

/// "12*7": the answer leads, files follow, "Search in Files" closes.
fn answer_session() -> Session {
    let mut session = Session::default();
    let request = session.begin(
        "12*7",
        vec![
            provider("calculator", Category::Calculator, Privacy::default()),
            provider("files", Category::Files, private_files()),
            provider("search-in", Category::SearchIn, Privacy::default()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("search-in".into()),
        Ok(vec![result(
            "search-in",
            "files",
            Category::SearchIn,
            "Search in Files",
        )]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("files".into()),
        Ok(vec![
            result("files", "a", Category::Files, "12*7 notes.txt"),
            result("files", "b", Category::Files, "12*7 old.txt"),
        ]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("calculator".into()),
        Ok(vec![result(
            "calculator",
            "12*7",
            Category::Calculator,
            "84",
        )]),
    );
    session
}

#[test]
fn answers_lead_and_search_in_rows_close_the_list() {
    let session = answer_session();
    assert_eq!(
        titles(&session),
        ["84", "12*7 notes.txt", "12*7 old.txt", "Search in Files"]
    );
    assert_eq!(session.selected(), Some(&id("calculator", "12*7")));
}

#[test]
fn answers_need_no_textual_match_but_ordinary_results_do() {
    let mut session = Session::default();
    let request = session.begin(
        "time in tokyo",
        vec![
            provider("clock", Category::Clock, Privacy::default()),
            provider("apps", Category::Applications, Privacy::default()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("clock".into()),
        Ok(vec![result(
            "clock",
            "Asia/Tokyo",
            Category::Clock,
            "Tokyo, Japan",
        )]),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(vec![result("apps", "maps", Category::Applications, "Maps")]),
    );
    assert_eq!(titles(&session), ["Tokyo, Japan"]);
}

#[test]
fn search_in_rows_survive_the_overall_limit() {
    let mut session = Session::with_limits(3, 12);
    let request = session.begin(
        "report",
        vec![
            provider("files", Category::Files, private_files()),
            provider("search-in", Category::SearchIn, Privacy::default()),
        ],
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("files".into()),
        Ok((0..6)
            .map(|index| {
                result(
                    "files",
                    &index.to_string(),
                    Category::Files,
                    &format!("report {index}"),
                )
            })
            .collect()),
    );
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("search-in".into()),
        Ok(vec![result(
            "search-in",
            "files",
            Category::SearchIn,
            "Search in Files",
        )]),
    );
    assert_eq!(
        titles(&session),
        ["report 0", "report 1", "Search in Files"]
    );
}

#[test]
fn command_arrows_jump_between_sections_without_wrapping() {
    let mut session = answer_session();
    // Sections: [answer] [files a, b] [search in].
    session.move_selection_by_section(MoveSelection::Next);
    assert_eq!(session.selected(), Some(&id("files", "a")));
    session.move_selection_by_section(MoveSelection::Next);
    assert_eq!(session.selected(), Some(&id("search-in", "files")));
    session.move_selection_by_section(MoveSelection::Next);
    assert_eq!(session.selected(), Some(&id("search-in", "files")));
    session.move_selection(MoveSelection::Previous);
    assert_eq!(session.selected(), Some(&id("files", "b")));
    // Up goes to the start of the current section first, then the previous.
    session.move_selection_by_section(MoveSelection::Previous);
    assert_eq!(session.selected(), Some(&id("files", "a")));
    session.move_selection_by_section(MoveSelection::Previous);
    assert_eq!(session.selected(), Some(&id("calculator", "12*7")));
    session.move_selection_by_section(MoveSelection::Previous);
    assert_eq!(session.selected(), Some(&id("calculator", "12*7")));
}

#[test]
fn the_top_hit_is_a_section_of_its_own() {
    use Category::*;
    assert_eq!(section_starts(&[]), Vec::<usize>::new());
    assert_eq!(section_starts(&[Files]), [0]);
    assert_eq!(section_starts(&[Files, Files, Files, Settings]), [0, 1, 3]);
    assert_eq!(
        section_starts(&[Applications, Files, Files, SearchIn]),
        [0, 1, 3]
    );
}

#[test]
fn a_learned_choice_moves_a_result_up() {
    let apps = || provider("apps", Category::Applications, Privacy::default());
    let batch = || {
        vec![
            result("apps", "terminal", Category::Applications, "Terminal"),
            result("apps", "te", Category::Applications, "Te"),
        ]
    };
    let mut session = Session::default();
    let request = session.begin("te", vec![apps()]);
    session.apply(
        request.generation,
        rmac_shell_settings::ProviderId("apps".into()),
        Ok(batch()),
    );
    assert_eq!(titles(&session), ["Te", "Terminal"]);

    let mut learning = Learning::default();
    for _ in 0..8 {
        learning.record("term", &id("apps", "terminal"), 1_000);
    }
    session.set_learning(std::sync::Arc::new(learning), 1_000);
    assert_eq!(titles(&session), ["Terminal", "Te"]);
}

#[test]
fn learning_relates_longer_and_shorter_queries_and_fades() {
    let terminal = id("apps", "terminal");
    let mut learning = Learning::default();
    learning.record("Term", &terminal, 0);
    let exact = learning.boost("term", &terminal, 0);
    let shorter = learning.boost("te", &terminal, 0);
    let longer = learning.boost("terminal", &terminal, 0);
    assert!(exact > shorter && shorter > longer && longer > 0);
    assert_eq!(learning.boost("notes", &terminal, 0), 0);
    assert_eq!(learning.boost("", &terminal, 0), 0);
    assert_eq!(learning.boost("term", &id("apps", "other"), 0), 0);
    // Two weeks halve it.
    let later = learning.boost("term", &terminal, 14 * 86_400);
    assert!((i32::from(later) - i32::from(exact) / 2).abs() <= 1);
    // Frequent choices weigh more, up to the cap.
    for _ in 0..20 {
        learning.record("term", &terminal, 0);
    }
    assert_eq!(learning.boost("term", &terminal, 0), MAX_BOOST);
}

#[test]
fn learning_forgets_old_choices_and_keeps_the_newest_when_full() {
    let mut learning = Learning::default();
    learning.record("old", &id("apps", "old"), 0);
    learning.record("new", &id("apps", "new"), 91 * 86_400);
    assert_eq!(learning.choices().len(), 1);
    assert_eq!(learning.choices()[0].query, "new");

    let mut full = Learning::default();
    for index in 0..(MAX_CHOICES as u64 + 10) {
        full.record(&format!("q{index}"), &id("apps", "x"), index);
    }
    assert_eq!(full.choices().len(), MAX_CHOICES);
    assert!(full.choices().iter().all(|choice| choice.last_used >= 10));

    full.forget(&id("apps", "x"));
    assert!(full.is_empty());
}

#[test]
fn learning_round_trips_through_text_and_skips_damage() {
    let mut learning = Learning::default();
    learning.record("tab\there", &id("files", "/home/alex/a\\b\nc.txt"), 42);
    learning.record("term", &id("apps", "terminal"), 43);
    learning.record("term", &id("apps", "terminal"), 44);
    let text = learning.to_text();
    assert_eq!(Learning::from_text(&text), learning);

    let damaged = format!("{text}garbage line\n1\t2\n0\t5\tapps\tx\tq\n");
    assert_eq!(Learning::from_text(&damaged), learning);
    assert!(Learning::from_text("something else\n1\t2\tapps\tx\tq\n").is_empty());
    // Blank queries and results never become choices.
    let mut ignored = Learning::default();
    ignored.record("   ", &id("apps", "terminal"), 1);
    ignored.record("term", &id("apps", " "), 1);
    assert!(ignored.is_empty());
}
