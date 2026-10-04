//! Platform-neutral user model, validation and the secret wrapper.

use std::path::PathBuf;

use zeroize::Zeroize as _;

/// AccountsService's `AccountType`: 0 standard, 1 administrator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AccountType {
    #[default]
    Standard,
    Administrator,
}

impl AccountType {
    pub fn from_dbus(value: i32) -> Self {
        if value == 1 {
            Self::Administrator
        } else {
            Self::Standard
        }
    }

    pub fn to_dbus(self) -> i32 {
        match self {
            Self::Standard => 0,
            Self::Administrator => 1,
        }
    }

    /// The Mac's wording under a user's name and in the New User pop-up.
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Administrator => "Administrator",
        }
    }

    /// The short badge the Mac shows under an administrator's name.
    pub fn badge(self) -> Option<&'static str> {
        (self == Self::Administrator).then_some("Admin")
    }
}

/// One AccountsService user, read back from the daemon.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct User {
    pub path: String,
    pub uid: u64,
    pub user_name: String,
    pub real_name: String,
    pub account_type: AccountType,
    /// The picture AccountsService stores, when it exists on disk.
    pub icon_file: Option<PathBuf>,
    pub automatic_login: bool,
    pub system_account: bool,
    pub locked: bool,
    pub password_hint: String,
}

impl User {
    /// The full name, or the account name when no full name is set.
    pub fn display_name(&self) -> &str {
        if self.real_name.trim().is_empty() {
            &self.user_name
        } else {
            &self.real_name
        }
    }

    /// One or two initials for the monogram shown without a picture.
    pub fn initials(&self) -> String {
        let mut initials: String = self
            .display_name()
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect();
        if initials.is_empty() {
            initials.push('?');
        }
        initials
    }
}

/// Sort the users the way the Mac's list reads: the signed-in user first,
/// then everyone else by name. System accounts are dropped.
pub fn people(mut users: Vec<User>, current_uid: u64) -> Vec<User> {
    users.retain(|user| !user.system_account);
    users.sort_by(|left, right| {
        (left.uid != current_uid)
            .cmp(&(right.uid != current_uid))
            .then_with(|| {
                left.display_name()
                    .to_lowercase()
                    .cmp(&right.display_name().to_lowercase())
            })
            .then_with(|| left.uid.cmp(&right.uid))
    });
    users
}

/// A change AccountsService announced (UserAdded/UserDeleted/Changed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserChange {
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameProblem {
    Empty,
    TooLong,
    /// Must start with a lowercase letter or underscore and contain only
    /// lowercase letters, digits, hyphens and underscores.
    Characters,
    Reserved,
}

impl NameProblem {
    pub fn message(self) -> &'static str {
        match self {
            NameProblem::Empty => "Enter a name.",
            NameProblem::TooLong => "The name is too long.",
            NameProblem::Characters => "The account name can contain only lowercase letters, numbers, hyphens and underscores, and must start with a letter.",
            NameProblem::Reserved => "This account name is reserved by the system. Choose a different name.",
        }
    }
}

/// `useradd`'s portable account-name rule, which AccountsService enforces
/// again on its side: `[a-z_][a-z0-9_-]*`, at most 32 bytes.
pub fn validate_user_name(name: &str) -> Result<(), NameProblem> {
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if name.len() > 32 {
        return Err(NameProblem::TooLong);
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or(' ');
    if !(first.is_ascii_lowercase() || first == '_')
        || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(NameProblem::Characters);
    }
    if matches!(
        name,
        "root"
            | "daemon"
            | "bin"
            | "sys"
            | "sync"
            | "games"
            | "man"
            | "lp"
            | "mail"
            | "news"
            | "uucp"
            | "proxy"
            | "www-data"
            | "backup"
            | "list"
            | "irc"
            | "nobody"
            | "admin"
            | "sudo"
            | "adm"
            | "lpadmin"
            | "polkitd"
            | "messagebus"
            | "systemd-network"
    ) {
        return Err(NameProblem::Reserved);
    }
    Ok(())
}

/// A full name must fit one GECOS field: no colon, comma or control
/// characters, at most 255 bytes.
pub fn validate_full_name(name: &str) -> Result<(), NameProblem> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(NameProblem::Empty);
    }
    if trimmed.len() > 255 {
        return Err(NameProblem::TooLong);
    }
    if trimmed
        .chars()
        .any(|c| c == ':' || c == ',' || c.is_control())
    {
        return Err(NameProblem::Characters);
    }
    Ok(())
}

/// The account name the Mac proposes as the full name is typed: lowercase
/// ASCII letters and digits of the words, joined without spaces.
pub fn suggest_user_name(full_name: &str) -> String {
    let mut name: String = full_name
        .chars()
        .filter_map(|c| {
            let c = c.to_ascii_lowercase();
            (c.is_ascii_lowercase() || c.is_ascii_digit()).then_some(c)
        })
        .collect();
    while name.starts_with(|c: char| c.is_ascii_digit()) {
        name.remove(0);
    }
    name.truncate(32);
    name
}

/// A password or other secret. Zeroized on drop, never printed, never
/// cloned implicitly.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

/// Why a new password was not accepted before anything ran.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasswordProblem {
    Empty,
    Mismatch,
    /// passwd and AccountsService take one line; a newline would split it.
    Characters,
    HintContainsPassword,
}

impl PasswordProblem {
    pub fn message(self) -> &'static str {
        match self {
            PasswordProblem::Empty => "Enter a password.",
            PasswordProblem::Mismatch => "The passwords you entered don’t match. Try again.",
            PasswordProblem::Characters => "The password can’t contain line breaks.",
            PasswordProblem::HintContainsPassword => {
                "The password hint can’t contain the password."
            }
        }
    }
}

/// The checks the Mac's New User and Change Password sheets make before
/// they submit: a non-empty password, typed the same twice, and a hint that
/// does not give it away.
pub fn check_new_password(
    password: &Secret,
    verify: &Secret,
    hint: &str,
) -> Result<(), PasswordProblem> {
    if password.is_empty() {
        return Err(PasswordProblem::Empty);
    }
    if password.expose() != verify.expose() {
        return Err(PasswordProblem::Mismatch);
    }
    if password.expose().contains(['\n', '\r', '\0']) {
        return Err(PasswordProblem::Characters);
    }
    if !hint.is_empty() && hint.contains(password.expose()) {
        return Err(PasswordProblem::HintContainsPassword);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: u64, name: &str, real: &str) -> User {
        User {
            uid,
            user_name: name.into(),
            real_name: real.into(),
            ..User::default()
        }
    }

    #[test]
    fn people_put_the_current_user_first_and_drop_system_accounts() {
        let mut system = user(120, "gdm", "");
        system.system_account = true;
        let users = people(
            vec![
                user(1001, "zoe", "Zoe Adams"),
                system,
                user(1002, "amy", "Amy Brown"),
                user(1000, "jacob", "Jacob Samas"),
            ],
            1000,
        );
        let names: Vec<_> = users.iter().map(|user| user.user_name.as_str()).collect();
        assert_eq!(names, ["jacob", "amy", "zoe"]);
    }

    #[test]
    fn display_name_and_initials_fall_back_to_the_account_name() {
        assert_eq!(user(1, "kim", "").display_name(), "kim");
        assert_eq!(user(1, "kim", "Kim Lee Park").initials(), "KL");
        assert_eq!(user(1, "kim", "").initials(), "K");
    }

    #[test]
    fn account_names_follow_the_useradd_rule() {
        assert!(validate_user_name("amy").is_ok());
        assert!(validate_user_name("_svc-1").is_ok());
        assert_eq!(validate_user_name(""), Err(NameProblem::Empty));
        assert_eq!(validate_user_name("Amy"), Err(NameProblem::Characters));
        assert_eq!(validate_user_name("1amy"), Err(NameProblem::Characters));
        assert_eq!(validate_user_name("a b"), Err(NameProblem::Characters));
        assert_eq!(validate_user_name("a/../b"), Err(NameProblem::Characters));
        assert_eq!(
            validate_user_name(&"a".repeat(33)),
            Err(NameProblem::TooLong)
        );
        assert_eq!(validate_user_name("root"), Err(NameProblem::Reserved));
    }

    #[test]
    fn full_names_fit_one_gecos_field() {
        assert!(validate_full_name("Amy Brown").is_ok());
        assert!(validate_full_name("Zoë O’Neill").is_ok());
        assert_eq!(validate_full_name("  "), Err(NameProblem::Empty));
        assert_eq!(validate_full_name("a:b"), Err(NameProblem::Characters));
        assert_eq!(validate_full_name("a,b"), Err(NameProblem::Characters));
        assert_eq!(validate_full_name("a\nb"), Err(NameProblem::Characters));
    }

    #[test]
    fn suggested_account_names_are_valid() {
        assert_eq!(suggest_user_name("Amy Brown"), "amybrown");
        assert_eq!(suggest_user_name("2 Zoë"), "zo");
        assert!(validate_user_name(&suggest_user_name("Amy Brown")).is_ok());
    }

    #[test]
    fn new_passwords_are_checked_before_submitting() {
        let secret = |value: &str| Secret::new(value.into());
        assert_eq!(
            check_new_password(&secret(""), &secret(""), ""),
            Err(PasswordProblem::Empty)
        );
        assert_eq!(
            check_new_password(&secret("abc"), &secret("abd"), ""),
            Err(PasswordProblem::Mismatch)
        );
        assert_eq!(
            check_new_password(&secret("a\nb"), &secret("a\nb"), ""),
            Err(PasswordProblem::Characters)
        );
        assert_eq!(
            check_new_password(&secret("tulip"), &secret("tulip"), "it is tulip"),
            Err(PasswordProblem::HintContainsPassword)
        );
        assert!(check_new_password(&secret("tulip"), &secret("tulip"), "a flower").is_ok());
    }

    #[test]
    fn secrets_never_print() {
        assert_eq!(
            format!("{:?}", Secret::new("hunter2".into())),
            "Secret(<redacted>)"
        );
    }

    #[test]
    fn account_types_round_trip() {
        assert_eq!(AccountType::from_dbus(1), AccountType::Administrator);
        assert_eq!(AccountType::from_dbus(0), AccountType::Standard);
        assert_eq!(AccountType::Administrator.to_dbus(), 1);
        assert_eq!(AccountType::Administrator.badge(), Some("Admin"));
        assert_eq!(AccountType::Standard.badge(), None);
    }
}
