use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::filesystem::{HASH_HEX_BYTES, LOCK_FILE};

fn root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rmac-wallpaper-portal-{label}-{}-{unique}",
        std::process::id()
    ))
}

fn fixture(root: &Path, name: &str, color: [u8; 4]) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let path = root.join(name);
    image::RgbaImage::from_pixel(8, 4, image::Rgba(color))
        .save(&path)
        .unwrap();
    path
}

fn request(path: &Path) -> rmac_wallpaper::portal::Request {
    rmac_wallpaper::portal::Request {
        app_id: "org.example.Photos".into(),
        uri: url::Url::from_file_path(path).unwrap().into(),
        set_on: rmac_wallpaper::portal::SetOn::Background,
        show_preview: false,
    }
}

fn importer(root: &Path) -> Importer {
    Importer::new(root.join("managed"), root.join("config/shell.json")).unwrap()
}

fn recv_event(events: &async_channel::Receiver<broker::PreviewEvent>) -> broker::PreviewEvent {
    futures_lite::future::block_on(events.recv()).unwrap()
}

fn recv_open(events: &async_channel::Receiver<broker::PreviewEvent>) -> broker::PreviewRequest {
    match recv_event(events).into_kind() {
        broker::PreviewEventKind::Open(request) => request,
        broker::PreviewEventKind::Close { id } => panic!("expected Open before Close for {id:?}"),
    }
}

fn recv_close(events: &async_channel::Receiver<broker::PreviewEvent>, expected: RequestId) {
    match recv_event(events).into_kind() {
        broker::PreviewEventKind::Close { id } => assert_eq!(id, expected),
        broker::PreviewEventKind::Open(request) => {
            panic!("expected Close before Open for {:?}", request.id())
        }
    }
}

fn visible_entries(root: &Path) -> Vec<String> {
    let mut entries = std::fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != LOCK_FILE)
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

#[test]
fn preparation_always_produces_a_bounded_private_preview() {
    let root = root("prepare");
    let source = fixture(&root, "private-photo.png", [1, 2, 3, 255]);
    let importer = importer(&root);

    let prepared = importer.prepare(request(&source)).unwrap();

    assert_eq!(prepared.app_id(), "org.example.Photos");
    assert_eq!(prepared.image().physical_size().width, 8);
    assert_eq!(prepared.image().physical_size().height, 4);
    assert!(prepared.byte_len() > 0);
    assert_eq!(prepared.format(), rmac_wallpaper_system::ImageFormat::Png);
    let debug = format!("{prepared:?}");
    assert!(!debug.contains("private-photo"));
    assert_eq!(visible_entries(&root.join("managed")).len(), 1);

    drop(prepared);
    assert!(visible_entries(&root.join("managed")).is_empty());
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_decode_and_decline_leave_settings_and_imports_untouched() {
    let root = root("decline");
    std::fs::create_dir_all(&root).unwrap();
    let corrupt = root.join("corrupt.png");
    std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
    let importer = importer(&root);
    let error = importer.prepare(request(&corrupt)).unwrap_err();
    assert_eq!(
        error.kind,
        ErrorKind::Decode(rmac_wallpaper_image::ErrorKind::Decode)
    );
    assert!(visible_entries(&root.join("managed")).is_empty());

    let source = fixture(&root, "valid.png", [4, 5, 6, 255]);
    let prepared = importer.prepare(request(&source)).unwrap();
    assert_eq!(
        importer.finish(prepared, Consent::Decline).unwrap(),
        Outcome::Cancelled
    );
    assert!(visible_entries(&root.join("managed")).is_empty());
    assert!(!root.join("config/shell.json").exists());

    let cancelled = importer.prepare(request(&source)).unwrap();
    assert_eq!(
        importer.finish(cancelled, Consent::Cancel).unwrap(),
        Outcome::Cancelled
    );
    assert!(visible_entries(&root.join("managed")).is_empty());
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn acceptance_imports_exact_reviewed_bytes_and_commits_whole_desktop_fill() {
    let root = root("accept");
    let source = fixture(&root, "selected.png", [10, 20, 30, 255]);
    let settings_path = root.join("config/shell.json");
    let store = rmac_shell_settings::ShellSettingsStore::new(settings_path.clone());
    let original = rmac_shell_settings::ShellSettings {
        dock: rmac_shell_settings::DockSettings {
            autohide: true,
            ..Default::default()
        },
        wallpaper: rmac_shell_settings::WallpaperSettings {
            default: rmac_shell_settings::WallpaperSelection {
                source: Some("builtin:rmac-aurora".into()),
                fit: rmac_shell_settings::WallpaperFit::Fit,
            },
            per_output: [(
                "DP-1".into(),
                rmac_shell_settings::WallpaperSelection::default(),
            )]
            .into_iter()
            .collect(),
        },
        ..Default::default()
    };
    store.save(&original).unwrap();
    let importer = Importer::new(root.join("managed"), settings_path).unwrap();
    let prepared = importer.prepare(request(&source)).unwrap();
    let reviewed = rmac_storage::fingerprint_bounded_regular_no_follow(
        &source,
        rmac_wallpaper_system::MAX_WALLPAPER_BYTES,
    )
    .unwrap();
    std::fs::write(&source, b"changed after preview").unwrap();

    assert_eq!(
        importer.finish(prepared, Consent::Accept).unwrap(),
        Outcome::Applied {
            cleanup_pending: false
        }
    );

    let loaded = store.load().unwrap().settings;
    assert!(loaded.dock.autohide);
    assert_eq!(
        loaded.wallpaper.default.fit,
        rmac_shell_settings::WallpaperFit::Fill
    );
    assert!(loaded.wallpaper.per_output.is_empty());
    let managed = PathBuf::from(loaded.wallpaper.default.source.unwrap());
    assert_eq!(managed.parent(), Some(root.join("managed").as_path()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(std::fs::metadata(&managed).unwrap().nlink(), 1);
    }
    assert_eq!(
        rmac_storage::fingerprint_bounded_no_follow(
            &managed,
            rmac_wallpaper_system::MAX_WALLPAPER_BYTES
        )
        .unwrap(),
        reviewed
    );
    assert_eq!(visible_entries(&root.join("managed")).len(), 1);
    drop(importer);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_failure_removes_only_a_new_import_and_keeps_reusable_content() {
    let root = root("settings-failure");
    let source = fixture(&root, "selected.png", [40, 50, 60, 255]);
    let healthy_settings = root.join("config/shell.json");
    let importer = Importer::new(root.join("managed"), healthy_settings.clone()).unwrap();
    let first = importer.prepare(request(&source)).unwrap();
    importer.finish(first, Consent::Accept).unwrap();
    let existing = visible_entries(&root.join("managed"));
    assert_eq!(existing.len(), 1);
    drop(importer);

    let blocker = root.join("not-a-directory");
    std::fs::write(&blocker, b"block").unwrap();
    let failing = Importer::new(root.join("managed"), blocker.join("shell.json")).unwrap();
    let novel = fixture(&root, "novel.png", [70, 80, 90, 255]);
    let new_import = failing.prepare(request(&novel)).unwrap();
    let error = failing.finish(new_import, Consent::Accept).unwrap_err();
    assert_eq!(error.operation, Operation::ReadSettings);
    assert_eq!(visible_entries(&root.join("managed")), existing);

    let second = failing.prepare(request(&source)).unwrap();
    let error = failing.finish(second, Consent::Accept).unwrap_err();
    assert_eq!(error.operation, Operation::ReadSettings);
    assert_eq!(visible_entries(&root.join("managed")), existing);
    assert!(!format!("{error:?}").contains(root.to_string_lossy().as_ref()));
    drop(failing);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn prepared_request_cannot_cross_import_authorities() {
    let root = root("wrong-authority");
    let source = fixture(&root, "selected.png", [2, 4, 8, 255]);
    let first = Importer::new(
        root.join("first-managed"),
        root.join("first-config/shell.json"),
    )
    .unwrap();
    let second = Importer::new(
        root.join("second-managed"),
        root.join("second-config/shell.json"),
    )
    .unwrap();
    let prepared = first.prepare(request(&source)).unwrap();

    let error = second.finish(prepared, Consent::Accept).unwrap_err();

    assert_eq!(error.kind, ErrorKind::WrongAuthority);
    assert!(visible_entries(&root.join("first-managed")).is_empty());
    drop(first);
    drop(second);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn exclusive_authority_recovers_only_recognized_unreferenced_files() {
    let root = root("recovery");
    let managed = root.join("managed");
    let settings = root.join("config/shell.json");
    let importer = Importer::new(managed.clone(), settings.clone()).unwrap();
    let busy = Importer::new(managed.clone(), settings.clone()).unwrap_err();
    assert_eq!(busy.kind, ErrorKind::AuthorityBusy);
    drop(importer);

    std::fs::write(managed.join(".incoming-crashed"), b"staged").unwrap();
    std::fs::write(
        managed.join(format!("{}.png", "a".repeat(HASH_HEX_BYTES))),
        b"orphan",
    )
    .unwrap();
    std::fs::write(managed.join("leave-me.txt"), b"unknown").unwrap();
    let recovered = Importer::new(managed.clone(), settings).unwrap();
    assert_eq!(visible_entries(&managed), ["leave-me.txt"]);
    drop(recovered);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn portal_response_mapping_is_exact() {
    assert_eq!(
        Outcome::Applied {
            cleanup_pending: false
        }
        .response() as u32,
        0
    );
    assert_eq!(Outcome::Cancelled.response() as u32, 1);
    assert_eq!(PortalResponse::Other as u32, 2);
}

#[test]
fn broker_requires_preview_and_applies_one_exact_decision() {
    let root = root("broker-accept");
    let source = fixture(&root, "private-preview.png", [11, 22, 33, 255]);
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);
    let cancellation = broker::Cancellation::new();
    let worker = {
        let broker = broker.clone();
        let cancellation = cancellation.clone();
        let request = request(&source);
        std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(
                request,
                "wayland:private-parent".into(),
                cancellation,
            ))
        })
    };

    let preview = recv_open(&previews);
    assert_eq!(preview.app_id(), "org.example.Photos");
    assert_eq!(preview.parent_window(), "wayland:private-parent");
    assert_eq!(preview.image().physical_size().width, 8);
    assert!(preview.source_bytes() > 0);
    assert!(!format!("{preview:?}").contains("private-parent"));
    assert!(broker.decide(preview.id(), Consent::Accept));
    assert!(!broker.decide(preview.id(), Consent::Decline));
    recv_close(&previews, preview.id());
    assert_eq!(worker.join().unwrap(), PortalResponse::Success);
    assert_eq!(broker.pending_count(), 0);
    assert!(root.join("config/shell.json").exists());

    drop(preview);
    drop(previews);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn close_and_decline_are_private_safe_and_replay_is_inert() {
    let root = root("broker-cancel");
    let source = fixture(&root, "selected.png", [7, 8, 9, 255]);
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);

    let declined = {
        let broker = broker.clone();
        let request = request(&source);
        std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(
                request,
                "".into(),
                broker::Cancellation::new(),
            ))
        })
    };
    let decline_preview = recv_open(&previews);
    assert!(broker.decide(decline_preview.id(), Consent::Decline));
    recv_close(&previews, decline_preview.id());
    assert_eq!(declined.join().unwrap(), PortalResponse::Cancelled);

    let cancellation = broker::Cancellation::new();
    let cancelled = {
        let broker = broker.clone();
        let request = request(&source);
        let cancellation = cancellation.clone();
        std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(request, "".into(), cancellation))
        })
    };
    let mut presenter = preview::Presenter::default();
    let cancel_open = recv_event(&previews);
    let cancel_id = match cancel_open.kind() {
        broker::PreviewEventKind::Open(request) => request.id(),
        broker::PreviewEventKind::Close { id } => panic!("unexpected Close for {id:?}"),
    };
    assert_eq!(
        presenter.apply(cancel_open),
        preview::Update::Opened { id: cancel_id }
    );
    assert!(cancellation.cancel());
    assert!(!cancellation.cancel());
    assert_eq!(
        presenter.apply(recv_event(&previews)),
        preview::Update::Closed {
            id: cancel_id,
            next: None,
        }
    );
    assert_eq!(cancelled.join().unwrap(), PortalResponse::Cancelled);
    assert!(!broker.decide(cancel_id, Consent::Accept));
    assert_eq!(broker.pending_count(), 0);
    assert!(!root.join("config/shell.json").exists());
    assert!(visible_entries(&root.join("managed")).is_empty());

    drop(decline_preview);
    drop(previews);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn broker_admission_is_bounded_before_additional_decode() {
    let root = root("broker-bound");
    let source = fixture(&root, "selected.png", [91, 92, 93, 255]);
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);
    let mut workers = Vec::new();
    let mut cancellations = Vec::new();
    let mut preview_requests = Vec::new();

    for _ in 0..broker::MAX_PENDING_REQUESTS {
        let cancellation = broker::Cancellation::new();
        cancellations.push(cancellation.clone());
        let broker = broker.clone();
        let request = request(&source);
        workers.push(std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(request, "".into(), cancellation))
        }));
        preview_requests.push(recv_open(&previews));
    }
    assert_eq!(broker.pending_count(), broker::MAX_PENDING_REQUESTS);

    assert_eq!(
        futures_lite::future::block_on(broker.request(
            request(&source),
            "".into(),
            broker::Cancellation::new(),
        )),
        PortalResponse::Other
    );
    for cancellation in cancellations {
        assert!(cancellation.cancel());
    }
    for worker in workers {
        assert_eq!(worker.join().unwrap(), PortalResponse::Cancelled);
    }
    // Terminal events retain all admission leases until the UI consumes them.
    assert_eq!(
        futures_lite::future::block_on(broker.request(
            request(&source),
            "".into(),
            broker::Cancellation::new(),
        )),
        PortalResponse::Other
    );
    let mut closing = preview_requests
        .iter()
        .map(broker::PreviewRequest::id)
        .collect::<std::collections::BTreeSet<_>>();
    while !closing.is_empty() {
        match recv_event(&previews).into_kind() {
            broker::PreviewEventKind::Close { id } => assert!(closing.remove(&id)),
            broker::PreviewEventKind::Open(request) => {
                panic!("unexpected extra Open for {:?}", request.id())
            }
        }
    }
    assert_eq!(broker.pending_count(), 0);
    assert!(visible_entries(&root.join("managed")).is_empty());

    drop(preview_requests);
    drop(previews);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_preview_consumer_and_invalid_parent_fail_without_mutation() {
    let root = root("broker-unavailable");
    let source = fixture(&root, "selected.png", [101, 102, 103, 255]);
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);

    assert_eq!(
        futures_lite::future::block_on(broker.request(
            request(&source),
            "bad\nparent".into(),
            broker::Cancellation::new(),
        )),
        PortalResponse::Other
    );
    assert!(previews.try_recv().is_err());
    drop(previews);
    assert_eq!(
        futures_lite::future::block_on(broker.request(
            request(&source),
            "".into(),
            broker::Cancellation::new(),
        )),
        PortalResponse::Other
    );
    assert_eq!(broker.pending_count(), 0);
    assert!(!root.join("config/shell.json").exists());
    assert!(visible_entries(&root.join("managed")).is_empty());

    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn consent_presenter_serializes_dialogs_and_waits_for_terminal_close() {
    let root = root("presenter-flow");
    let source = fixture(&root, "selected.png", [120, 121, 122, 255]);
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);
    let mut presenter = preview::Presenter::default();

    let first_worker = {
        let broker = broker.clone();
        let request = request(&source);
        std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(
                request,
                "wayland:first-private-parent".into(),
                broker::Cancellation::new(),
            ))
        })
    };
    let first_event = recv_event(&previews);
    let first_id = match first_event.kind() {
        broker::PreviewEventKind::Open(request) => request.id(),
        broker::PreviewEventKind::Close { id } => panic!("unexpected Close for {id:?}"),
    };
    assert_eq!(
        presenter.apply(first_event),
        preview::Update::Opened { id: first_id }
    );

    let second_worker = {
        let broker = broker.clone();
        let request = request(&source);
        std::thread::spawn(move || {
            futures_lite::future::block_on(broker.request(
                request,
                "wayland:second-private-parent".into(),
                broker::Cancellation::new(),
            ))
        })
    };
    let second_event = recv_event(&previews);
    let second_id = match second_event.kind() {
        broker::PreviewEventKind::Open(request) => request.id(),
        broker::PreviewEventKind::Close { id } => panic!("unexpected Close for {id:?}"),
    };
    assert_eq!(
        presenter.apply(second_event),
        preview::Update::Queued {
            id: second_id,
            position: 1,
        }
    );
    assert_eq!(presenter.queued_count(), 1);

    let dialog = presenter.active().unwrap();
    assert_eq!(dialog.id(), first_id);
    assert_eq!(dialog.focused(), preview::Control::Accept);
    assert_eq!(dialog.phase(), preview::Phase::AwaitingDecision);
    assert_eq!(dialog.layout().destination.height, preview::PREVIEW_HEIGHT);
    let semantics = dialog.semantics();
    assert_eq!(semantics.title, "Change Wallpaper?");
    assert_eq!(semantics.accept_label, "Set Wallpaper");
    assert!(semantics.description.contains("every display"));
    assert!(semantics.description.contains("per-display"));
    assert!(!format!("{dialog:?}").contains("first-private-parent"));

    assert_eq!(
        presenter.input(preview::Key::Tab),
        preview::InputOutcome::FocusChanged(preview::Control::Cancel)
    );
    let first_decision = match presenter.input(preview::Key::Enter) {
        preview::InputOutcome::Decision(decision) => decision,
        outcome => panic!("expected first decision, got {outcome:?}"),
    };
    assert_eq!(first_decision.id, first_id);
    assert_eq!(first_decision.consent, Consent::Decline);
    assert!(broker.decide(first_decision.id, first_decision.consent));
    assert!(presenter
        .decision_delivery(first_decision.id, true)
        .is_none());
    assert_eq!(
        presenter.active().unwrap().phase(),
        preview::Phase::Resolving
    );
    assert_eq!(
        presenter.input(preview::Key::Escape),
        preview::InputOutcome::Ignored
    );

    let first_close = recv_event(&previews);
    assert_eq!(
        presenter.apply(first_close),
        preview::Update::Closed {
            id: first_id,
            next: Some(second_id),
        }
    );
    assert_eq!(presenter.active().unwrap().id(), second_id);
    assert_eq!(presenter.queued_count(), 0);

    let second_decision = match presenter.window_closed() {
        preview::InputOutcome::Decision(decision) => decision,
        outcome => panic!("expected window-close decision, got {outcome:?}"),
    };
    assert_eq!(second_decision.consent, Consent::Cancel);
    assert!(broker.decide(second_decision.id, second_decision.consent));
    let second_close = recv_event(&previews);
    assert_eq!(
        presenter.apply(second_close),
        preview::Update::Closed {
            id: second_id,
            next: None,
        }
    );
    assert!(presenter.active().is_none());
    assert_eq!(first_worker.join().unwrap(), PortalResponse::Cancelled);
    assert_eq!(second_worker.join().unwrap(), PortalResponse::Cancelled);

    drop(previews);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn consent_presenter_rejects_duplicate_malformed_and_excess_events() {
    fn synthetic(id: u64, rgba: Vec<u8>) -> broker::PreviewRequest {
        broker::PreviewRequest::new(
            RequestId(id),
            "org.example.Photos".into(),
            "".into(),
            std::sync::Arc::new(rmac_wallpaper_image::Decoded {
                width: 2,
                height: 2,
                rgba: rgba.into(),
            }),
            16,
            rmac_wallpaper_system::ImageFormat::Png,
        )
    }

    let mut presenter = preview::Presenter::default();
    let first = synthetic(1, vec![0; 16]);
    assert_eq!(
        presenter.apply(broker::PreviewEvent::open(first.clone())),
        preview::Update::Opened { id: RequestId(1) }
    );
    assert_eq!(
        presenter.apply(broker::PreviewEvent::open(first)),
        preview::Update::Rejected {
            id: RequestId(1),
            reason: preview::RejectReason::Duplicate,
        }
    );
    assert_eq!(
        presenter.apply(broker::PreviewEvent::open(synthetic(2, vec![0; 15]))),
        preview::Update::Rejected {
            id: RequestId(2),
            reason: preview::RejectReason::InvalidImage,
        }
    );
    for id in 2..=broker::MAX_PENDING_REQUESTS as u64 {
        let update = presenter.apply(broker::PreviewEvent::open(synthetic(id, vec![0; 16])));
        assert!(matches!(update, preview::Update::Queued { .. }));
    }
    assert_eq!(presenter.queued_count(), broker::MAX_PENDING_REQUESTS - 1);
    assert_eq!(
        presenter.apply(broker::PreviewEvent::open(synthetic(99, vec![0; 16]))),
        preview::Update::Rejected {
            id: RequestId(99),
            reason: preview::RejectReason::Capacity,
        }
    );
}

#[test]
fn wallpaper_interface_introspection_has_the_exact_backend_method_shape() {
    use zbus::object_server::Interface as _;

    let root = root("wire-introspection");
    let importer = importer(&root);
    let (broker, previews) = broker::Broker::new(importer);
    let interface = dbus::WallpaperInterface::new(broker.clone());
    let mut xml = String::new();
    interface.introspect_to_writer(&mut xml, 0);

    assert!(xml.contains("org.freedesktop.impl.portal.Wallpaper"));
    assert!(xml.contains("method name=\"SetWallpaperURI\""));
    assert!(xml.contains("arg name=\"handle\" type=\"o\" direction=\"in\""));
    assert!(xml.contains("arg name=\"app_id\" type=\"s\" direction=\"in\""));
    assert!(xml.contains("arg name=\"parent_window\" type=\"s\" direction=\"in\""));
    assert!(xml.contains("arg name=\"uri\" type=\"s\" direction=\"in\""));
    assert!(xml.contains("arg name=\"options\" type=\"a{sv}\" direction=\"in\""));
    assert!(xml.contains("arg type=\"u\" direction=\"out\""));
    assert!(!xml.contains("property name="));

    drop(interface);
    drop(previews);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}
