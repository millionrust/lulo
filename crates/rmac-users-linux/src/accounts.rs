//! AccountsService (`org.freedesktop.Accounts`) over the system bus.
//!
//! Reads need no authorisation. Every mutation is sent with
//! `ALLOW_INTERACTIVE_AUTHORIZATION`, so polkit decides inside
//! AccountsService (and shows its password dialog when a session agent is
//! running): `change-own-user-data` for the caller's own name, picture and
//! hint; `user-administration` for creating, deleting and setting another
//! user's password; `set-login-option` for automatic login. Nothing here
//! runs as root or calls sudo.

use std::collections::HashMap;
use std::path::PathBuf;

use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::proxy::MethodFlags;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::model::{AccountType, User, UserChange};
use crate::Error;

const SERVICE: &str = "org.freedesktop.Accounts";
const ROOT: &str = "/org/freedesktop/Accounts";
const MANAGER: &str = "org.freedesktop.Accounts";
const USER: &str = "org.freedesktop.Accounts.User";

pub struct AccountsService {
    connection: Connection,
}

impl AccountsService {
    /// The system bus (`DBUS_SYSTEM_BUS_ADDRESS` when set, as in the private
    /// nested test session).
    pub fn system() -> Result<Self, Error> {
        Ok(Self {
            connection: Connection::system().map_err(|_| Error::Unavailable)?,
        })
    }

    /// A specific connection, for tests against a private bus.
    pub fn on_connection(connection: Connection) -> Self {
        Self { connection }
    }

    fn proxy<'a>(&'a self, path: &'a str, interface: &'a str) -> Result<Proxy<'a>, Error> {
        Proxy::new(&self.connection, SERVICE, path, interface).map_err(|_| Error::Unavailable)
    }

    /// Every cached (human) user AccountsService knows, read fresh.
    pub fn users(&self) -> Result<Vec<User>, Error> {
        let manager = self.proxy(ROOT, MANAGER)?;
        let paths: Vec<OwnedObjectPath> =
            manager.call("ListCachedUsers", &()).map_err(map_error)?;
        let mut users = Vec::with_capacity(paths.len());
        for path in paths {
            match self.user(path.as_str()) {
                Ok(user) => users.push(user),
                // Deleted between the list and the read.
                Err(Error::UnknownUser) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(users)
    }

    /// One user's properties.
    pub fn user(&self, path: &str) -> Result<User, Error> {
        let properties = self.proxy(path, "org.freedesktop.DBus.Properties")?;
        let values: HashMap<String, OwnedValue> =
            properties.call("GetAll", &(USER,)).map_err(map_error)?;
        Ok(user_from_properties(path, &values))
    }

    /// The object path of the user with `uid`.
    pub fn find_user(&self, uid: u64) -> Result<String, Error> {
        let manager = self.proxy(ROOT, MANAGER)?;
        let uid = i64::try_from(uid).map_err(|_| Error::UnknownUser)?;
        let path: OwnedObjectPath = manager.call("FindUserById", &(uid,)).map_err(map_error)?;
        Ok(path.as_str().to_owned())
    }

    fn mutate<B>(&self, path: &str, interface: &str, method: &str, body: &B) -> Result<(), Error>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        let proxy = self.proxy(path, interface)?;
        proxy
            .call_with_flags::<_, B, ()>(method, MethodFlags::AllowInteractiveAuth.into(), body)
            .map(|_| ())
            .map_err(map_error)
    }

    pub fn set_real_name(&self, path: &str, name: &str) -> Result<(), Error> {
        crate::model::validate_full_name(name).map_err(|_| Error::Failed)?;
        self.mutate(path, USER, "SetRealName", &(name.trim(),))
    }

    /// `file` must be a picture the caller can read; AccountsService copies
    /// it into its own store.
    pub fn set_icon_file(&self, path: &str, file: &std::path::Path) -> Result<(), Error> {
        let file = file.to_str().ok_or(Error::Failed)?;
        self.mutate(path, USER, "SetIconFile", &(file,))
    }

    pub fn set_password_hint(&self, path: &str, hint: &str) -> Result<(), Error> {
        self.mutate(path, USER, "SetPasswordHint", &(hint,))
    }

    pub fn set_automatic_login(&self, path: &str, enabled: bool) -> Result<(), Error> {
        self.mutate(path, USER, "SetAutomaticLogin", &(enabled,))
    }

    pub fn set_account_type(&self, path: &str, account_type: AccountType) -> Result<(), Error> {
        self.mutate(path, USER, "SetAccountType", &(account_type.to_dbus(),))
    }

    /// Set another user's password from an already crypted hash
    /// ([`crate::crypt::hash_password`]). Needs `user-administration`.
    pub fn set_password_hash(&self, path: &str, crypted: &str, hint: &str) -> Result<(), Error> {
        if !crypted.starts_with("$6$") {
            return Err(Error::Failed);
        }
        self.mutate(path, USER, "SetPassword", &(crypted, hint))
    }

    /// Create a user and return its object path. Needs `user-administration`.
    pub fn create_user(
        &self,
        user_name: &str,
        full_name: &str,
        account_type: AccountType,
    ) -> Result<String, Error> {
        crate::model::validate_user_name(user_name).map_err(|_| Error::Failed)?;
        crate::model::validate_full_name(full_name).map_err(|_| Error::Failed)?;
        let manager = self.proxy(ROOT, MANAGER)?;
        let path: OwnedObjectPath = manager
            .call_with_flags(
                "CreateUser",
                MethodFlags::AllowInteractiveAuth.into(),
                &(user_name, full_name.trim(), account_type.to_dbus()),
            )
            .map_err(map_error)?
            .ok_or(Error::Failed)?;
        Ok(path.as_str().to_owned())
    }

    /// Delete a user, keeping or removing their home folder. Needs
    /// `user-administration`. Refuses the caller's own account.
    pub fn delete_user(&self, uid: u64, remove_files: bool) -> Result<(), Error> {
        if uid == current_uid() || uid == 0 {
            return Err(Error::Failed);
        }
        let uid = i64::try_from(uid).map_err(|_| Error::UnknownUser)?;
        self.mutate(ROOT, MANAGER, "DeleteUser", &(uid, remove_files))
    }

    /// Block, calling `emit` whenever AccountsService announces a user was
    /// added, deleted or changed. Returns when the bus goes away.
    pub fn watch(&self, emit: &mut dyn FnMut(UserChange)) -> Result<(), Error> {
        let rule = "type='signal',sender='org.freedesktop.Accounts'";
        let iterator = MessageIterator::for_match_rule(rule, &self.connection, Some(32))
            .map_err(|_| Error::Unavailable)?;
        for message in iterator {
            if message.is_err() {
                return Err(Error::Unavailable);
            }
            emit(UserChange::Changed);
        }
        Ok(())
    }
}

/// The real user ID of this process.
pub fn current_uid() -> u64 {
    // SAFETY: getuid has no preconditions and cannot fail.
    u64::from(unsafe { libc::getuid() })
}

fn value<T>(values: &HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    T::try_from(values.get(key)?.try_clone().ok()?).ok()
}

fn user_from_properties(path: &str, values: &HashMap<String, OwnedValue>) -> User {
    let icon_file = value::<String>(values, "IconFile")
        .filter(|file| !file.is_empty())
        .map(PathBuf::from)
        .filter(|file| file.is_file());
    User {
        path: path.to_owned(),
        uid: value::<u64>(values, "Uid").unwrap_or(u64::MAX),
        user_name: value(values, "UserName").unwrap_or_default(),
        real_name: value(values, "RealName").unwrap_or_default(),
        account_type: AccountType::from_dbus(value::<i32>(values, "AccountType").unwrap_or(0)),
        icon_file,
        automatic_login: value(values, "AutomaticLogin").unwrap_or(false),
        system_account: value(values, "SystemAccount").unwrap_or(false),
        locked: value(values, "Locked").unwrap_or(false),
        password_hint: value(values, "PasswordHint").unwrap_or_default(),
    }
}

fn map_error(error: zbus::Error) -> Error {
    match &error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.Accounts.Error.PermissionDenied"
            | "org.freedesktop.DBus.Error.AccessDenied"
            | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => Error::NotAuthorized,
            "org.freedesktop.Accounts.Error.UserExists" => Error::UserExists,
            "org.freedesktop.Accounts.Error.UserDoesNotExist"
            | "org.freedesktop.DBus.Error.UnknownObject" => Error::UnknownUser,
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.NoReply" => Error::Unavailable,
            _ => Error::Failed,
        },
        zbus::Error::InputOutput(_) | zbus::Error::Address(_) => Error::Unavailable,
        _ => Error::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Str, Value};

    fn owned(value: Value<'_>) -> OwnedValue {
        value.try_to_owned().unwrap()
    }

    #[test]
    fn users_read_every_property_and_drop_missing_pictures() {
        let mut values = HashMap::new();
        values.insert("Uid".to_owned(), owned(Value::U64(1001)));
        values.insert("UserName".to_owned(), owned(Value::Str(Str::from("amy"))));
        values.insert(
            "RealName".to_owned(),
            owned(Value::Str(Str::from("Amy Brown"))),
        );
        values.insert("AccountType".to_owned(), owned(Value::I32(1)));
        values.insert(
            "IconFile".to_owned(),
            owned(Value::Str(Str::from("/nonexistent/amy.png"))),
        );
        values.insert("AutomaticLogin".to_owned(), owned(Value::Bool(true)));
        let user = user_from_properties("/org/freedesktop/Accounts/User1001", &values);
        assert_eq!(user.uid, 1001);
        assert_eq!(user.display_name(), "Amy Brown");
        assert_eq!(user.account_type, AccountType::Administrator);
        assert!(user.automatic_login);
        assert_eq!(user.icon_file, None);
    }
}
