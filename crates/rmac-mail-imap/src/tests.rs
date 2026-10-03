use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig, ServerConnection, StreamOwned,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};

const CERT: &[u8] = include_bytes!("../tests/fixtures/server.cert.der");
const KEY: &[u8] = include_bytes!("../tests/fixtures/server.key.der");
const CA: &[u8] = include_bytes!("../tests/fixtures/ca.der");

fn roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(CA)).unwrap();
    roots
}

fn server_config() -> Arc<ServerConfig> {
    Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY)),
            )
            .unwrap(),
    )
}

fn fixture(
    starttls: bool,
    script: impl FnOnce(TcpStream) + Send + 'static,
) -> (Config, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        script(stream);
    });
    (
        Config {
            host: "127.0.0.1".into(),
            port,
            tls: if starttls {
                TlsMode::StartTls
            } else {
                TlsMode::Implicit
            },
            timeout: Duration::from_secs(3),
        },
        handle,
    )
}

fn read_line(stream: &mut impl Read) -> String {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            return String::from_utf8(line).unwrap();
        }
    }
}

fn send(stream: &mut impl Write, message: &str) {
    stream.write_all(message.as_bytes()).unwrap();
    stream.flush().unwrap();
}

#[test]
fn tls_plain_sync_move_uidplus_special_use_and_idle() {
    let (config, handle) = fixture(false, |socket| {
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        send(&mut stream, "* OK ready\r\n");
        assert_eq!(read_line(&mut stream), "L00000001 CAPABILITY\r\n");
        send(&mut stream, "* CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR CONDSTORE QRESYNC IDLE MOVE UIDPLUS SPECIAL-USE\r\nL00000001 OK done\r\n");
        let line = read_line(&mut stream);
        assert!(line.starts_with("L00000002 AUTHENTICATE PLAIN "));
        assert_eq!(
            STANDARD
                .decode(line.trim().split(' ').next_back().unwrap())
                .unwrap(),
            b"\0alice@example.test\0secret-password"
        );
        send(&mut stream, "L00000002 OK authenticated\r\n");
        assert_eq!(read_line(&mut stream), "L00000003 CAPABILITY\r\n");
        send(&mut stream, "* CAPABILITY IMAP4rev1 CONDSTORE QRESYNC IDLE MOVE UIDPLUS SPECIAL-USE\r\nL00000003 OK done\r\n");
        assert_eq!(read_line(&mut stream), "L00000004 ENABLE QRESYNC\r\n");
        send(&mut stream, "* ENABLED QRESYNC\r\nL00000004 OK done\r\n");
        assert_eq!(
            read_line(&mut stream),
            "L00000005 LIST \"\" \"*\" RETURN (SPECIAL-USE)\r\n"
        );
        send(&mut stream, "* LIST (\\HasNoChildren) \"/\" INBOX\r\n* LIST (\\Sent) \"/\" \"Sent Mail\"\r\nL00000005 OK done\r\n");
        assert_eq!(
            read_line(&mut stream),
            "L00000006 SELECT \"INBOX\" (QRESYNC (42 100))\r\n"
        );
        send(&mut stream, "* 2 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n* OK [UIDNEXT 9] next\r\n* OK [HIGHESTMODSEQ 105] modseq\r\n* VANISHED (EARLIER) 3:4\r\n* 1 FETCH (UID 7 FLAGS (\\Seen) MODSEQ (104))\r\nL00000006 OK done\r\n");
        assert_eq!(
            read_line(&mut stream),
            "L00000007 UID FETCH 1:* (UID FLAGS MODSEQ) (CHANGEDSINCE 100)\r\n"
        );
        send(
            &mut stream,
            "* 1 FETCH (UID 7 FLAGS (\\Seen \\Flagged) MODSEQ (105))\r\nL00000007 OK done\r\n",
        );
        assert_eq!(
            read_line(&mut stream),
            "L00000008 UID FETCH 7 (UID BODY.PEEK[])\r\n"
        );
        send(
            &mut stream,
            "* 1 FETCH (UID 7 BODY[] {12}\r\nhello\r\nworld)\r\nL00000008 OK done\r\n",
        );
        assert_eq!(
            read_line(&mut stream),
            "L00000009 UID MOVE 7 \"Archive\"\r\n"
        );
        send(&mut stream, "L00000009 OK [COPYUID 42 7 11] moved\r\n");
        assert_eq!(read_line(&mut stream), "L00000010 UID EXPUNGE 7\r\n");
        send(&mut stream, "L00000010 OK done\r\n");
        assert_eq!(read_line(&mut stream), "L00000011 IDLE\r\n");
        send(&mut stream, "+ idling\r\n* 3 EXISTS\r\n");
        assert_eq!(read_line(&mut stream), "DONE\r\n");
        send(&mut stream, "L00000011 OK done\r\n");
    });
    let mut client = Client::connect_with_roots(&config, roots()).unwrap();
    client
        .authenticate(Authentication::Plain {
            user: "alice@example.test",
            password: &Secret::new("secret-password"),
        })
        .unwrap();
    let mailboxes = client.list_mailboxes().unwrap();
    assert_eq!(mailboxes.len(), 2);
    assert_eq!(mailboxes[1].kind, MailboxKind::Sent);
    let state = client
        .select(
            "INBOX",
            Some(&SyncCursor {
                uid_validity: 42,
                highest_modseq: 100,
            }),
        )
        .unwrap();
    assert_eq!(state.exists, 2);
    assert_eq!(state.uid_validity, Some(42));
    assert_eq!(state.highest_modseq, Some(105));
    assert_eq!(state.vanished, ["3:4"]);
    assert_eq!(state.changed[0].uid, 7);
    let changes = client.fetch_changes(Some(100)).unwrap();
    assert_eq!(changes[0].flags, ["\\Seen", "\\Flagged"]);
    assert_eq!(
        client.fetch_body(7).unwrap(),
        Some(b"hello\r\nworld".to_vec())
    );
    assert_eq!(
        client.move_uids("7", "Archive").unwrap(),
        Some(CopyUid {
            uid_validity: 42,
            source_uids: "7".into(),
            destination_uids: "11".into(),
        })
    );
    client.expunge_uids("7").unwrap();
    assert_eq!(
        client.idle_once(Duration::from_secs(2)).unwrap(),
        Some("exists".into())
    );
    handle.join().unwrap();
}

#[test]
fn starttls_xoauth2_challenge_and_redaction() {
    let (config, handle) = fixture(true, |mut socket| {
        send(&mut socket, "* OK ready\r\n");
        assert_eq!(read_line(&mut socket), "L00000000 STARTTLS\r\n");
        send(&mut socket, "L00000000 OK begin TLS\r\n");
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        assert_eq!(read_line(&mut stream), "L00000001 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2\r\nL00000001 OK done\r\n",
        );
        assert_eq!(read_line(&mut stream), "L00000002 AUTHENTICATE XOAUTH2\r\n");
        send(&mut stream, "+ \r\n");
        let value = STANDARD.decode(read_line(&mut stream).trim()).unwrap();
        assert_eq!(
            value,
            b"user=alice@example.test\x01auth=Bearer planted-token\x01\x01"
        );
        send(&mut stream, "L00000002 OK done\r\n");
        assert_eq!(read_line(&mut stream), "L00000003 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1 IDLE\r\nL00000003 OK done\r\n",
        );
    });
    let secret = Secret::new("planted-token");
    assert_eq!(format!("{secret:?} {secret}"), "[redacted] [redacted]");
    let mut client = Client::connect_with_roots(&config, roots()).unwrap();
    client
        .authenticate(Authentication::XOAuth2 {
            user: "alice@example.test",
            token: &secret,
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn untrusted_certificate_is_rejected() {
    let (config, handle) = fixture(false, |socket| {
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        let _ = stream.write_all(b"* OK ready\r\n");
    });
    let result = Client::connect_with_roots(&config, rustls::RootCertStore::empty());
    assert!(result.is_err());
    handle.join().unwrap();
}

#[test]
fn oauth_rejection_does_not_expose_challenge_or_token() {
    let (config, handle) = fixture(false, |socket| {
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        send(&mut stream, "* OK ready\r\n");
        assert_eq!(read_line(&mut stream), "L00000001 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2 SASL-IR\r\nL00000001 OK done\r\n",
        );
        assert!(read_line(&mut stream).starts_with("L00000002 AUTHENTICATE XOAUTH2 "));
        send(&mut stream, "+ cGxhbnRlZC1jaGFsbGVuZ2U=\r\n");
        assert_eq!(read_line(&mut stream), "\r\n");
        send(
            &mut stream,
            "L00000002 NO planted-token planted-challenge\r\n",
        );
    });
    let mut client = Client::connect_with_roots(&config, roots()).unwrap();
    let error = client
        .authenticate(Authentication::XOAuth2 {
            user: "alice@example.test",
            token: &Secret::new("planted-token"),
        })
        .unwrap_err();
    let printed = format!("{error:?} {error}");
    assert!(!printed.contains("planted-token"));
    assert!(!printed.contains("planted-challenge"));
    handle.join().unwrap();
}

#[test]
fn tls_login_password_fallback() {
    let (config, handle) = fixture(false, |socket| {
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        send(&mut stream, "* OK ready\r\n");
        assert_eq!(read_line(&mut stream), "L00000001 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1\r\nL00000001 OK done\r\n",
        );
        assert_eq!(
            read_line(&mut stream),
            "L00000002 LOGIN \"alice\" \"secret-password\"\r\n"
        );
        send(&mut stream, "L00000002 OK done\r\n");
        assert_eq!(read_line(&mut stream), "L00000003 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1\r\nL00000003 OK done\r\n",
        );
        assert_eq!(read_line(&mut stream), "L00000004 SELECT \"INBOX\"\r\n");
        send(&mut stream, "* 1 EXISTS\r\nL00000004 OK done\r\n");
        assert_eq!(
            read_line(&mut stream),
            "L00000005 UID FETCH 1:* (UID FLAGS)\r\n"
        );
        send(
            &mut stream,
            "* 1 FETCH (UID 7 FLAGS (\\Seen))\r\nL00000005 OK done\r\n",
        );
    });
    let mut client = Client::connect_with_roots(&config, roots()).unwrap();
    client
        .authenticate(Authentication::Login {
            user: "alice",
            password: &Secret::new("secret-password"),
        })
        .unwrap();
    assert_eq!(client.select("INBOX", None).unwrap().exists, 1);
    assert_eq!(client.fetch_changes(Some(100)).unwrap()[0].modseq, None);
    handle.join().unwrap();
}

#[test]
fn oauthbearer_escapes_authorization_identity() {
    let (config, handle) = fixture(false, |socket| {
        let mut stream = StreamOwned::new(ServerConnection::new(server_config()).unwrap(), socket);
        send(&mut stream, "* OK ready\r\n");
        assert_eq!(read_line(&mut stream), "L00000001 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1 AUTH=OAUTHBEARER SASL-IR\r\nL00000001 OK done\r\n",
        );
        let line = read_line(&mut stream);
        assert!(line.starts_with("L00000002 AUTHENTICATE OAUTHBEARER "));
        let value = STANDARD
            .decode(line.trim().split(' ').next_back().unwrap())
            .unwrap();
        assert_eq!(
            value,
            b"n,a=alice=2Cwork=3D@example.test,\x01auth=Bearer token\x01\x01"
        );
        send(&mut stream, "L00000002 OK done\r\n");
        assert_eq!(read_line(&mut stream), "L00000003 CAPABILITY\r\n");
        send(
            &mut stream,
            "* CAPABILITY IMAP4rev1\r\nL00000003 OK done\r\n",
        );
    });
    let mut client = Client::connect_with_roots(&config, roots()).unwrap();
    client
        .authenticate(Authentication::OAuthBearer {
            user: "alice,work=@example.test",
            token: &Secret::new("token"),
        })
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn unsafe_mailbox_and_uid_set_are_rejected() {
    assert!(protocol::quote("INBOX\r\nEVIL").is_err());
    assert!(protocol::validate_uid_set("1\r\nEVIL").is_err());
    assert_eq!(
        protocol::quote("Sent \"Work\"").unwrap(),
        "\"Sent \\\"Work\\\"\""
    );
    assert!(validate_auth_field("alice\x01auth=Bearer attacker").is_err());
    assert!(validate_auth_field("alice\r\n").is_err());
}
