//! Focused launcher runtime contracts.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use rmac_launcher::{Action, Cancellation, Category, Privacy, ProviderError, SearchResult};

fn provider_id(value: &str) -> rmac_shell_settings::ProviderId {
    rmac_shell_settings::ProviderId(value.into())
}

#[derive(Clone)]
struct FakeProvider {
    descriptor: ProviderDescriptor,
    title: &'static str,
    calls: Arc<AtomicUsize>,
}

impl Provider for FakeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn search(
        &self,
        _: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if cancellation.is_cancelled() {
            return Err(ProviderError {
                detail: "cancelled".into(),
            });
        }
        Ok(vec![result(
            &self.descriptor.id.0,
            self.descriptor.category,
            self.title,
        )])
    }
}

struct ChangingProvider {
    descriptor_calls: AtomicUsize,
}

impl Provider for ChangingProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        let id = if self.descriptor_calls.fetch_add(1, Ordering::AcqRel) == 0 {
            "stable"
        } else {
            "changed"
        };
        descriptor(id, Category::Settings, false)
    }

    fn search(&self, _: &str, _: &Cancellation) -> Result<Vec<SearchResult>, ProviderError> {
        panic!("a provider with changed identity must not receive a query")
    }
}

fn descriptor(id: &str, category: Category, private: bool) -> ProviderDescriptor {
    ProviderDescriptor {
        id: provider_id(id),
        category,
        privacy: Privacy {
            private_content: private,
            network: false,
        },
    }
}

fn result(id: &str, category: Category, title: &str) -> SearchResult {
    let primary = match category {
        Category::Applications => Action::LaunchApplication {
            app_id: title.to_lowercase(),
            spec: rmac_apps::LaunchSpec::OpenPath("/Applications/Test.app".into()),
        },
        Category::Settings => Action::OpenSetting {
            pane_id: title.to_lowercase(),
        },
        Category::Files => Action::OpenFile {
            path: "/home/alex/report.txt".into(),
        },
        Category::Calculator | Category::Clock | Category::Dictionary | Category::Other => {
            Action::CopyText { text: title.into() }
        }
        Category::SearchIn => Action::SearchFiles {
            query: title.into(),
        },
    };
    SearchResult {
        id: ResultId {
            provider: provider_id(id),
            local: title.to_lowercase(),
        },
        category,
        application_group: None,
        title: title.into(),
        subtitle: None,
        detail: None,
        icon: None,
        primary,
        alternate: (category == Category::Files)
            .then(|| Action::RevealFile {
                path: "/home/alex/report.txt".into(),
            })
            .or_else(|| {
                (category == Category::Applications).then(|| Action::RevealApplication {
                    source: "/Applications/Test.app".into(),
                })
            }),
        recency_rank: 0,
    }
}

fn batch(request: &Request, id: &str, results: Result<Vec<SearchResult>, ProviderError>) -> Batch {
    Batch {
        generation: request.generation,
        provider: provider_id(id),
        results,
    }
}

fn allow_private(
    id: &str,
) -> BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy> {
    BTreeMap::from([(
        provider_id(id),
        rmac_shell_settings::ProviderPolicy {
            enabled: true,
            allow_private_content: true,
            allow_network: false,
        },
    )])
}

#[test]
fn registry_rejects_duplicate_identity_and_dispatches_only_admitted_providers() {
    let public_calls = Arc::new(AtomicUsize::new(0));
    let private_calls = Arc::new(AtomicUsize::new(0));
    let public: Arc<dyn Provider> = Arc::new(FakeProvider {
        descriptor: descriptor("apps", Category::Applications, false),
        title: "Terminal",
        calls: public_calls.clone(),
    });
    let duplicate: Arc<dyn Provider> = Arc::new(FakeProvider {
        descriptor: descriptor("apps", Category::Settings, false),
        title: "Settings",
        calls: Arc::new(AtomicUsize::new(0)),
    });
    assert!(Registry::new(vec![public.clone(), duplicate]).is_err());

    let private: Arc<dyn Provider> = Arc::new(FakeProvider {
        descriptor: descriptor("files", Category::Files, true),
        title: "Report.txt",
        calls: private_calls.clone(),
    });
    let registry = Registry::new(vec![public, private]).expect("registry is valid");
    let mut coordinator = Coordinator::new(registry.descriptors(), BTreeMap::new());
    let request = coordinator.open().request;
    let (sender, receiver) = async_channel::unbounded();
    futures_lite::future::block_on(registry.dispatch(request, sender));
    let batches: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(public_calls.load(Ordering::Acquire), 1);
    assert_eq!(private_calls.load(Ordering::Acquire), 0);

    let closed_request = coordinator
        .set_query("terminal")
        .expect("overlay remains open");
    let cancellation = closed_request.cancellation.clone();
    let (sender, receiver) = async_channel::bounded(1);
    drop(receiver);
    futures_lite::future::block_on(registry.dispatch(closed_request, sender));
    assert!(cancellation.is_cancelled());
    assert_eq!(public_calls.load(Ordering::Acquire), 1);
}

#[test]
fn changed_provider_identity_finishes_as_an_error_instead_of_loading_forever() {
    let registry = Registry::new(vec![Arc::new(ChangingProvider {
        descriptor_calls: AtomicUsize::new(0),
    })])
    .expect("initial descriptor is valid");
    let mut coordinator = Coordinator::new(registry.descriptors(), BTreeMap::new());
    let request = coordinator.open().request;
    let (sender, receiver) = async_channel::bounded(1);
    futures_lite::future::block_on(registry.dispatch(request, sender));
    let batch = receiver
        .try_recv()
        .expect("descriptor failure is published");
    assert!(coordinator.apply(batch));
    assert_eq!(coordinator.snapshot().phase, Phase::Unavailable);
}

#[test]
fn open_focuses_before_search_and_new_queries_reject_stale_batches() {
    let descriptors = vec![descriptor("apps", Category::Applications, false)];
    let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
    let opened = coordinator.open();
    assert_eq!(opened.focus, FocusTarget::Query);
    assert_eq!(coordinator.snapshot().phase, Phase::Loading);
    assert_eq!(
        coordinator.handle_key(KeyCommand::ArrowDown),
        KeyEffect::None
    );
    let newer = coordinator.set_query("term").expect("overlay is open");
    assert!(!coordinator.apply(batch(
        &opened.request,
        "apps",
        Ok(vec![result("apps", Category::Applications, "Old")]),
    )));
    assert!(coordinator.apply(batch(
        &newer,
        "apps",
        Ok(vec![result("apps", Category::Applications, "Terminal")]),
    )));
    assert_eq!(coordinator.snapshot().rows[0].title, "Terminal");
}

#[test]
fn spotlight_entry_points_toggle_once_and_ignore_replays_and_other_ids() {
    let mut coordinator = Coordinator::new(
        vec![descriptor("apps", Category::Applications, false)],
        BTreeMap::new(),
    );
    let other = rmac_shortcuts::Event::Activated {
        id: rmac_shortcuts::ShortcutId("notification-center".into()),
        timestamp_ms: 9,
    };
    assert!(matches!(
        coordinator.handle_shortcut(&other),
        ShortcutEffect::None
    ));

    let open = rmac_shortcuts::Event::Activated {
        id: rmac_shortcuts::ShortcutId("launcher".into()),
        timestamp_ms: 10,
    };
    let ShortcutEffect::Open(effect) = coordinator.handle_shortcut(&open) else {
        panic!("fresh launcher shortcut opens");
    };
    assert_eq!(effect.focus, FocusTarget::Query);
    assert!(coordinator.snapshot().open);
    assert!(matches!(
        coordinator.handle_shortcut(&open),
        ShortcutEffect::None
    ));
    assert!(coordinator.snapshot().open);

    let close = rmac_shortcuts::Event::Activated {
        id: rmac_shortcuts::ShortcutId("launcher".into()),
        timestamp_ms: 11,
    };
    assert!(matches!(
        coordinator.handle_shortcut(&close),
        ShortcutEffect::Dismissed
    ));
    assert_eq!(coordinator.snapshot().phase, Phase::Closed);
    assert!(matches!(
        coordinator.handle_shortcut(&rmac_shortcuts::Event::Deactivated {
            id: rmac_shortcuts::ShortcutId("launcher".into()),
            timestamp_ms: 12,
        }),
        ShortcutEffect::None
    ));

    let apps = rmac_shortcuts::Event::Activated {
        id: rmac_shortcuts::ShortcutId("app-drawer".into()),
        timestamp_ms: 13,
    };
    assert!(matches!(
        coordinator.handle_shortcut(&apps),
        ShortcutEffect::Open(_)
    ));
    assert!(coordinator.snapshot().open);
}

#[test]
fn keyboard_journey_announces_category_and_uses_exact_alternate_action() {
    let descriptors = vec![descriptor("files", Category::Files, true)];
    let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
    let request = coordinator.open().request;
    coordinator.apply(batch(
        &request,
        "files",
        Ok(vec![
            result("files", Category::Files, "Alpha.txt"),
            result("files", Category::Files, "Beta.txt"),
        ]),
    ));
    assert_eq!(
        coordinator.snapshot().announcement.as_deref(),
        Some("Alpha.txt, Files, 1 of 2 results")
    );
    assert_eq!(
        coordinator.handle_key(KeyCommand::ArrowDown),
        KeyEffect::SelectionChanged
    );
    assert_eq!(
        coordinator.snapshot().announcement.as_deref(),
        Some("Beta.txt, Files, 2 of 2 results")
    );
    assert_eq!(coordinator.snapshot().rows[1].primary_label, "Open");
    assert_eq!(
        coordinator.snapshot().rows[1].alternate_label,
        Some("Show in Folder")
    );
    let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::AlternateReturn)
    else {
        panic!("alternate return activates");
    };
    assert!(matches!(activation.action, Action::RevealFile { .. }));
    assert_eq!(coordinator.snapshot().phase, Phase::Activating);
    assert_eq!(
        coordinator.snapshot().activating,
        Some(ActivationMode::Alternate)
    );
    assert_eq!(coordinator.handle_key(KeyCommand::Return), KeyEffect::None);
    assert_eq!(
        coordinator.handle_key(KeyCommand::ArrowDown),
        KeyEffect::None
    );
    assert!(coordinator.set_query("ignored while opening").is_none());
    assert!(
        coordinator.finish_activation(Ok(rmac_launcher_system::Receipt {
            activation: activation.id,
            outcome: rmac_launcher_system::Outcome::FileRevealed,
        }))
    );
    assert_eq!(coordinator.snapshot().phase, Phase::Closed);
}

#[test]
fn progressive_results_expose_searching_and_degraded_state_without_flicker() {
    let descriptors = vec![
        descriptor("apps", Category::Applications, false),
        descriptor("settings", Category::Settings, false),
    ];
    let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
    let request = coordinator.open().request;
    assert!(coordinator.apply(batch(
        &request,
        "apps",
        Ok(vec![result("apps", Category::Applications, "Terminal",)]),
    )));
    assert_eq!(
        coordinator.snapshot().phase,
        Phase::Results {
            still_searching: true,
            degraded: false,
        }
    );
    assert!(coordinator.apply(batch(
        &request,
        "settings",
        Err(ProviderError {
            detail: "settings service unavailable".into(),
        }),
    )));
    let snapshot = coordinator.snapshot();
    assert_eq!(
        snapshot.phase,
        Phase::Results {
            still_searching: false,
            degraded: true,
        }
    );
    assert_eq!(snapshot.rows[0].title, "Terminal");
    assert_eq!(snapshot.failed_providers, 1);
}

#[test]
fn catalog_revisions_restart_open_search_once_and_health_retains_results() {
    let descriptors = vec![descriptor("apps", Category::Applications, false)];
    let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
    let original = coordinator.open().request;
    let starting = coordinator.apply_catalog(CatalogUpdate {
        health: CatalogHealth::Starting,
        revision: 0,
        changed: false,
        detail: None,
    });
    assert!(starting.visible);
    assert!(starting.request.is_none());

    let refreshed = coordinator.apply_catalog(CatalogUpdate {
        health: CatalogHealth::Healthy,
        revision: 1,
        changed: true,
        detail: None,
    });
    let refreshed = refreshed.request.expect("new revision reissues query");
    assert!(original.cancellation.is_cancelled());
    assert!(
        !coordinator
            .apply_catalog(CatalogUpdate {
                health: CatalogHealth::Healthy,
                revision: 1,
                changed: true,
                detail: None,
            })
            .visible
    );
    assert!(coordinator.apply(batch(
        &refreshed,
        "apps",
        Ok(vec![result("apps", Category::Applications, "Terminal",)]),
    )));

    let unavailable = coordinator.apply_catalog(CatalogUpdate {
        health: CatalogHealth::Unavailable,
        revision: 1,
        changed: false,
        detail: Some("private catalog path detail".into()),
    });
    assert!(unavailable.visible);
    assert!(unavailable.request.is_none());
    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.application_catalog, CatalogHealth::Unavailable);
    assert_eq!(snapshot.rows[0].title, "Terminal");
    assert_eq!(
        snapshot.phase,
        Phase::Results {
            still_searching: false,
            degraded: true,
        }
    );
    assert!(!snapshot
        .announcement
        .expect("selection is announced")
        .contains("private catalog"));

    let stale = coordinator.apply_catalog(CatalogUpdate {
        health: CatalogHealth::Starting,
        revision: 0,
        changed: true,
        detail: None,
    });
    assert!(!stale.visible);
    assert_eq!(
        coordinator.snapshot().application_catalog,
        CatalogHealth::Unavailable
    );
}

#[test]
fn provider_and_activation_failures_are_truthful_but_private_safe() {
    let descriptors = vec![descriptor("files", Category::Files, true)];
    let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
    let request = coordinator.open().request;
    coordinator.apply(batch(
        &request,
        "files",
        Err(ProviderError {
            detail: "secret /home/alex/report.txt".into(),
        }),
    ));
    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.phase, Phase::Unavailable);
    assert_eq!(
        snapshot.announcement.as_deref(),
        Some("Search providers are unavailable")
    );
    assert!(!snapshot.announcement.unwrap().contains("alex"));

    let request = coordinator.set_query("report").expect("overlay open");
    coordinator.apply(batch(
        &request,
        "files",
        Ok(vec![result("files", Category::Files, "Report.txt")]),
    ));
    let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
        panic!("return activates");
    };
    let failure = rmac_launcher_system::BackendError::new(
        rmac_launcher_system::FailureKind::Rejected,
        "secret /home/alex/report.txt",
    );
    let error = futures_lite::future::block_on(rmac_launcher_system::execute(
        activation.id,
        &activation.action,
        &RejectingBackend(failure),
    ))
    .expect_err("activation fails");
    assert!(coordinator.finish_activation(Err(error)));
    let snapshot = coordinator.snapshot();
    assert_eq!(snapshot.phase, Phase::ActivationFailed);
    assert_eq!(
        snapshot.announcement.as_deref(),
        Some("Could not open the file")
    );
    assert!(!snapshot.announcement.unwrap().contains("alex"));
}

#[test]
fn policy_change_cancels_private_work_and_escape_ignores_late_activation() {
    let files = descriptor("files", Category::Files, true);
    let mut policies = BTreeMap::new();
    policies.insert(
        files.id.clone(),
        rmac_shell_settings::ProviderPolicy {
            enabled: true,
            allow_private_content: true,
            allow_network: false,
        },
    );
    let mut coordinator = Coordinator::new(vec![files.clone()], policies);
    let original = coordinator.open().request;
    let replacement = coordinator
        .set_policies(BTreeMap::new())
        .expect("open overlay restarts search");
    assert!(original.cancellation.is_cancelled());
    assert!(replacement.providers.is_empty());
    assert_eq!(coordinator.snapshot().phase, Phase::Empty);

    let request = coordinator
        .set_policies({
            let mut policies = BTreeMap::new();
            policies.insert(
                files.id,
                rmac_shell_settings::ProviderPolicy {
                    enabled: true,
                    allow_private_content: true,
                    allow_network: false,
                },
            );
            policies
        })
        .expect("provider is readmitted");
    coordinator.apply(batch(
        &request,
        "files",
        Ok(vec![result("files", Category::Files, "Report.txt")]),
    ));
    let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
        panic!("return activates");
    };
    assert_eq!(
        coordinator.handle_key(KeyCommand::Escape),
        KeyEffect::Dismissed
    );
    assert!(
        !coordinator.finish_activation(Ok(rmac_launcher_system::Receipt {
            activation: activation.id,
            outcome: rmac_launcher_system::Outcome::SettingOpened,
        }))
    );
    assert_eq!(coordinator.snapshot().phase, Phase::Closed);
}

#[test]
fn environment_change_reissues_open_query_with_new_descriptors() {
    let apps = descriptor("apps", Category::Applications, false);
    let mut coordinator = Coordinator::new(vec![apps], BTreeMap::new());
    let original = coordinator.open().request;
    let settings = descriptor("settings", Category::Settings, false);
    let replacement = coordinator
        .set_environment(vec![settings.clone()], BTreeMap::new())
        .expect("changed provider environment restarts the open query");
    assert!(original.cancellation.is_cancelled());
    assert_eq!(replacement.providers, [settings]);
    assert!(coordinator
        .set_environment(replacement.providers.clone(), BTreeMap::new())
        .is_none());
}

struct RejectingBackend(rmac_launcher_system::BackendError);

impl rmac_launcher_system::Backend for RejectingBackend {
    fn launch<'a>(
        &'a self,
        _: &'a rmac_apps::LaunchSpec,
    ) -> rmac_launcher_system::BackendFuture<
        'a,
        Result<rmac_app_launch::Outcome, rmac_launcher_system::BackendError>,
    > {
        Box::pin(async { Err(self.0.clone()) })
    }

    fn open_setting<'a>(
        &'a self,
        _: &'a str,
    ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
    {
        Box::pin(async { Err(self.0.clone()) })
    }

    fn open_file<'a>(
        &'a self,
        _: &'a std::path::Path,
    ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
    {
        Box::pin(async { Err(self.0.clone()) })
    }

    fn reveal_file<'a>(
        &'a self,
        _: &'a std::path::Path,
    ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
    {
        Box::pin(async { Err(self.0.clone()) })
    }

    fn copy_text<'a>(
        &'a self,
        _: &'a str,
    ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
    {
        Box::pin(async { Err(self.0.clone()) })
    }
}

#[test]
fn a_successful_activation_reports_its_choice_once() {
    let descriptors = vec![descriptor("files", Category::Files, true)];
    let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
    let request = coordinator.open().request;
    let request = coordinator.set_query("alpha").unwrap_or(request);
    coordinator.apply(batch(
        &request,
        "files",
        Ok(vec![result("files", Category::Files, "Alpha.txt")]),
    ));
    let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
        panic!("return activates the top hit");
    };
    assert_eq!(coordinator.take_completed_choice(), None);
    assert!(
        coordinator.finish_activation(Ok(rmac_launcher_system::Receipt {
            activation: activation.id,
            outcome: rmac_launcher_system::Outcome::FileOpened,
        }))
    );
    let (query, id) = coordinator
        .take_completed_choice()
        .expect("the opened row is learned");
    assert_eq!(query, "alpha");
    assert_eq!(id.local, "alpha.txt");
    assert_eq!(coordinator.take_completed_choice(), None);
}

#[test]
fn a_failed_activation_teaches_nothing() {
    let descriptors = vec![descriptor("files", Category::Files, true)];
    let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
    let request = coordinator.open().request;
    coordinator.apply(batch(
        &request,
        "files",
        Ok(vec![result("files", Category::Files, "Alpha.txt")]),
    ));
    let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
        panic!("return activates the top hit");
    };
    let error = futures_lite::future::block_on(rmac_launcher_system::execute(
        activation.id,
        &activation.action,
        &RejectingBackend(rmac_launcher_system::BackendError::new(
            rmac_launcher_system::FailureKind::Rejected,
            "rejected",
        )),
    ))
    .expect_err("activation fails");
    assert!(coordinator.finish_activation(Err(error)));
    assert_eq!(coordinator.take_completed_choice(), None);
}

#[test]
fn command_arrows_move_by_section() {
    let descriptors = vec![
        descriptor("apps", Category::Applications, false),
        descriptor("settings", Category::Settings, false),
    ];
    let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
    let request = coordinator.open().request;
    let request = coordinator.set_query("s").unwrap_or(request);
    coordinator.apply(batch(
        &request,
        "apps",
        Ok(vec![
            result("apps", Category::Applications, "Safari"),
            result("apps", Category::Applications, "Stickies"),
        ]),
    ));
    coordinator.apply(batch(
        &request,
        "settings",
        Ok(vec![result("settings", Category::Settings, "Sound")]),
    ));
    let selected = |coordinator: &Coordinator| {
        coordinator
            .snapshot()
            .rows
            .iter()
            .find(|row| row.selected)
            .map(|row| row.title.clone())
    };
    assert_eq!(selected(&coordinator).as_deref(), Some("Safari"));
    assert!(coordinator.move_selection_by_section(MoveSelection::Next));
    assert_eq!(selected(&coordinator).as_deref(), Some("Stickies"));
    assert!(coordinator.move_selection_by_section(MoveSelection::Next));
    assert_eq!(selected(&coordinator).as_deref(), Some("Sound"));
    assert!(!coordinator.move_selection_by_section(MoveSelection::Next));
}
