//! AccountsService round trips against python-dbusmock on a private bus
//! (tests/dbusmock/accounts_service.py). Never touches the real system
//! bus. Skipped when python3-dbusmock is missing unless
//! RMAC_REQUIRE_DBUSMOCK=1 (set in CI), which turns a missing mock into a
//! failure.
#![cfg(target_os = "linux")]

#[path = "../../../tests/dbusmock/private_bus.rs"]
mod private_bus;

use rmac_users_linux::model::{people, AccountType};
use rmac_users_linux::{crypt, AccountsService, Error};

// High UIDs so no test ever names the account running it.
const PEOPLE: &str = r#"{"users": [
    {"uid": 2000, "name": "jacob", "real_name": "Jacob Samas", "admin": true},
    {"uid": 2001, "name": "amy", "real_name": "Amy Brown", "admin": false}
]}"#;

fn service() -> Option<(private_bus::MockBus, AccountsService)> {
    let bus =
        private_bus::MockBus::start("accounts_service.py", "org.freedesktop.Accounts", PEOPLE)?;
    let service = AccountsService::on_connection(bus.connection());
    Some((bus, service))
}

#[test]
fn lists_people_with_names_and_admin_badges() {
    let Some((_bus, service)) = service() else {
        return;
    };
    let users = people(service.users().unwrap(), 2001);
    let names: Vec<_> = users.iter().map(|user| user.display_name()).collect();
    assert_eq!(names, ["Amy Brown", "Jacob Samas"]);
    assert_eq!(users[1].account_type, AccountType::Administrator);
    assert_eq!(users[1].account_type.badge(), Some("Admin"));
    assert_eq!(users[0].account_type.badge(), None);
}

#[test]
fn changes_a_full_name_picture_and_hint() {
    let Some((_bus, service)) = service() else {
        return;
    };
    let path = service.find_user(2000).unwrap();
    service.set_real_name(&path, "Jacob S.").unwrap();
    service.set_password_hint(&path, "the usual").unwrap();
    let picture =
        std::env::temp_dir().join(format!("rmac-users-picture-{}.png", std::process::id()));
    std::fs::write(&picture, b"png").unwrap();
    service.set_icon_file(&path, &picture).unwrap();
    let user = service.user(&path).unwrap();
    assert_eq!(user.real_name, "Jacob S.");
    assert_eq!(user.password_hint, "the usual");
    assert_eq!(user.icon_file.as_deref(), Some(picture.as_path()));
    let _ = std::fs::remove_file(picture);
    // A full name that would break /etc/passwd never reaches the bus.
    assert_eq!(service.set_real_name(&path, "a:b"), Err(Error::Failed));
}

#[test]
fn creates_a_user_with_a_hashed_password_and_deletes_it() {
    let Some((bus, service)) = service() else {
        return;
    };
    let path = service
        .create_user("kim", "Kim Lee", AccountType::Standard)
        .unwrap();
    let hash = crypt::hash_password("first password").unwrap();
    service.set_password_hash(&path, &hash, "a hint").unwrap();
    let stored: String = bus.user_property(&path, "MockPasswordHash");
    assert!(stored.starts_with("$6$"));
    assert!(!stored.contains("first password"));
    let user = service.user(&path).unwrap();
    assert_eq!(user.user_name, "kim");
    assert_eq!(user.account_type, AccountType::Standard);

    assert_eq!(
        service.create_user("kim", "Kim Again", AccountType::Standard),
        Err(Error::UserExists)
    );
    // Plain text is refused before it reaches the bus.
    assert_eq!(
        service.set_password_hash(&path, "first password", ""),
        Err(Error::Failed)
    );

    service.delete_user(user.uid, true).unwrap();
    assert!(service
        .users()
        .unwrap()
        .iter()
        .all(|other| other.uid != user.uid));
    let deleted: Vec<(i64, bool)> = bus.call_mock("GetDeletedUsers");
    assert_eq!(deleted, [(i64::try_from(user.uid).unwrap(), true)]);
}

#[test]
fn automatic_login_moves_between_users() {
    let Some((_bus, service)) = service() else {
        return;
    };
    let jacob = service.find_user(2000).unwrap();
    let amy = service.find_user(2001).unwrap();
    service.set_automatic_login(&jacob, true).unwrap();
    service.set_automatic_login(&amy, true).unwrap();
    assert!(!service.user(&jacob).unwrap().automatic_login);
    assert!(service.user(&amy).unwrap().automatic_login);
    service.set_automatic_login(&amy, false).unwrap();
    assert!(!service.user(&amy).unwrap().automatic_login);
}

#[test]
fn polkit_refusal_is_reported_and_changes_nothing() {
    let Some((bus, service)) = service() else {
        return;
    };
    bus.call_mock_with::<_, ()>("SetAuthorized", &(false,));
    let path = service.find_user(2001).unwrap();
    assert_eq!(
        service.create_user("lee", "Lee", AccountType::Administrator),
        Err(Error::NotAuthorized)
    );
    assert_eq!(service.delete_user(2001, false), Err(Error::NotAuthorized));
    assert_eq!(
        service.set_automatic_login(&path, true),
        Err(Error::NotAuthorized)
    );
    assert_eq!(service.users().unwrap().len(), 2);
}
