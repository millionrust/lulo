//! SHA-512 crypt (`$6$`, Ulrich Drepper's specification), the scheme
//! AccountsService's `SetPassword` expects and every glibc/libxcrypt system
//! verifies. Only a new account's first password goes through here; the
//! caller's own password changes through `passwd(1)` so PAM checks the old
//! one.

use sha2::{Digest as _, Sha512};
use zeroize::Zeroizing;

const ITOA64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const DEFAULT_ROUNDS: u32 = 5000;
const SALT_LEN: usize = 16;

/// Hash `password` with a fresh 16-character random salt and the default
/// 5000 rounds.
pub fn hash_password(password: &str) -> Result<String, crate::Error> {
    let mut random = Zeroizing::new([0_u8; SALT_LEN]);
    getrandom::fill(&mut random[..]).map_err(|_| crate::Error::Failed)?;
    let salt: String = random
        .iter()
        .map(|byte| ITOA64[usize::from(byte & 0x3f)] as char)
        .collect();
    Ok(sha512_crypt(password.as_bytes(), salt.as_bytes(), None))
}

/// `crypt(3)` with a `$6$[rounds=N$]salt` setting. `rounds` of `None` uses
/// the implicit default and leaves it out of the result.
pub fn sha512_crypt(key: &[u8], salt: &[u8], rounds: Option<u32>) -> String {
    let salt = &salt[..salt.len().min(SALT_LEN)];
    let round_count = rounds.unwrap_or(DEFAULT_ROUNDS).clamp(1000, 999_999_999);

    let mut alternate = Sha512::new();
    alternate.update(key);
    alternate.update(salt);
    alternate.update(key);
    let alternate = finish(alternate);

    let mut context = Sha512::new();
    context.update(key);
    context.update(salt);
    let mut remaining = key.len();
    while remaining > 64 {
        context.update(&alternate[..]);
        remaining -= 64;
    }
    context.update(&alternate[..remaining]);
    let mut bits = key.len();
    while bits > 0 {
        if bits & 1 != 0 {
            context.update(&alternate[..]);
        } else {
            context.update(key);
        }
        bits >>= 1;
    }
    let mut digest = finish(context);

    let mut p_context = Sha512::new();
    for _ in 0..key.len() {
        p_context.update(key);
    }
    let p_digest = finish(p_context);
    let p_bytes = Zeroizing::new(repeat_to(&p_digest[..], key.len()));

    let mut s_context = Sha512::new();
    for _ in 0..16 + usize::from(digest[0]) {
        s_context.update(salt);
    }
    let s_digest = finish(s_context);
    let s_bytes = repeat_to(&s_digest[..], salt.len());

    for round in 0..round_count {
        let mut context = Sha512::new();
        if round & 1 != 0 {
            context.update(&p_bytes[..]);
        } else {
            context.update(&digest[..]);
        }
        if round % 3 != 0 {
            context.update(&s_bytes);
        }
        if round % 7 != 0 {
            context.update(&p_bytes[..]);
        }
        if round & 1 != 0 {
            context.update(&digest[..]);
        } else {
            context.update(&p_bytes[..]);
        }
        digest = finish(context);
    }

    let mut out = String::from("$6$");
    if let Some(rounds) = rounds {
        out.push_str(&format!("rounds={}$", rounds.clamp(1000, 999_999_999)));
    }
    out.push_str(&String::from_utf8_lossy(salt));
    out.push('$');
    for index in 0..21 {
        let (a, b, c) = (index, index + 21, index + 42);
        let (first, second, third) = match index % 3 {
            0 => (a, b, c),
            1 => (b, c, a),
            _ => (c, a, b),
        };
        push_base64(&mut out, digest[first], digest[second], digest[third], 4);
    }
    push_base64(&mut out, 0, 0, digest[63], 2);
    out
}

fn finish(context: Sha512) -> Zeroizing<[u8; 64]> {
    let mut out = Zeroizing::new([0_u8; 64]);
    out.copy_from_slice(&context.finalize());
    out
}

fn repeat_to(block: &[u8], len: usize) -> Vec<u8> {
    block.iter().copied().cycle().take(len).collect()
}

fn push_base64(out: &mut String, high: u8, middle: u8, low: u8, count: usize) {
    let mut word = (u32::from(high) << 16) | (u32::from(middle) << 8) | u32::from(low);
    for _ in 0..count {
        out.push(ITOA64[(word & 0x3f) as usize] as char);
        word >>= 6;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from `openssl passwd -6 -salt <salt> <password>`.
    #[test]
    fn matches_the_reference_implementation() {
        assert_eq!(
            sha512_crypt(b"Hello world!", b"saltstring", None),
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );
        assert_eq!(
            sha512_crypt(b"Hello world!", b"saltstringsaltstring", None),
            "$6$saltstringsaltst$e.3mR68CqZEpesEX1HlFZT6sEanSOjM/b5UoDyDo00a8syek2cJldMjrbtKP86.FJvzluVR7nc3DNzelAwTxj."
        );
        assert_eq!(
            sha512_crypt(
                b"a-password-longer-than-sixty-four-bytes-0123456789-0123456789-0123456789",
                b"xyz",
                None
            ),
            "$6$xyz$uPqZUHAP0ENu7xUtw6.mwMJY5D5C5CpqonwyoHnnubjgrcZNPmmydlDAlNoOX7kINl4VmSYX4EObontoJdqRt."
        );
    }

    #[test]
    fn fresh_hashes_use_a_random_salt_and_never_contain_the_password() {
        let first = hash_password("correct horse").unwrap();
        let second = hash_password("correct horse").unwrap();
        assert!(first.starts_with("$6$"));
        assert_eq!(first.split('$').nth(2).map(str::len), Some(16));
        assert_ne!(first, second);
        assert!(!first.contains("correct"));
        let salt = first.split('$').nth(2).unwrap();
        assert_eq!(sha512_crypt(b"correct horse", salt.as_bytes(), None), first);
    }
}
