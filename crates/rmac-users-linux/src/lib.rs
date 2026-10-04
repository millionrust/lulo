//! Local user accounts for System Settings ▸ Users & Groups and Login
//! Password.
//!
//! * [`accounts`] talks to AccountsService (`org.freedesktop.Accounts`) on
//!   the system bus. Every mutation is authorised by polkit inside
//!   AccountsService; this crate never escalates privileges itself.
//! * [`passwd`] changes the caller's own password by driving `passwd(1)`
//!   over a private pseudo-terminal, so PAM verifies the current password.
//! * [`crypt`] hashes a new account's first password (SHA-512 crypt) before
//!   it crosses D-Bus; the plain text never leaves this process.
//!
//! Secrets live in [`Secret`], which zeroizes on drop and never prints. The
//! calls here block; callers own a worker thread.

pub mod accounts;
pub mod crypt;
pub mod model;
pub mod passwd;

pub use accounts::AccountsService;
pub use model::{
    validate_full_name, validate_user_name, AccountType, NameProblem, Secret, User, UserChange,
};

/// Why an AccountsService call did not complete. Messages are for people,
/// never contain secrets, and never include raw D-Bus text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// AccountsService is not running or the system bus is unreachable.
    Unavailable,
    /// polkit refused the change (no administrator authorisation).
    NotAuthorized,
    /// `CreateUser` named an account that already exists.
    UserExists,
    /// The user is gone (deleted elsewhere).
    UnknownUser,
    /// AccountsService accepted the call but the change failed.
    Failed,
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Error::Unavailable => "Users & Groups is unavailable. Check that AccountsService is installed and running.",
            Error::NotAuthorized => "You need an administrator’s authorisation to make this change.",
            Error::UserExists => "An account with this name already exists. Choose a different account name.",
            Error::UnknownUser => "This user no longer exists.",
            Error::Failed => "The change couldn’t be made. Try again.",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for Error {}
