//! The agent against a python-dbusmock polkitd on a private bus
//! (tests/dbusmock/polkit_authority.py): registration for the session,
//! BeginAuthentication through the fake helper, CancelAuthentication,
//! queueing, three wrong passwords, and refusing callers other than
//! polkitd. Never touches the real system bus. Skipped when
//! python3-dbusmock is missing unless RMAC_REQUIRE_DBUSMOCK=1 (set in CI).
#![cfg(target_os = "linux")]

#[path = "support/fake_helper.rs"]
mod fake_helper;
#[path = "../../../tests/dbusmock/private_bus.rs"]
mod private_bus;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmac_polkit_agent::dbus::{self as agent_dbus, AGENT_PATH};
use rmac_polkit_agent::helper::HelperConfig;
use rmac_polkit_agent::identity::{Identity, WireIdentity};
use rmac_polkit_agent::request::{Coordinator, Environment, FromUi, ToUi};
use rmac_polkit_agent::secret::Secret;

const WAIT: Duration = Duration::from_secs(30);

struct Fake;

impl Environment for Fake {
    fn current_uid(&self) -> u32 {
        2000
    }
    fn resolve(&self, identities: &[WireIdentity], _: u32) -> Vec<Identity> {
        identities
            .iter()
            .filter_map(|(kind, details)| {
                let uid = details.get("uid")?.downcast_ref::<u32>().ok()?;
                (kind == "unix-user").then(|| Identity {
                    uid,
                    user_name: format!("user{uid}"),
                    display_name: format!("User {uid}"),
                })
            })
            .collect()
    }
    fn executable_of(&self, _: u32) -> Option<String> {
        Some("/usr/libexec/rmac/rmac-system-settings".into())
    }
}

fn secret(text: &str) -> Secret {
    let mut secret = Secret::new();
    assert!(secret.push_str(text));
    secret
}

/// A scripted dialog: `ok-*` types the right password, `bad-*` a wrong one
/// every time it is asked, anything else waits. It logs every request it is
/// shown, in order.
fn fake_dialog(rx: async_channel::Receiver<ToUi>, shown: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        let mut responders = HashMap::new();
        while let Ok(message) = rx.recv_blocking() {
            match message {
                ToUi::Open { dialog, responder } => {
                    assert_eq!(dialog.app_name, "System Settings");
                    assert_eq!(dialog.confirm_label, "Modify Settings");
                    shown.lock().unwrap().push(dialog.cookie.clone());
                    let password = if dialog.cookie.starts_with("ok-") {
                        Some(fake_helper::PASSWORD)
                    } else if dialog.cookie.starts_with("bad-") {
                        Some("wrong")
                    } else {
                        None
                    };
                    if let Some(password) = password {
                        responder.send(FromUi::Submit {
                            identity: 0,
                            secret: secret(password),
                        });
                    }
                    responders.insert(dialog.cookie, responder);
                }
                ToUi::Prompt { cookie, .. } if cookie.starts_with("bad-") => {
                    if let Some(responder) = responders.get(&cookie) {
                        responder.send(FromUi::Submit {
                            identity: 0,
                            secret: secret("wrong"),
                        });
                    }
                }
                _ => {}
            }
        }
    });
}

struct Harness {
    bus: private_bus::MockBus,
    agent_name: String,
    shown: Arc<Mutex<Vec<String>>>,
}

fn harness() -> Option<Harness> {
    let bus =
        private_bus::MockBus::start("polkit_authority.py", "org.freedesktop.PolicyKit1", "{}")?;
    let (ui, ui_rx) = async_channel::unbounded();
    let shown = Arc::new(Mutex::new(Vec::new()));
    fake_dialog(ui_rx, shown.clone());
    let coordinator = Coordinator::new(
        ui,
        HelperConfig {
            socket: None,
            executable: Some(fake_helper::path()),
        },
        Box::new(Fake),
    );
    let address = bus.address().to_owned();
    let (name_tx, name_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        async_io::block_on(async move {
            let connection = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .build()
                .await
                .unwrap();
            name_tx
                .send(connection.unique_name().unwrap().to_string())
                .unwrap();
            let _ = agent_dbus::serve(&connection, agent_dbus::session_subject("c7"), coordinator)
                .await;
        });
    });
    let agent_name = name_rx.recv_timeout(WAIT).expect("agent connected");
    let harness = Harness {
        bus,
        agent_name,
        shown,
    };
    harness.wait(|| !harness.registrations().is_empty());
    Some(harness)
}

impl Harness {
    fn wait(&self, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !ready() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn registrations(&self) -> Vec<(String, String, String, String)> {
        self.bus.call_mock("GetRegistrations")
    }

    fn begin(&self, cookie: &str) {
        let details = HashMap::from([("polkit.subject-pid", "4242")]);
        self.bus.call_mock_with::<_, ()>(
            "Begin",
            &(
                self.agent_name.as_str(),
                AGENT_PATH,
                "org.freedesktop.accounts.user-administration",
                "Authentication is required to change another user’s data.",
                "system-users",
                details,
                cookie,
                vec![2001_u32],
            ),
        );
    }

    fn cancel(&self, cookie: &str) {
        self.bus
            .call_mock_with::<_, ()>("Cancel", &(self.agent_name.as_str(), AGENT_PATH, cookie));
    }

    fn result(&self, cookie: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let result: String = self.bus.call_mock_with("GetResult", &(cookie,));
            if !result.is_empty() {
                return result;
            }
            assert!(Instant::now() < deadline, "{cookie} never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn shown(&self) -> Vec<String> {
        self.shown.lock().unwrap().clone()
    }
}

#[test]
fn registers_for_the_session_and_authenticates_through_the_helper() {
    let Some(harness) = harness() else {
        return;
    };
    let registrations = harness.registrations();
    assert_eq!(registrations.len(), 1);
    let (kind, session, locale, path) = &registrations[0];
    assert_eq!(kind, "unix-session");
    assert_eq!(session, "c7");
    assert!(!locale.is_empty());
    assert_eq!(path, AGENT_PATH);

    harness.begin("ok-1");
    assert_eq!(harness.result("ok-1"), "ok");
    assert_eq!(harness.shown(), ["ok-1"]);
}

#[test]
fn cancel_authentication_dismisses_the_dialog() {
    let Some(harness) = harness() else {
        return;
    };
    harness.begin("wait-1");
    harness.wait(|| harness.shown() == ["wait-1"]);
    harness.cancel("wait-1");
    assert_eq!(
        harness.result("wait-1"),
        "org.freedesktop.PolicyKit1.Error.Cancelled"
    );
}

#[test]
fn concurrent_requests_queue_behind_the_open_dialog() {
    let Some(harness) = harness() else {
        return;
    };
    harness.begin("wait-2");
    harness.wait(|| harness.shown() == ["wait-2"]);
    harness.begin("queue-3");
    harness.begin("ok-4");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(harness.shown(), ["wait-2"], "only one dialog at a time");

    // A queued request can be cancelled before it is ever shown.
    harness.cancel("queue-3");
    assert_eq!(
        harness.result("queue-3"),
        "org.freedesktop.PolicyKit1.Error.Cancelled"
    );
    harness.cancel("wait-2");
    assert_eq!(
        harness.result("wait-2"),
        "org.freedesktop.PolicyKit1.Error.Cancelled"
    );
    assert_eq!(harness.result("ok-4"), "ok");
    assert_eq!(harness.shown(), ["wait-2", "ok-4"]);
}

#[test]
fn three_wrong_passwords_end_the_request() {
    let Some(harness) = harness() else {
        return;
    };
    harness.begin("bad-5");
    assert_eq!(
        harness.result("bad-5"),
        "org.freedesktop.PolicyKit1.Error.Failed"
    );
}

#[test]
fn only_the_authority_may_drive_the_agent() {
    let Some(harness) = harness() else {
        return;
    };
    let connection = harness.bus.connection();
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        harness.agent_name.as_str(),
        AGENT_PATH,
        "org.freedesktop.PolicyKit1.AuthenticationAgent",
    )
    .unwrap();
    let identities: Vec<WireIdentity> = Vec::new();
    let error = proxy
        .call::<_, _, ()>(
            "BeginAuthentication",
            &(
                "org.example.action",
                "spoofed",
                "",
                HashMap::<String, String>::new(),
                "ok-6",
                identities,
            ),
        )
        .unwrap_err();
    match error {
        zbus::Error::MethodError(name, _, _) => {
            assert_eq!(
                name.as_str(),
                "org.freedesktop.PolicyKit1.Error.NotAuthorized"
            );
        }
        other => panic!("unexpected error {other:?}"),
    }
    let error = proxy
        .call::<_, _, ()>("CancelAuthentication", &("ok-6",))
        .unwrap_err();
    assert!(matches!(error, zbus::Error::MethodError(..)));
    assert!(harness.shown().is_empty());
}
