//! The helper line protocol against a fake `polkit-agent-helper-1`, over
//! both transports polkit uses: the spawned (setuid) helper and the
//! socket-activated one.
#![cfg(unix)]

#[path = "support/fake_helper.rs"]
mod fake_helper;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rmac_polkit_agent::helper::{self, HelperConfig, HelperEvent, HelperSession};
use rmac_polkit_agent::secret::Secret;

const WAIT: Duration = Duration::from_secs(20);

fn spawned() -> HelperConfig {
    HelperConfig {
        socket: None,
        executable: Some(fake_helper::path()),
    }
}

fn start(
    config: &HelperConfig,
    user: &str,
    cookie: &str,
) -> (HelperSession, mpsc::Receiver<HelperEvent>) {
    let (events, receiver) = mpsc::channel();
    let session = helper::start(config, user, cookie, move |event| {
        let _ = events.send(event);
    })
    .expect("start the helper");
    (session, receiver)
}

fn next(receiver: &mpsc::Receiver<HelperEvent>) -> HelperEvent {
    receiver.recv_timeout(WAIT).expect("helper event")
}

fn secret(text: &str) -> Secret {
    let mut secret = Secret::new();
    assert!(secret.push_str(text));
    secret
}

#[test]
fn the_right_password_succeeds_through_the_spawned_helper() {
    let (session, events) = start(&spawned(), "jacob", "cookie-1");
    assert_eq!(next(&events), HelperEvent::Info("Hello there".into()));
    assert_eq!(
        next(&events),
        HelperEvent::Prompt {
            echo: false,
            text: "Password:".into()
        }
    );
    session.answer(secret(fake_helper::PASSWORD));
    assert_eq!(next(&events), HelperEvent::Finished(true));
}

#[test]
fn a_wrong_password_fails_with_pams_message() {
    let (session, events) = start(&spawned(), "jacob", "cookie-2");
    assert!(matches!(next(&events), HelperEvent::Info(_)));
    assert!(matches!(next(&events), HelperEvent::Prompt { .. }));
    session.answer(secret("wrong"));
    assert_eq!(next(&events), HelperEvent::Error("Sorry".into()));
    assert_eq!(next(&events), HelperEvent::Finished(false));
}

#[test]
fn the_cookie_goes_on_stdin_never_on_the_command_line() {
    // The script refuses a cookie it does not recognise and any extra
    // argument; a cookie it rejects proves the stdin path is what it reads.
    let (_session, events) = start(&spawned(), "jacob", "unexpected");
    assert_eq!(next(&events), HelperEvent::Finished(false));
}

#[test]
fn cancelling_ends_the_conversation() {
    let (session, events) = start(&spawned(), "jacob", "cookie-3");
    assert!(matches!(next(&events), HelperEvent::Info(_)));
    assert!(matches!(next(&events), HelperEvent::Prompt { .. }));
    session.cancel();
    assert_eq!(next(&events), HelperEvent::Finished(false));
    drop(session);
}

fn socket_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rmac-polkit-socket-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn the_socket_helper_gets_the_user_then_the_cookie() {
    let path = socket_path("agent-helper.socket");
    let listener = UnixListener::bind(&path).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut writer = stream;
        let mut user = String::new();
        let mut cookie = String::new();
        reader.read_line(&mut user).unwrap();
        reader.read_line(&mut cookie).unwrap();
        writer
            .write_all(b"PAM_PROMPT_ECHO_OFF Password: \n")
            .unwrap();
        let mut answer = String::new();
        reader.read_line(&mut answer).unwrap();
        let ok = answer == format!("{}\n", fake_helper::PASSWORD);
        writer
            .write_all(if ok { b"SUCCESS\n" } else { b"FAILURE\n" })
            .unwrap();
        (user, cookie)
    });
    // The socket wins over the executable when both exist.
    let config = HelperConfig {
        socket: Some(path),
        executable: Some(PathBuf::from("/nonexistent/polkit-agent-helper-1")),
    };
    let (session, events) = start(&config, "jacob", "cookie-4");
    assert!(matches!(
        next(&events),
        HelperEvent::Prompt { echo: false, .. }
    ));
    session.answer(secret(fake_helper::PASSWORD));
    assert_eq!(next(&events), HelperEvent::Finished(true));
    let (user, cookie) = server.join().unwrap();
    assert_eq!(user, "jacob\n");
    assert_eq!(cookie, "cookie-4\n");
}

#[test]
fn a_missing_socket_falls_back_to_the_spawned_helper() {
    let config = HelperConfig {
        socket: Some(socket_path("missing.socket")),
        executable: Some(fake_helper::path()),
    };
    let (session, events) = start(&config, "jacob", "cookie-5");
    assert!(matches!(next(&events), HelperEvent::Info(_)));
    assert!(matches!(next(&events), HelperEvent::Prompt { .. }));
    session.answer(secret(fake_helper::PASSWORD));
    assert_eq!(next(&events), HelperEvent::Finished(true));
}
