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
        Some(Time(7_010))
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
    assert!(server.expire(Time(6_999)).is_empty());
    assert_eq!(
        server.expire(Time(7_000)),
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
