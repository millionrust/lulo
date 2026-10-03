//! Optional laptop probe. It only reads ObjectManager and never mutates the
//! owner's accounts or prints identities.

#![cfg(target_os = "linux")]

use rmac_accounts_linux::{goa::GoaBus, GoaApi};

#[test]
#[ignore = "read-only reference laptop GOA probe"]
fn real_goa_object_manager_read_only() {
    assert_eq!(std::env::var("RMAC_READ_ONLY_GOA_TEST").as_deref(), Ok("1"));
    let accounts = GoaBus::session().unwrap().accounts().unwrap();
    assert!(accounts.iter().all(|account| account
        .path
        .starts_with("/org/gnome/OnlineAccounts/Accounts/")));
}
