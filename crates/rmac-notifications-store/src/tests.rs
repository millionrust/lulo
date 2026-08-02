//! Focused Notification Center storage contracts.

use super::*;
use rmac_notifications::protocol::{self, PortalInput};
use rmac_notifications::{DeliveryPolicy, Server, TimeoutPolicy};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "rmac-notification-store-{}-{label}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
        .join("history.json")
}

fn notification(app: &str, external: &str, updated: u64, urgent: bool) -> Notification {
    let mut request = protocol::portal(PortalInput {
        app_id: app.into(),
        id: external.into(),
        title: Some(format!("Private title {external}")),
        body: Some("Private body".into()),
        default_action: Some("app.open".into()),
        ..PortalInput::default()
    })
    .unwrap();
    if urgent {
        request.priority = Priority::Urgent;
    }
    let mut server = Server::new(10, TimeoutPolicy::default());
    let outcome = server
        .post(request, Time(updated), DeliveryPolicy::default())
        .unwrap();
    let mut notification = server
        .active()
        .find(|record| record.id == outcome.id)
        .unwrap()
        .clone();
    notification.id = NotificationId::from_protocol(updated as u32).unwrap();
    notification
}

#[test]
fn private_store_round_trips_actions_content_policy_and_mode() {
    let path = temp_path("roundtrip");
    let store = Store::at(path.clone());
    assert!(!format!("{store:?}").contains(path.to_string_lossy().as_ref()));
    let mut center = Center::default();
    let app_id = AppId::parse("org.example.Chat").unwrap();
    center.upsert(notification("org.example.Chat", "one", 10, true));
    center
        .set_policy(
            app_id.clone(),
            AppPolicy {
                sounds: false,
                lock_preview: LockPreview::HideContent,
                ..AppPolicy::default()
            },
        )
        .unwrap();
    store.save(&center).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.recovery, Recovery::None);
    assert_eq!(loaded.center.history().len(), 1);
    assert_eq!(
        loaded.center.history()[0].content.title(),
        "Private title one"
    );
    assert_eq!(
        loaded.center.history()[0]
            .default_action
            .as_ref()
            .unwrap()
            .id(),
        "app.open"
    );
    assert!(!loaded.center.policy(&app_id).sounds);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn lock_projection_applies_the_stricter_hint_without_actions_or_unbounded_data() {
    let mut center = Center::default();
    for (app, preview) in [
        ("org.example.Chat", LockPreview::Show),
        ("org.example.Mail", LockPreview::HideContent),
        ("org.example.AppHint", LockPreview::Show),
        ("org.example.Secret", LockPreview::Hide),
        ("org.example.AppHide", LockPreview::Show),
    ] {
        center
            .set_policy(
                AppId::parse(app).unwrap(),
                AppPolicy {
                    lock_preview: preview,
                    ..AppPolicy::default()
                },
            )
            .unwrap();
    }

    center.upsert(notification("org.example.Chat", "chat", 10, false));
    center.upsert(notification("org.example.Mail", "mail", 20, false));
    let mut app_restricted = notification("org.example.AppHint", "restricted", 30, false);
    app_restricted.display.lock_screen = LockScreenVisibility::HideContent;
    center.upsert(app_restricted);
    center.upsert(notification("org.example.Secret", "secret", 40, false));
    let mut app_hidden = notification("org.example.AppHide", "hidden", 45, false);
    app_hidden.display.lock_screen = LockScreenVisibility::Hide;
    center.upsert(app_hidden);

    let previews = center.lock_previews(usize::MAX);
    assert_eq!(previews.len(), 3);
    assert_eq!(previews[0].app_id.as_str(), "org.example.AppHint");
    assert!(previews[0].content.is_none());
    assert_eq!(previews[1].app_id.as_str(), "org.example.Mail");
    assert!(previews[1].content.is_none());
    assert_eq!(previews[2].app_id.as_str(), "org.example.Chat");
    assert_eq!(
        previews[2].content.as_ref().unwrap().title(),
        "Private title chat"
    );
    let debug = format!("{previews:?}");
    assert!(!debug.contains("org.example"));
    assert!(!debug.contains("Private title"));

    let mail = AppId::parse("org.example.Mail").unwrap();
    center.mark_all_read(Some(&mail));
    assert!(center
        .lock_previews(usize::MAX)
        .iter()
        .all(|preview| preview.app_id.as_str() != mail.as_str()));

    let chat = AppId::parse("org.example.Chat").unwrap();
    for index in 50..80 {
        center.upsert(notification(
            "org.example.Chat",
            &format!("bounded-{index}"),
            index,
            false,
        ));
    }
    assert_eq!(center.lock_previews(usize::MAX).len(), MAX_LOCK_PREVIEWS);
    center
        .set_policy(
            chat,
            AppPolicy {
                lock_preview: LockPreview::Hide,
                ..AppPolicy::default()
            },
        )
        .unwrap();
    assert!(center
        .lock_previews(usize::MAX)
        .iter()
        .all(|preview| preview.app_id.as_str() != "org.example.Chat"));
}

#[test]
fn corrupt_primary_recovers_last_good_without_logging_payload() {
    let path = temp_path("recovery");
    let store = Store::at(path.clone());
    let mut center = Center::default();
    center.upsert(notification("org.example.Chat", "one", 10, false));
    store.save(&center).unwrap();
    std::fs::write(&path, b"private corrupt payload").unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.recovery, Recovery::LastGood);
    assert_eq!(loaded.center.history().len(), 1);
    let debug = format!("{:?}", loaded.center);
    assert!(!debug.contains("Private title"));
    assert!(!debug.contains("org.example"));
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[test]
fn grouping_clear_read_badges_and_policy_are_coherent() {
    let mut center = Center::default();
    center.upsert(notification("org.example.Chat", "one", 10, false));
    center.upsert(notification("org.example.Mail", "two", 20, true));
    center.upsert(notification("org.example.Chat", "three", 30, false));
    assert_eq!(center.groups().len(), 2);
    assert_eq!(center.groups()[0].app_id.as_str(), "org.example.Chat");
    assert_eq!(
        center.indicator(),
        Indicator {
            unread_count: 3,
            has_urgent: true
        }
    );
    let mail = AppId::parse("org.example.Mail").unwrap();
    center.mark_all_read(Some(&mail));
    assert_eq!(
        center.indicator(),
        Indicator {
            unread_count: 2,
            has_urgent: false
        }
    );
    center
        .set_policy(
            mail.clone(),
            AppPolicy {
                history: false,
                ..AppPolicy::default()
            },
        )
        .unwrap();
    assert!(center
        .history()
        .iter()
        .all(|record| record.source.app_id() != &mail));
    center.clear(None);
    assert!(center.history().is_empty());
}

#[test]
fn transient_or_policy_blocked_notifications_never_enter_history() {
    let mut transient = notification("org.example.Chat", "one", 10, false);
    transient.delivery.history = false;
    let mut center = Center::default();
    center.upsert(transient);
    assert!(center.history().is_empty());
    let app_id = AppId::parse("org.example.Chat").unwrap();
    center
        .set_policy(
            app_id,
            AppPolicy {
                history: false,
                ..AppPolicy::default()
            },
        )
        .unwrap();
    center.upsert(notification("org.example.Chat", "two", 20, false));
    assert!(center.history().is_empty());
}

#[test]
fn per_app_sound_policy_reaches_authoritative_delivery() {
    let policy = AppPolicy {
        sounds: false,
        ..AppPolicy::default()
    };
    let mut server = Server::new(10, TimeoutPolicy::default());
    let request = protocol::portal(PortalInput {
        app_id: "org.example.Chat".into(),
        id: "one".into(),
        title: Some("Message".into()),
        ..PortalInput::default()
    })
    .unwrap();
    let outcome = server
        .post(request, Time(1), policy.delivery(false))
        .unwrap();
    assert!(outcome.delivery.banner);
    assert!(!outcome.delivery.sound);
    assert!(outcome.delivery.history);
}

#[test]
fn process_scoped_id_reuse_does_not_replace_another_source() {
    let mut center = Center::default();
    let first = notification("org.example.Chat", "one", 10, false);
    let reused = notification("org.example.Mail", "two", 10, false);
    assert_eq!(first.id, reused.id);
    center.upsert(first);
    center.upsert(reused);
    assert_eq!(center.history().len(), 2);

    let replacement = notification("org.example.Chat", "one", 10, true);
    center.upsert(replacement);
    assert_eq!(center.history().len(), 2);
    assert!(center
        .history()
        .iter()
        .any(|record| record.source.app_id().as_str() == "org.example.Mail"));
}

#[test]
fn malformed_and_oversized_files_recover_to_empty_without_content_errors() {
    let path = temp_path("invalid");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
    let loaded = Store::at(path.clone()).load().unwrap();
    assert_eq!(loaded.recovery, Recovery::Empty);
    assert!(loaded.center.history().is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
