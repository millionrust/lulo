//! The signed-in Windows account, for System Settings ▸ Users & Groups.
//!
//! Windows keeps its accounts in the SAM (or a Microsoft account), and
//! creating, deleting or editing other accounts needs an administrator
//! through Windows' own Settings. Lulo's pane on Windows therefore shows
//! the person signed in — their name, account type and picture — and
//! offers no changes; every mutation here reports `Error::Unavailable`.

use std::path::PathBuf;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::NetworkManagement::NetManagement::{
    NetApiBufferFree, NetUserGetInfo, USER_INFO_2, USER_PRIV_ADMIN,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::WindowsProgramming::GetUserNameW;

use crate::model::{AccountType, User, UserChange};
use crate::Error;

/// The one account this pane lists on Windows.
const CURRENT_PATH: &str = "windows:current-user";
/// Windows has no numeric user id; the signed-in person is always 0 here.
const CURRENT_UID: u64 = 0;

pub struct AccountsService;

impl AccountsService {
    pub fn system() -> Result<Self, Error> {
        Ok(Self)
    }

    /// The signed-in account only.
    pub fn users(&self) -> Result<Vec<User>, Error> {
        Ok(vec![current_user()?])
    }

    pub fn user(&self, path: &str) -> Result<User, Error> {
        if path == CURRENT_PATH {
            current_user()
        } else {
            Err(Error::UnknownUser)
        }
    }

    pub fn find_user(&self, uid: u64) -> Result<String, Error> {
        if uid == CURRENT_UID {
            Ok(CURRENT_PATH.to_owned())
        } else {
            Err(Error::UnknownUser)
        }
    }

    pub fn set_real_name(&self, _path: &str, _name: &str) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn set_icon_file(&self, _path: &str, _file: &std::path::Path) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn set_password_hint(&self, _path: &str, _hint: &str) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn set_automatic_login(&self, _path: &str, _enabled: bool) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn set_account_type(&self, _path: &str, _account_type: AccountType) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn set_password_hash(&self, _path: &str, _crypted: &str, _hint: &str) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    pub fn create_user(
        &self,
        _user_name: &str,
        _real_name: &str,
        _account_type: AccountType,
    ) -> Result<String, Error> {
        Err(Error::Unavailable)
    }

    pub fn delete_user(&self, _uid: u64, _remove_files: bool) -> Result<(), Error> {
        Err(Error::Unavailable)
    }

    /// Windows announces no account changes to apps; the pane reads the
    /// account each time it opens instead.
    pub fn watch(&self, _emit: &mut dyn FnMut(UserChange)) -> Result<(), Error> {
        Ok(())
    }
}

/// The signed-in person's id in [`User::uid`].
pub fn current_uid() -> u64 {
    CURRENT_UID
}

fn current_user() -> Result<User, Error> {
    let user_name = user_name().ok_or(Error::Unavailable)?;
    let (real_name, administrator) = local_account(&user_name).unwrap_or_default();
    let icon_file = user_sid().and_then(|sid| account_picture(&sid));
    Ok(User {
        path: CURRENT_PATH.to_owned(),
        uid: CURRENT_UID,
        user_name,
        real_name,
        account_type: if administrator {
            AccountType::Administrator
        } else {
            AccountType::Standard
        },
        icon_file,
        automatic_login: false,
        system_account: false,
        locked: false,
        password_hint: String::new(),
    })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The text of a NUL-terminated wide string Windows handed back.
fn from_pwstr(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: Windows returned a valid NUL-terminated string.
    unsafe { text.to_string() }.unwrap_or_default()
}

fn user_name() -> Option<String> {
    let mut buffer = [0u16; 257];
    let mut length = buffer.len() as u32;
    // SAFETY: `length` holds the buffer's capacity in characters.
    unsafe { GetUserNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut length) }.ok()?;
    let length = (length as usize).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length])).filter(|name| !name.is_empty())
}

/// The full name and whether the account is an administrator, from the
/// local account database. A Microsoft or domain account that the local
/// database does not answer for keeps its sign-in name.
fn local_account(user_name: &str) -> Option<(String, bool)> {
    let name = wide(user_name);
    let mut buffer: *mut u8 = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; Windows allocates `buffer`, which is
    // freed below.
    let status = unsafe { NetUserGetInfo(PCWSTR::null(), PCWSTR(name.as_ptr()), 2, &mut buffer) };
    if status != 0 || buffer.is_null() {
        return None;
    }
    // SAFETY: level 2 fills a USER_INFO_2.
    let info = unsafe { &*(buffer as *const USER_INFO_2) };
    let result = (
        from_pwstr(info.usri2_full_name).trim().to_owned(),
        info.usri2_priv == USER_PRIV_ADMIN,
    );
    // SAFETY: `buffer` came from NetUserGetInfo.
    unsafe { NetApiBufferFree(Some(buffer as *const _)) };
    Some(result)
}

/// This process's user SID as text (`S-1-5-21-…`).
fn user_sid() -> Option<String> {
    let mut token = HANDLE::default();
    // SAFETY: a query-only token of this process, closed below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut needed = 0u32;
    // SAFETY: the first call only asks for the size.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    // SAFETY: `buffer` is at least `needed` bytes and 8-byte aligned.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    };
    let mut text = String::new();
    if read.is_ok() {
        // SAFETY: TokenUser fills a TOKEN_USER at the buffer's start.
        let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
        let mut string = PWSTR::null();
        // SAFETY: the SID lives in `buffer`; Windows allocates `string`.
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut string) }.is_ok() {
            text = from_pwstr(string);
            // SAFETY: `string` came from ConvertSidToStringSidW.
            unsafe { LocalFree(Some(HLOCAL(string.0.cast()))) };
        }
    }
    // SAFETY: the token was opened above.
    let _ = unsafe { CloseHandle(token) };
    Some(text).filter(|text| !text.is_empty())
}

/// The picture Windows shows for this account, largest first, as Windows'
/// own account settings saved it.
fn account_picture(sid: &str) -> Option<PathBuf> {
    let key = wide(&format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\AccountPicture\Users\{sid}"
    ));
    ["Image448", "Image240", "Image192", "Image96"]
        .into_iter()
        .find_map(|value| {
            let value = wide(value);
            let mut data = [0u16; 520];
            let mut size = std::mem::size_of_val(&data) as u32;
            // SAFETY: `data` holds `size` bytes; both names are NUL-terminated.
            let status = unsafe {
                RegGetValueW(
                    HKEY_LOCAL_MACHINE,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(value.as_ptr()),
                    RRF_RT_REG_SZ,
                    None,
                    Some(data.as_mut_ptr().cast()),
                    Some(&mut size),
                )
            };
            if status.is_err() {
                return None;
            }
            let length = data
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(data.len());
            let path = PathBuf::from(String::from_utf16_lossy(&data[..length]));
            path.is_file().then_some(path)
        })
}
