//! Secrets in the Windows Credential Manager (`CredWriteW` and friends).
//!
//! The Credential Manager keeps a generic credential's blob to 2560 bytes,
//! and a Microsoft access token is longer than that, so a secret is stored as
//! a small header credential (`<name>`, holding the chunk count) and one
//! credential per 2000-byte chunk (`<name>#0`, `<name>#1`, ...). Everything
//! sits under the `Lulo/` target prefix, per user and per machine, and
//! Windows encrypts it with the user's logon secret (DPAPI). No secret is
//! ever written to Lulo's own files, logs or `Debug` output.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

use crate::Error;

/// Bytes per credential chunk, well under the 2560-byte blob limit.
const CHUNK: usize = 2000;
/// A secret is never legitimately longer than this (a very long token).
const MAX_CHUNKS: usize = 16;
const HEADER: &str = "lulo-secret:";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn write_one(target: &str, blob: &[u8]) -> Result<(), Error> {
    let mut target_name = wide(target);
    let mut user_name = wide("Lulo");
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_name.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_ptr().cast_mut(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user_name.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: `credential` points at buffers that outlive the call, and
    // CredWriteW copies everything it keeps.
    unsafe { CredWriteW(&credential, 0) }.map_err(|_| Error::Unavailable)
}

fn read_one(target: &str) -> Result<Option<Vec<u8>>, Error> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated; `found` receives a buffer freed below.
    let read = unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut found) };
    match read {
        Ok(()) => {
            // SAFETY: a successful read leaves `found` pointing at a credential
            // whose blob is `CredentialBlobSize` bytes long.
            let blob = unsafe {
                let credential = &*found;
                if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(
                        credential.CredentialBlob,
                        credential.CredentialBlobSize as usize,
                    )
                    .to_vec()
                }
            };
            // SAFETY: allocated by CredReadW, freed once.
            unsafe { CredFree(found.cast()) };
            Ok(Some(blob))
        }
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => Ok(None),
        Err(_) => Err(Error::Unavailable),
    }
}

fn delete_one(target: &str) {
    let name = wide(target);
    // SAFETY: `name` is NUL-terminated and outlives the call. A missing
    // credential is the state we want.
    let _ = unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) };
}

fn chunk_target(name: &str, index: usize) -> String {
    format!("{name}#{index}")
}

fn chunk_count(name: &str) -> Result<Option<usize>, Error> {
    let Some(header) = read_one(name)? else {
        return Ok(None);
    };
    let text = String::from_utf8(header).map_err(|_| Error::InvalidResponse)?;
    let count = text
        .strip_prefix(HEADER)
        .and_then(|count| count.parse::<usize>().ok())
        .filter(|count| *count <= MAX_CHUNKS)
        .ok_or(Error::InvalidResponse)?;
    Ok(Some(count))
}

/// Store `value` under `name`, replacing any earlier value.
pub fn write_secret(name: &str, value: &str) -> Result<(), Error> {
    let bytes = value.as_bytes();
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    if chunks.len() > MAX_CHUNKS {
        return Err(Error::InvalidResponse);
    }
    let old = chunk_count(name).ok().flatten().unwrap_or(0);
    for (index, chunk) in chunks.iter().enumerate() {
        write_one(&chunk_target(name, index), chunk)?;
    }
    write_one(name, format!("{HEADER}{}", chunks.len()).as_bytes())?;
    for index in chunks.len()..old {
        delete_one(&chunk_target(name, index));
    }
    Ok(())
}

/// The secret stored under `name`, or `None` when there is none.
pub fn read_secret(name: &str) -> Result<Option<String>, Error> {
    let Some(count) = chunk_count(name)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    for index in 0..count {
        let Some(chunk) = read_one(&chunk_target(name, index))? else {
            return Err(Error::InvalidResponse);
        };
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Error::InvalidResponse)
}

/// Remove the secret and every chunk of it; absent is fine.
pub fn delete_secret(name: &str) {
    let count = chunk_count(name).ok().flatten().unwrap_or(MAX_CHUNKS);
    for index in 0..count {
        delete_one(&chunk_target(name, index));
    }
    delete_one(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(label: &str) -> String {
        format!("Lulo/test/{label}-{}", std::process::id())
    }

    #[test]
    fn a_short_secret_round_trips_and_is_deleted() {
        let name = unique("short");
        assert_eq!(read_secret(&name).unwrap(), None);
        write_secret(&name, "planted-password").unwrap();
        assert_eq!(read_secret(&name).unwrap().as_deref(), Some("planted-password"));
        write_secret(&name, "replaced").unwrap();
        assert_eq!(read_secret(&name).unwrap().as_deref(), Some("replaced"));
        delete_secret(&name);
        assert_eq!(read_secret(&name).unwrap(), None);
    }

    #[test]
    fn a_token_longer_than_one_credential_round_trips() {
        let name = unique("long");
        let long: String = (0..5_000).map(|n| char::from(b'a' + (n % 26) as u8)).collect();
        write_secret(&name, &long).unwrap();
        assert_eq!(read_secret(&name).unwrap().as_deref(), Some(long.as_str()));
        // A shorter value afterwards leaves no stale chunks behind.
        write_secret(&name, "short").unwrap();
        assert_eq!(read_secret(&name).unwrap().as_deref(), Some("short"));
        assert!(read_one(&chunk_target(&name, 1)).unwrap().is_none());
        delete_secret(&name);
        assert!(read_one(&chunk_target(&name, 0)).unwrap().is_none());
    }

    #[test]
    fn an_empty_secret_is_distinct_from_a_missing_one() {
        let name = unique("empty");
        write_secret(&name, "").unwrap();
        assert_eq!(read_secret(&name).unwrap().as_deref(), Some(""));
        delete_secret(&name);
        assert_eq!(read_secret(&name).unwrap(), None);
    }
}
