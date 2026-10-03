//! Run only through scripts/behavior/run_goa_private_bus.sh. The test owns
//! a disposable session bus and cannot see the owner's GOA accounts.

#![cfg(target_os = "linux")]

use std::collections::HashMap;

use rmac_accounts::{
    model::{Service, Services},
    provider::Provider,
    Secret,
};
use rmac_accounts_linux::{goa::GoaBus, GoaApi, OAuthAccount};
use zbus::{
    blocking::{connection::Builder, Proxy},
    fdo::ObjectManager,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const PATH: &str = "/org/gnome/OnlineAccounts/Accounts/fake_1";

struct FakeManager;

#[zbus::interface(name = "org.gnome.OnlineAccounts.Manager")]
impl FakeManager {
    fn add_account(
        &self,
        provider: &str,
        identity: &str,
        _presentation: &str,
        credentials: HashMap<String, OwnedValue>,
        details: HashMap<String, String>,
    ) -> OwnedObjectPath {
        assert_eq!(provider, "google");
        assert_eq!(identity, "planted@example.com");
        assert!(credentials.contains_key("access_token"));
        assert_eq!(
            details.get("CalendarEnabled").map(String::as_str),
            Some("true")
        );
        OwnedObjectPath::try_from(PATH).unwrap()
    }
}

struct FakeAccount {
    mail_disabled: bool,
}

#[zbus::interface(name = "org.gnome.OnlineAccounts.Account")]
impl FakeAccount {
    #[zbus(property)]
    fn id(&self) -> &str {
        "fake_1"
    }
    #[zbus(property)]
    fn provider_type(&self) -> &str {
        "google"
    }
    #[zbus(property)]
    fn presentation_identity(&self) -> &str {
        "planted@example.com"
    }
    #[zbus(property)]
    fn mail_disabled(&self) -> bool {
        self.mail_disabled
    }
    #[zbus(property)]
    fn set_mail_disabled(&mut self, value: bool) {
        self.mail_disabled = value;
    }
    fn ensure_credentials(&self) -> i32 {
        3600
    }
    fn remove(&self) {}
}

struct FakeMail;
#[zbus::interface(name = "org.gnome.OnlineAccounts.Mail")]
impl FakeMail {
    #[zbus(property)]
    fn email_address(&self) -> &str {
        "planted@example.com"
    }
}

struct FakeOAuth;
#[zbus::interface(name = "org.gnome.OnlineAccounts.OAuth2Based")]
impl FakeOAuth {
    fn get_access_token(&self) -> (String, i32) {
        ("planted-token".into(), 3600)
    }
}

struct FakePassword;
#[zbus::interface(name = "org.gnome.OnlineAccounts.PasswordBased")]
impl FakePassword {
    fn get_password(&self, id: &str) -> String {
        assert_eq!(id, "imap-password");
        "planted-password".into()
    }
}

#[test]
#[ignore = "requires the private dbus-run-session wrapper"]
fn adapter_uses_goa_wire_contract_on_private_bus() {
    assert_eq!(std::env::var("RMAC_TEST_PRIVATE_BUS").as_deref(), Ok("1"));
    let _daemon = Builder::session()
        .unwrap()
        .name("org.gnome.OnlineAccounts")
        .unwrap()
        .serve_at("/org/gnome/OnlineAccounts", ObjectManager)
        .unwrap()
        .serve_at("/org/gnome/OnlineAccounts/Manager", FakeManager)
        .unwrap()
        .serve_at(
            PATH,
            FakeAccount {
                mail_disabled: false,
            },
        )
        .unwrap()
        .serve_at(PATH, FakeMail)
        .unwrap()
        .serve_at(PATH, FakeOAuth)
        .unwrap()
        .serve_at(PATH, FakePassword)
        .unwrap()
        .build()
        .unwrap();

    let client = GoaBus::session().unwrap();
    let accounts = client.accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].provider, "google");
    assert!(accounts[0].services.mail);
    let token = Secret::new("planted-token".into());
    let input = OAuthAccount {
        provider: Provider::Google,
        identity: "planted@example.com",
        presentation_identity: "planted@example.com",
        access_token: &token,
        refresh_token: None,
        expires_at: 123,
        services: Services::ALL,
    };
    assert_eq!(client.add_oauth(&input).unwrap(), PATH);
    assert_eq!(client.access_token(PATH).unwrap().expose(), "planted-token");
    assert_eq!(
        client.password(PATH, "imap-password").unwrap().expose(),
        "planted-password"
    );
    client.set_service(PATH, Service::Mail, false).unwrap();
    let proxy = Proxy::new(
        &_daemon,
        "org.gnome.OnlineAccounts",
        PATH,
        "org.gnome.OnlineAccounts.Account",
    )
    .unwrap();
    assert!(proxy.get_property::<bool>("MailDisabled").unwrap());
    client.remove(PATH).unwrap();
}
