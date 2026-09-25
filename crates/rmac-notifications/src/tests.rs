//! Focused notification domain contracts.

use super::*;

fn portal_request(app: &str, external: &str, title: &str) -> Request {
    Request {
        source: Source::portal(AppId::parse(app).unwrap(), external).unwrap(),
        content: Content::new(title, "Private message body").unwrap(),
        priority: Priority::Normal,
        timeout: Timeout::Default,
        default_action: Some(Action::new("app.open", "Open", None).unwrap()),
        actions: vec![Action::new("app.reply", "Reply", None).unwrap()],
        category: Some("im.received".into()),
        sound: Sound::Policy,
        display: DisplayHints::default(),
        replaces: None,
    }
}

#[test]
fn portal_replacement_is_atomic_stable_and_app_scoped() {
    let mut server = Server::new(20, TimeoutPolicy::default());
    let first = server
        .post(
            portal_request("org.example.Chat", "message-4", "First"),
            Time(100),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let replaced = server
        .post(
            portal_request("org.example.Chat", "message-4", "Updated"),
            Time(200),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let other_app = server
        .post(
            portal_request("org.example.Mail", "message-4", "Mail"),
            Time(300),
            DeliveryPolicy::default(),
        )
        .unwrap();

    assert_eq!(replaced.id, first.id);
    assert_eq!(replaced.kind, PostKind::Replaced);
    assert!(!replaced.announce_as_new);
    assert_ne!(other_app.id, first.id);
    assert_eq!(server.active.get(&first.id).unwrap().created_at, Time(100));
    assert_eq!(
        server.active.get(&first.id).unwrap().content.title(),
        "Updated"
    );
    assert_eq!(server.history.len(), 2);
}

#[test]
fn legacy_replacement_checks_owner_and_preserves_numeric_id() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let mut original = portal_request("org.example.Chat", "one", "First");
    original.source = Source::Freedesktop {
        app_id: AppId::parse("org.example.Chat").unwrap(),
    };
    let first = server
        .post(original, Time(1), DeliveryPolicy::default())
        .unwrap();

    let mut replacement = portal_request("org.example.Chat", "unused", "Second");
    replacement.source = Source::Freedesktop {
        app_id: AppId::parse("org.example.Chat").unwrap(),
    };
    replacement.replaces = Some(first.id);
    assert_eq!(
        server
            .post(replacement.clone(), Time(2), DeliveryPolicy::default())
            .unwrap()
            .id,
        first.id
    );
    replacement.source = Source::Freedesktop {
        app_id: AppId::parse("org.attacker.App").unwrap(),
    };
    assert_eq!(
        server.post(replacement, Time(3), DeliveryPolicy::default()),
        Err(ServerError::WrongOwner)
    );
}

#[test]
fn focus_suppresses_banner_and_sound_but_retains_allowed_history() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let focused = DeliveryPolicy {
        focus_active: true,
        ..DeliveryPolicy::default()
    };
    let normal = server
        .post(
            portal_request("org.example.Chat", "one", "Message"),
            Time(0),
            focused,
        )
        .unwrap();
    assert_eq!(
        normal.delivery,
        Delivery {
            banner: false,
            history: true,
            sound: false
        }
    );

    let mut urgent = portal_request("org.example.Chat", "two", "Alarm");
    urgent.priority = Priority::Urgent;
    let urgent = server.post(urgent, Time(0), focused).unwrap();
    assert!(urgent.delivery.banner);
    assert!(urgent.delivery.sound);
}

#[test]
fn transient_tray_and_block_policies_are_enforced() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let mut transient = portal_request("org.example.Chat", "one", "Transient");
    transient.display.transient = true;
    let result = server
        .post(transient, Time(0), DeliveryPolicy::default())
        .unwrap();
    assert_eq!(
        result.delivery,
        Delivery {
            banner: true,
            history: false,
            sound: true
        }
    );

    let no_history = DeliveryPolicy {
        history: HistoryPolicy::Block,
        ..DeliveryPolicy::default()
    };
    let result = server
        .post(
            portal_request("org.example.Chat", "two", "No history"),
            Time(1),
            no_history,
        )
        .unwrap();
    assert_eq!(
        result.delivery,
        Delivery {
            banner: true,
            history: false,
            sound: true
        }
    );

    let blocked = DeliveryPolicy {
        enabled: false,
        ..DeliveryPolicy::default()
    };
    let result = server
        .post(
            portal_request("org.example.Chat", "three", "Blocked"),
            Time(2),
            blocked,
        )
        .unwrap();
    assert_eq!(
        result.delivery,
        Delivery {
            banner: false,
            history: false,
            sound: false
        }
    );
    assert_eq!(server.active().count(), 2);
}

#[test]
fn banner_and_sound_preferences_are_independent() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let policy = DeliveryPolicy {
        banner: BannerPolicy::Suppress,
        sounds: true,
        ..DeliveryPolicy::default()
    };
    let outcome = server
        .post(
            portal_request("org.example.Chat", "silent-banner", "Message"),
            Time(0),
            policy,
        )
        .unwrap();
    assert!(!outcome.delivery.banner);
    assert!(outcome.delivery.sound);
    assert!(outcome.delivery.history);
}

#[test]
fn default_expiry_respects_priority_and_explicit_protocol_values() {
    let policy = TimeoutPolicy::default();
    assert_eq!(
        expiry_for(Timeout::Default, Priority::Normal, Time(10), policy),
        Some(Time(5_010))
    );
    assert_eq!(
        expiry_for(Timeout::Default, Priority::Urgent, Time(10), policy),
        None
    );
    assert_eq!(Timeout::from_freedesktop(-1).unwrap(), Timeout::Default);
    assert_eq!(Timeout::from_freedesktop(0).unwrap(), Timeout::Never);
    assert_eq!(
        Timeout::from_freedesktop(250).unwrap(),
        Timeout::Milliseconds(250)
    );
    assert!(Timeout::from_freedesktop(-2).is_err());
}

#[test]
fn expiration_leaves_history_and_actions_close_unless_persistent() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let posted = server
        .post(
            portal_request("org.example.Chat", "one", "Message"),
            Time(0),
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert!(server.expire(Time(4_999)).is_empty());
    assert_eq!(
        server.expire(Time(5_000)),
        vec![Closed {
            id: posted.id,
            reason: CloseReason::Expired
        }]
    );
    assert_eq!(server.history().count(), 1);

    let second = server
        .post(
            portal_request("org.example.Chat", "two", "Reply"),
            Time(8_000),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let (invocation, closed) = server.invoke(second.id, "app.reply").unwrap();
    assert_eq!(invocation.action_id, "app.reply");
    assert_eq!(closed.unwrap().reason, CloseReason::ActionInvoked);
    assert_eq!(server.history().count(), 1);
}

#[test]
fn persistent_notification_rejects_user_dismissal_but_allows_sender_withdrawal() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let mut request = portal_request("org.example.Chat", "one", "Ongoing call");
    request.display.persistent = true;
    let app_id = request.source.app_id().clone();
    let posted = server
        .post(request, Time(0), DeliveryPolicy::default())
        .unwrap();
    assert_eq!(
        server.dismiss(posted.id),
        Err(ServerError::PersistentNotification)
    );
    assert_eq!(
        server.withdraw(&app_id, posted.id).unwrap().reason,
        CloseReason::Withdrawn
    );
}

#[test]
fn validation_rejects_ambiguous_or_unbounded_input() {
    let mut request = portal_request("org.example.Chat", "one", "Message");
    for index in 0..MAX_ACTIONS {
        request
            .actions
            .push(Action::new(format!("app.extra-{index}"), "Extra", None).unwrap());
    }
    assert_eq!(request.validate().unwrap_err().problem, Problem::TooMany);
    request.actions.truncate(1);
    request.display.transient = true;
    request.display.tray_only = true;
    assert_eq!(request.validate().unwrap_err().problem, Problem::Conflict);
    assert!(Content::new("ok", "x".repeat(MAX_BODY_BYTES + 1)).is_err());
}

#[test]
fn debug_output_redacts_user_content_and_action_targets() {
    let target = ActionTarget::new("s", b"secret target".to_vec()).unwrap();
    let mut request = portal_request(
        "org.example.SecretChat8472",
        "secret-external-8472",
        "Secret title",
    );
    request.content = Content::new("Secret title", "Secret body").unwrap();
    request.actions = vec![Action::new("app.reply", "Secret label", Some(target)).unwrap()];
    let debug = format!("{request:?}");
    assert!(!debug.contains("Secret title"));
    assert!(!debug.contains("Secret body"));
    assert!(!debug.contains("Secret label"));
    assert!(!debug.contains("secret target"));
    assert!(!debug.contains("org.example.SecretChat8472"));
    assert!(!debug.contains("secret-external-8472"));
}

#[test]
fn repeated_portal_actions_keep_the_selected_target() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    let mut request = portal_request("org.example.Chat", "one", "Message");
    request.actions = vec![
        Action::new(
            "app.open",
            "First",
            Some(ActionTarget::new("s", b"first".to_vec()).unwrap()),
        )
        .unwrap(),
        Action::new(
            "app.open",
            "Second",
            Some(ActionTarget::new("s", b"second".to_vec()).unwrap()),
        )
        .unwrap()
        .with_purpose(protocol::DOCUMENT_OPEN_PURPOSE)
        .unwrap(),
    ];
    let posted = server
        .post(request, Time(0), DeliveryPolicy::default())
        .unwrap();
    let (invocation, _) = server.invoke_button(posted.id, 1).unwrap();
    assert_eq!(invocation.target.unwrap().bytes(), b"second");
    assert_eq!(
        invocation.purpose.as_deref(),
        Some(protocol::DOCUMENT_OPEN_PURPOSE)
    );
}

#[test]
fn externally_retained_ids_are_never_reallocated_after_restart() {
    let mut server = Server::new(10, TimeoutPolicy::default());
    server.reserve_ids([
        NotificationId::from_protocol(1).unwrap(),
        NotificationId::from_protocol(u32::MAX).unwrap(),
    ]);
    assert_eq!(
        server
            .post(
                portal_request("org.example.Chat", "one", "Message"),
                Time(0),
                DeliveryPolicy::default(),
            )
            .unwrap()
            .id
            .get(),
        2
    );
    server.next_id = u32::MAX;
    assert_eq!(
        server
            .post(
                portal_request("org.example.Chat", "two", "Message"),
                Time(1),
                DeliveryPolicy::default(),
            )
            .unwrap()
            .id
            .get(),
        3
    );
}

#[test]
fn bounded_history_and_indicator_are_deterministic() {
    let mut server = Server::new(2, TimeoutPolicy::default());
    for index in 0..3 {
        let mut request = portal_request("org.example.Chat", &format!("id-{index}"), "Message");
        if index == 2 {
            request.priority = Priority::Urgent;
        }
        server
            .post(request, Time(index), DeliveryPolicy::default())
            .unwrap();
    }
    assert_eq!(server.history().count(), 2);
    assert_eq!(
        server.indicator(),
        Indicator {
            unread_count: 2,
            has_urgent: true
        }
    );
    server.mark_all_read();
    assert_eq!(server.indicator(), Indicator::default());
}

fn legacy_request(app: &str, title: &str) -> Request {
    let mut request = portal_request(app, "unused", title);
    request.source = Source::Freedesktop {
        app_id: AppId::parse(app).unwrap(),
    };
    request
}

#[test]
fn one_sender_cannot_grow_live_notifications_without_bound() {
    let mut server = Server::new(500, TimeoutPolicy::default());
    let mut ids = Vec::new();
    for index in 0..MAX_ACTIVE_PER_APP {
        let outcome = server
            .post(
                legacy_request(":1.42", &format!("n{index}")),
                Time(index as u64),
                DeliveryPolicy::default(),
            )
            .unwrap();
        assert!(server.take_evictions().is_empty());
        ids.push(outcome.id);
    }
    let outcome = server
        .post(
            legacy_request(":1.42", "one more"),
            Time(10_000),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let evictions = server.take_evictions();
    assert_eq!(evictions.len(), 1);
    assert_eq!(evictions[0].closed.id, ids[0]);
    assert_eq!(evictions[0].closed.reason, CloseReason::Expired);
    assert!(matches!(evictions[0].source, Source::Freedesktop { .. }));
    assert!(!server.active.contains_key(&ids[0]));
    assert!(server.active.contains_key(&outcome.id));
    assert_eq!(server.active.len(), MAX_ACTIVE_PER_APP);

    // Another sender is untouched by the first one's cap.
    server
        .post(
            legacy_request(":1.43", "other"),
            Time(10_001),
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert!(server.take_evictions().is_empty());
    assert_eq!(server.active.len(), MAX_ACTIVE_PER_APP + 1);
}

#[test]
fn eviction_prefers_hidden_banners_and_never_closes_urgent_or_persistent() {
    let mut server = Server::new(500, TimeoutPolicy::default());
    let mut urgent = legacy_request(":1.7", "urgent");
    urgent.priority = Priority::Urgent;
    let urgent = server
        .post(urgent, Time(0), DeliveryPolicy::default())
        .unwrap()
        .id;
    let mut persistent = legacy_request(":1.7", "persistent");
    persistent.display.persistent = true;
    let persistent = server
        .post(persistent, Time(1), DeliveryPolicy::default())
        .unwrap()
        .id;
    let mut visible = Vec::new();
    for index in 2..MAX_ACTIVE_PER_APP as u64 {
        visible.push(
            server
                .post(
                    legacy_request(":1.7", "visible"),
                    Time(index),
                    DeliveryPolicy::default(),
                )
                .unwrap()
                .id,
        );
    }
    // The newest banner has already left the screen.
    let hidden = *visible.last().unwrap();
    server.expire_one(hidden).unwrap();
    assert!(server.active.contains_key(&hidden));

    server
        .post(
            legacy_request(":1.7", "next"),
            Time(1_000),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let evictions = server.take_evictions();
    assert_eq!(evictions.len(), 1);
    assert_eq!(evictions[0].closed.id, hidden);

    server
        .post(
            legacy_request(":1.7", "after"),
            Time(1_001),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let evictions = server.take_evictions();
    assert_eq!(evictions.len(), 1);
    assert_eq!(evictions[0].closed.id, visible[0]);
    assert!(server.active.contains_key(&urgent));
    assert!(server.active.contains_key(&persistent));
}

#[test]
fn a_sender_full_of_urgent_notifications_is_refused_without_changes() {
    let mut server = Server::new(500, TimeoutPolicy::default());
    for index in 0..MAX_ACTIVE_PER_APP as u64 {
        let mut urgent = legacy_request(":1.9", "urgent");
        urgent.priority = Priority::Urgent;
        server
            .post(urgent, Time(index), DeliveryPolicy::default())
            .unwrap();
    }
    let before = server.active.len();
    assert_eq!(
        server.post(
            legacy_request(":1.9", "more"),
            Time(1_000),
            DeliveryPolicy::default()
        ),
        Err(ServerError::TooManyNotifications)
    );
    assert_eq!(server.active.len(), before);
    assert!(server.take_evictions().is_empty());
}

#[test]
fn replacing_at_the_cap_does_not_evict() {
    let mut server = Server::new(500, TimeoutPolicy::default());
    for index in 0..MAX_ACTIVE_PER_APP {
        server
            .post(
                portal_request("org.example.Chat", &format!("m{index}"), "t"),
                Time(index as u64),
                DeliveryPolicy::default(),
            )
            .unwrap();
    }
    let outcome = server
        .post(
            portal_request("org.example.Chat", "m5", "updated"),
            Time(1_000),
            DeliveryPolicy::default(),
        )
        .unwrap();
    assert_eq!(outcome.kind, PostKind::Replaced);
    assert!(server.take_evictions().is_empty());
    assert_eq!(server.active.len(), MAX_ACTIVE_PER_APP);
}

#[test]
fn many_senders_cannot_exceed_the_global_live_bound() {
    let mut server = Server::new(0, TimeoutPolicy::default());
    for index in 0..MAX_ACTIVE {
        server
            .post(
                legacy_request(&format!(":1.{index}"), "t"),
                Time(index as u64),
                DeliveryPolicy::default(),
            )
            .unwrap();
    }
    assert_eq!(server.active.len(), MAX_ACTIVE);
    server
        .post(
            legacy_request(":2.1", "t"),
            Time(5_000),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let evictions = server.take_evictions();
    assert_eq!(evictions.len(), 1);
    assert_eq!(evictions[0].source.app_id().as_str(), ":1.0");
    assert_eq!(server.active.len(), MAX_ACTIVE);
}

#[test]
fn live_payload_bytes_are_bounded_per_sender() {
    let mut server = Server::new(0, TimeoutPolicy::default());
    let body = "x".repeat(MAX_BODY_BYTES);
    let target = ActionTarget::new("v", vec![0; MAX_TARGET_BYTES]).unwrap();
    let heavy = |title: &str| {
        let mut request = legacy_request(":1.5", title);
        request.content = Content::new(title, body.clone()).unwrap();
        request.actions = (0..MAX_ACTIONS)
            .map(|index| Action::new(format!("app.a{index}"), "A", Some(target.clone())).unwrap())
            .collect();
        request
    };
    let mut evicted = 0;
    for index in 0..MAX_ACTIVE_PER_APP as u64 {
        server
            .post(heavy("t"), Time(index), DeliveryPolicy::default())
            .unwrap();
        evicted += server.take_evictions().len();
    }
    assert!(evicted > 0);
    let live_bytes: usize = server.active.values().map(notification_bytes).sum();
    assert!(live_bytes <= MAX_ACTIVE_BYTES_PER_APP);
    assert!(server.active.len() < MAX_ACTIVE_PER_APP);
}

#[test]
fn history_eviction_releases_live_entries_whose_banner_is_gone() {
    let mut server = Server::new(2, TimeoutPolicy::default());
    let first = server
        .post(
            legacy_request(":1.3", "a"),
            Time(0),
            DeliveryPolicy::default(),
        )
        .unwrap()
        .id;
    server.expire_one(first).unwrap();
    assert!(server.active.contains_key(&first));
    server
        .post(
            legacy_request(":1.3", "b"),
            Time(1),
            DeliveryPolicy::default(),
        )
        .unwrap();
    server
        .post(
            legacy_request(":1.3", "c"),
            Time(2),
            DeliveryPolicy::default(),
        )
        .unwrap();
    let evictions = server.take_evictions();
    assert_eq!(evictions.len(), 1);
    assert_eq!(evictions[0].closed.id, first);
    assert!(!server.active.contains_key(&first));
}
