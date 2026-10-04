//! The accounts polkit will accept a password for, resolved to names.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

use zbus::zvariant::OwnedValue;

/// One account the dialog can authenticate as.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub uid: u32,
    /// The login name the helper is given.
    pub user_name: String,
    /// What the User Name field shows: the full name, or the login name.
    pub display_name: String,
}

/// A polkit identity as it crosses D-Bus: `(kind, details)`.
pub type WireIdentity = (String, HashMap<String, OwnedValue>);

/// Resolve polkit's identities to accounts, expanding groups to their
/// members, dropping duplicates and anything that does not resolve. The
/// signed-in user (`current_uid`) comes first when present, as macOS fills
/// in the signed-in administrator.
pub fn resolve(identities: &[WireIdentity], current_uid: u32) -> Vec<Identity> {
    let mut uids = Vec::new();
    for (kind, details) in identities {
        match kind.as_str() {
            "unix-user" => {
                if let Some(uid) = details.get("uid").and_then(number) {
                    uids.push(uid);
                }
            }
            "unix-group" => {
                if let Some(gid) = details.get("gid").and_then(number) {
                    uids.extend(group_members(gid));
                }
            }
            _ => {}
        }
    }
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for uid in uids {
        if seen.contains(&uid) {
            continue;
        }
        seen.push(uid);
        if let Some(identity) = user_by_uid(uid) {
            out.push(identity);
        }
    }
    order(&mut out, current_uid);
    out
}

/// polkit sends ids as `u`; accept a non-negative `i` too.
fn number(value: &OwnedValue) -> Option<u32> {
    value.downcast_ref::<u32>().ok().or_else(|| {
        value
            .downcast_ref::<i32>()
            .ok()
            .and_then(|id| u32::try_from(id).ok())
    })
}

/// Put the signed-in user first, keeping polkit's order otherwise.
pub fn order(identities: &mut [Identity], current_uid: u32) {
    if let Some(position) = identities.iter().position(|id| id.uid == current_uid) {
        identities[..=position].rotate_right(1);
    }
}

fn identity_from_passwd(entry: &libc::passwd) -> Option<Identity> {
    if entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: getpw*_r filled `entry` with NUL-terminated strings that live
    // in the caller's buffer for the duration of this call.
    let user_name = unsafe { CStr::from_ptr(entry.pw_name) }
        .to_str()
        .ok()?
        .to_owned();
    if user_name.is_empty() || user_name.contains(['\n', '\0']) {
        return None;
    }
    let gecos = if entry.pw_gecos.is_null() {
        String::new()
    } else {
        // SAFETY: as above.
        unsafe { CStr::from_ptr(entry.pw_gecos) }
            .to_string_lossy()
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let display_name = crate::text::display(&gecos, 64);
    Some(Identity {
        uid: entry.pw_uid,
        display_name: if display_name.is_empty() {
            user_name.clone()
        } else {
            display_name
        },
        user_name,
    })
}

pub fn user_by_uid(uid: u32) -> Option<Identity> {
    let mut buffer = vec![0_u8; 16 * 1024];
    // SAFETY: zeroed passwd is a valid out-parameter for getpwuid_r.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call and the buffer length is
    // its real size.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        return None;
    }
    identity_from_passwd(&entry)
}

fn user_by_name(name: &CStr) -> Option<Identity> {
    let mut buffer = vec![0_u8; 16 * 1024];
    // SAFETY: as in `user_by_uid`.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: as in `user_by_uid`.
    let status = unsafe {
        libc::getpwnam_r(
            name.as_ptr(),
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        return None;
    }
    identity_from_passwd(&entry)
}

fn group_members(gid: u32) -> Vec<u32> {
    let mut buffer = vec![0_u8; 64 * 1024];
    // SAFETY: zeroed group is a valid out-parameter for getgrgid_r.
    let mut entry: libc::group = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call and the buffer length is
    // its real size.
    let status = unsafe {
        libc::getgrgid_r(
            gid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.gr_mem.is_null() {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut cursor = entry.gr_mem;
    // SAFETY: gr_mem is a NULL-terminated array of NUL-terminated strings in
    // `buffer`; at most 256 members are read.
    unsafe {
        while !(*cursor).is_null() && names.len() < 256 {
            names.push(CString::from(CStr::from_ptr(*cursor)));
            cursor = cursor.add(1);
        }
    }
    names
        .iter()
        .filter_map(|name| user_by_name(name))
        .map(|identity| identity.uid)
        .collect()
}

/// The signed-in user's uid.
pub fn current_uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(uid: u32) -> Identity {
        Identity {
            uid,
            user_name: format!("user{uid}"),
            display_name: format!("User {uid}"),
        }
    }

    #[test]
    fn the_signed_in_user_comes_first() {
        let mut list = vec![identity(1), identity(2), identity(3)];
        order(&mut list, 3);
        assert_eq!(list.iter().map(|id| id.uid).collect::<Vec<_>>(), [3, 1, 2]);
        order(&mut list, 9);
        assert_eq!(list[0].uid, 3);
    }

    #[test]
    fn resolves_the_running_user_and_skips_unknown_kinds() {
        let uid = current_uid();
        let wire: Vec<WireIdentity> = vec![
            (
                "unix-user".into(),
                HashMap::from([("uid".into(), OwnedValue::from(uid))]),
            ),
            (
                "unix-user".into(),
                HashMap::from([("uid".into(), OwnedValue::from(uid))]),
            ),
            ("unix-netgroup".into(), HashMap::new()),
        ];
        let resolved = resolve(&wire, uid);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].uid, uid);
        assert!(!resolved[0].user_name.is_empty());
    }
}
