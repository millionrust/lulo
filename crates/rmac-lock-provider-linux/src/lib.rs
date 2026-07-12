//! Linux lock-provider adapter primitives.
//!
//! The Wayland and PAM adapters are under construction and remain disconnected
//! from the installed lock service until their runtime evidence gates pass.
//! Secret input uses one fixed-capacity allocation, no cloning, redacted
//! diagnostics, and zeroization on backspace, clear, transfer, and drop.

use std::fmt;

use zeroize::Zeroize as _;

pub mod key_repeat;
pub mod keyboard;
pub mod paint;
pub mod pam_broker;
pub mod pam_conversation;
#[cfg(any(target_os = "linux", test))]
mod pointer;
#[cfg(any(test, all(target_os = "linux", feature = "development-provider")))]
mod process;
#[cfg(any(target_os = "linux", test))]
mod prompt_label;
pub mod runtime;
pub mod surface;

#[cfg(target_os = "linux")]
pub mod pam;

#[cfg(target_os = "linux")]
pub mod shm;

#[cfg(target_os = "linux")]
mod text_renderer;

#[cfg(target_os = "linux")]
pub mod wayland;

/// Uninstalled process entry point for the Linux recovery/evidence harness.
///
/// The feature is deliberately opt-in and the session installer does not build
/// or select it while swaylock remains the accepted provider.
#[cfg(all(target_os = "linux", feature = "development-provider"))]
#[doc(hidden)]
pub mod development_process;

#[cfg(target_os = "linux")]
mod xkb_keyboard;

#[cfg(any(target_os = "linux", test))]
mod registry_probe {
    pub const COMPOSITOR_INTERFACE: &str = "wl_compositor";
    pub const MANAGER_INTERFACE: &str = "ext_session_lock_manager_v1";
    pub const OUTPUT_INTERFACE: &str = "wl_output";
    pub const SEAT_INTERFACE: &str = "wl_seat";
    pub const SHM_INTERFACE: &str = "wl_shm";
    pub const REQUIRED_COMPOSITOR_VERSION: u32 = 4;
    pub const REQUIRED_SESSION_LOCK_VERSION: u32 = 1;
    pub const REQUIRED_SHM_VERSION: u32 = 1;
    pub const REQUIRED_SEAT_VERSION: u32 = 4;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Capabilities {
        pub compositor_version: u32,
        pub session_lock_version: u32,
        pub shm_version: u32,
        pub output_count: usize,
        pub seat_version: u32,
        pub seat_count: usize,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Error {
        CompositorUnavailable,
        CompositorVersion { advertised: u32, required: u32 },
        SessionLockUnavailable,
        SessionLockVersion { advertised: u32, required: u32 },
        ShmUnavailable,
        ShmVersion { advertised: u32, required: u32 },
        SeatUnavailable,
        SeatVersion { advertised: u32, required: u32 },
        NoOutputs,
    }

    pub fn classify<'a>(
        interfaces: impl IntoIterator<Item = (&'a str, u32)>,
    ) -> Result<Capabilities, Error> {
        let mut compositor_version = None;
        let mut manager_version = None;
        let mut shm_version = None;
        let mut output_count = 0_usize;
        let mut seat_version = None;
        let mut seat_count = 0_usize;

        for (name, version) in interfaces {
            if name == COMPOSITOR_INTERFACE {
                compositor_version =
                    Some(compositor_version.map_or(version, |current: u32| current.max(version)));
            } else if name == MANAGER_INTERFACE {
                manager_version =
                    Some(manager_version.map_or(version, |current: u32| current.max(version)));
            } else if name == OUTPUT_INTERFACE {
                output_count = output_count.saturating_add(1);
            } else if name == SHM_INTERFACE {
                shm_version =
                    Some(shm_version.map_or(version, |current: u32| current.max(version)));
            } else if name == SEAT_INTERFACE {
                seat_version =
                    Some(seat_version.map_or(version, |current: u32| current.max(version)));
                seat_count = seat_count.saturating_add(1);
            }
        }

        let compositor_version = compositor_version.ok_or(Error::CompositorUnavailable)?;
        if compositor_version < REQUIRED_COMPOSITOR_VERSION {
            return Err(Error::CompositorVersion {
                advertised: compositor_version,
                required: REQUIRED_COMPOSITOR_VERSION,
            });
        }
        let manager_version = manager_version.ok_or(Error::SessionLockUnavailable)?;
        if manager_version < REQUIRED_SESSION_LOCK_VERSION {
            return Err(Error::SessionLockVersion {
                advertised: manager_version,
                required: REQUIRED_SESSION_LOCK_VERSION,
            });
        }
        let shm_version = shm_version.ok_or(Error::ShmUnavailable)?;
        if shm_version < REQUIRED_SHM_VERSION {
            return Err(Error::ShmVersion {
                advertised: shm_version,
                required: REQUIRED_SHM_VERSION,
            });
        }
        if output_count == 0 {
            return Err(Error::NoOutputs);
        }
        let seat_version = seat_version.ok_or(Error::SeatUnavailable)?;
        if seat_version < REQUIRED_SEAT_VERSION {
            return Err(Error::SeatVersion {
                advertised: seat_version,
                required: REQUIRED_SEAT_VERSION,
            });
        }

        Ok(Capabilities {
            compositor_version,
            session_lock_version: manager_version,
            shm_version,
            output_count,
            seat_version,
            seat_count,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn accepts_version_one_with_every_observed_output() {
            let capabilities = classify([
                (COMPOSITOR_INTERFACE, 6),
                (OUTPUT_INTERFACE, 4),
                (MANAGER_INTERFACE, 1),
                (SHM_INTERFACE, 1),
                (SEAT_INTERFACE, 9),
                (OUTPUT_INTERFACE, 4),
            ])
            .unwrap();

            assert_eq!(capabilities.compositor_version, 6);
            assert_eq!(capabilities.session_lock_version, 1);
            assert_eq!(capabilities.shm_version, 1);
            assert_eq!(capabilities.output_count, 2);
            assert_eq!(capabilities.seat_version, 9);
            assert_eq!(capabilities.seat_count, 1);
        }

        #[test]
        fn rejects_a_missing_session_lock_manager() {
            assert_eq!(
                classify([
                    (COMPOSITOR_INTERFACE, 4),
                    (SHM_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                    (OUTPUT_INTERFACE, 4),
                ]),
                Err(Error::SessionLockUnavailable)
            );
        }

        #[test]
        fn rejects_an_unsupported_manager_version() {
            assert_eq!(
                classify([
                    (COMPOSITOR_INTERFACE, 4),
                    (MANAGER_INTERFACE, 0),
                    (SHM_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                    (OUTPUT_INTERFACE, 4),
                ]),
                Err(Error::SessionLockVersion {
                    advertised: 0,
                    required: 1,
                })
            );
        }

        #[test]
        fn rejects_a_headless_registry_snapshot() {
            assert_eq!(
                classify([
                    (COMPOSITOR_INTERFACE, 4),
                    (MANAGER_INTERFACE, 1),
                    (SHM_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                ]),
                Err(Error::NoOutputs)
            );
        }

        #[test]
        fn rejects_missing_or_old_rendering_authorities() {
            assert_eq!(
                classify([
                    (MANAGER_INTERFACE, 1),
                    (SHM_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                    (OUTPUT_INTERFACE, 4),
                ]),
                Err(Error::CompositorUnavailable)
            );
            assert_eq!(
                classify([
                    (COMPOSITOR_INTERFACE, 3),
                    (MANAGER_INTERFACE, 1),
                    (SHM_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                    (OUTPUT_INTERFACE, 4),
                ]),
                Err(Error::CompositorVersion {
                    advertised: 3,
                    required: 4,
                })
            );
            assert_eq!(
                classify([
                    (COMPOSITOR_INTERFACE, 4),
                    (MANAGER_INTERFACE, 1),
                    (SEAT_INTERFACE, 9),
                    (OUTPUT_INTERFACE, 4),
                ]),
                Err(Error::ShmUnavailable)
            );
        }

        #[test]
        fn rejects_missing_or_old_keyboard_seats() {
            let base = [
                (COMPOSITOR_INTERFACE, 4),
                (MANAGER_INTERFACE, 1),
                (SHM_INTERFACE, 1),
                (OUTPUT_INTERFACE, 4),
            ];
            assert_eq!(classify(base), Err(Error::SeatUnavailable));
            assert_eq!(
                classify(base.into_iter().chain([(SEAT_INTERFACE, 3)])),
                Err(Error::SeatVersion {
                    advertised: 3,
                    required: 4,
                })
            );
        }
    }
}

/// Linux-PAM currently bounds a conversation response to 512 bytes. Keeping
/// the allocation fixed also prevents secret copies caused by `Vec` growth.
pub const MAX_SECRET_BYTES: usize = 512;

pub struct SecretInput {
    bytes: Vec<u8>,
}

impl Default for SecretInput {
    fn default() -> Self {
        Self {
            bytes: Vec::with_capacity(MAX_SECRET_BYTES),
        }
    }
}

impl SecretInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn character_count(&self) -> usize {
        // Construction preserves UTF-8, so failure would mean memory
        // corruption rather than user input and is treated as empty.
        std::str::from_utf8(&self.bytes)
            .map(|value| value.chars().count())
            .unwrap_or(0)
    }

    pub fn push(&mut self, character: char) -> Result<(), Error> {
        let mut encoded = [0_u8; 4];
        let value = character.encode_utf8(&mut encoded).as_bytes();
        let result = if self.bytes.len() + value.len() > MAX_SECRET_BYTES {
            Err(Error::Full)
        } else {
            self.bytes.extend_from_slice(value);
            Ok(())
        };
        encoded.zeroize();
        result
    }

    /// Append one already-decoded UTF-8 fragment atomically. If it does not
    /// fit, no prefix is retained.
    pub fn push_text(&mut self, value: &str) -> Result<(), Error> {
        if self.bytes.len() + value.len() > MAX_SECRET_BYTES {
            return Err(Error::Full);
        }
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub fn backspace(&mut self) -> bool {
        let Some(index) = std::str::from_utf8(&self.bytes)
            .ok()
            .and_then(|value| value.char_indices().next_back().map(|(index, _)| index))
        else {
            return false;
        };
        self.bytes[index..].zeroize();
        self.bytes.truncate(index);
        true
    }

    pub fn clear(&mut self) {
        self.bytes.zeroize();
    }

    /// Move the allocation to the bounded PAM worker without copying it.
    pub fn finish(mut self) -> SecretResponse {
        SecretResponse {
            bytes: std::mem::take(&mut self.bytes),
        }
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretInput(<redacted>)")
    }
}

impl Drop for SecretInput {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

pub struct SecretResponse {
    bytes: Vec<u8>,
}

impl SecretResponse {
    /// Scope plaintext access to the PAM conversation callback. Callers must
    /// not clone, log, cache, or return a borrow from this closure.
    pub fn expose<R>(&self, use_secret: impl FnOnce(&[u8]) -> R) -> R {
        use_secret(&self.bytes)
    }
}

impl fmt::Debug for SecretResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretResponse(<redacted>)")
    }
}

impl Drop for SecretResponse {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Full,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authentication response is too long")
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_input_and_backspace_preserve_scalar_boundaries() {
        let mut input = SecretInput::new();
        input.push('a').unwrap();
        input.push('é').unwrap();
        input.push('🔒').unwrap();
        assert_eq!(input.character_count(), 3);
        assert!(input.backspace());
        assert_eq!(input.character_count(), 2);
        let response = input.finish();
        response.expose(|bytes| assert_eq!(bytes, "aé".as_bytes()));
    }

    #[test]
    fn fixed_capacity_rejects_overflow_without_partial_input() {
        let mut input = SecretInput::new();
        for _ in 0..MAX_SECRET_BYTES {
            input.push('x').unwrap();
        }
        assert_eq!(input.character_count(), MAX_SECRET_BYTES);
        assert_eq!(input.push('y'), Err(Error::Full));
        assert_eq!(input.character_count(), MAX_SECRET_BYTES);
    }

    #[test]
    fn decoded_fragment_overflow_is_atomic() {
        let mut input = SecretInput::new();
        input.push_text(&"x".repeat(500)).unwrap();
        assert_eq!(input.push_text("ééééééé"), Err(Error::Full));
        assert_eq!(input.character_count(), 500);
        let response = input.finish();
        response.expose(|bytes| assert_eq!(bytes, vec![b'x'; 500]));
    }

    #[test]
    fn clear_erases_content_and_reuses_the_bounded_allocation() {
        let mut input = SecretInput::new();
        input.push('s').unwrap();
        input.push('e').unwrap();
        input.clear();
        assert!(input.is_empty());
        assert!(!input.backspace());
        input.push('n').unwrap();
        input.finish().expose(|bytes| assert_eq!(bytes, b"n"));
    }

    #[test]
    fn diagnostics_never_include_secret_or_length() {
        let mut input = SecretInput::new();
        for character in "private password".chars() {
            input.push(character).unwrap();
        }
        let input_debug = format!("{input:?}");
        let response = input.finish();
        let response_debug = format!("{response:?}");
        for debug in [input_debug, response_debug] {
            assert!(!debug.contains("private password"));
            assert!(!debug.contains("16"));
            assert!(debug.contains("<redacted>"));
        }
    }
}

#[cfg(test)]
mod evidence_assets {
    const CUSTOM_UNIT: &str = include_str!("../evidence/units/rmac-lock-provider-evidence.service");
    const FALLBACK_UNIT: &str =
        include_str!("../evidence/units/rmac-lock-fallback-evidence.service");
    const NESTED_SWAY: &str = include_str!("../evidence/nested-sway.conf");
    const INSTALLER: &str =
        include_str!("../../../scripts/linux/install-lock-provider-evidence.sh");
    const LAUNCHER: &str = include_str!("../../../scripts/linux/launch-lock-provider-evidence.sh");
    const NORMAL_INSTALLER: &str = include_str!("../../../scripts/linux/install-session-units.sh");
    const RECOVERY_GATE: &str =
        include_str!("../../../scripts/linux/run-lock-provider-recovery-gate.sh");

    #[test]
    fn evidence_units_are_separate_readiness_gated_crash_domains() {
        assert!(CUSTOM_UNIT.contains("Type=notify"));
        assert!(CUSTOM_UNIT.contains("NotifyAccess=all"));
        assert!(CUSTOM_UNIT.contains("Restart=on-failure"));
        assert!(CUSTOM_UNIT.contains("StartLimitIntervalSec=30s"));
        assert!(CUSTOM_UNIT.contains("StartLimitBurst=5"));
        assert!(CUSTOM_UNIT.contains("KillMode=control-group"));
        assert!(CUSTOM_UNIT.contains("EnvironmentFile=%t/rmac-lock-evidence/environment"));
        assert!(CUSTOM_UNIT.contains("/rmac-evidence/rmac-lock-provider"));

        assert!(FALLBACK_UNIT.contains("Type=notify"));
        assert!(FALLBACK_UNIT.contains("NotifyAccess=all"));
        assert!(FALLBACK_UNIT.contains("Restart=on-failure"));
        assert!(FALLBACK_UNIT.contains("StartLimitIntervalSec=0"));
        assert!(FALLBACK_UNIT.contains("/rmac/rmac-locker --config"));
        assert!(FALLBACK_UNIT.contains("EnvironmentFile=%t/rmac-lock-evidence/environment"));

        for unit in [CUSTOM_UNIT, FALLBACK_UNIT] {
            assert!(!unit.contains("[Install]"));
            assert!(!unit.contains("/bin/sh"));
            assert!(!unit.contains("rmac-lock.service"));
        }
    }

    #[test]
    fn evidence_scripts_require_opt_in_nested_recovery_without_installing_it_normally() {
        assert!(INSTALLER.contains("--features development-provider"));
        assert!(INSTALLER.contains("at least 25 GiB free"));
        assert!(INSTALLER.contains("no provider was started or enabled"));
        assert!(!INSTALLER.contains("systemctl --user enable"));
        assert!(!INSTALLER.contains("rmac-lock.service"));
        assert!(!NORMAL_INSTALLER.contains("development-provider"));
        assert!(!NORMAL_INSTALLER.contains("rmac-lock-provider-evidence"));

        assert!(LAUNCHER.contains("/run/user/$(id -u)"));
        assert!(LAUNCHER.contains("XDG_SESSION_ID"));
        assert!(LAUNCHER.contains("rmac-lock-provider-evidence.service"));
        assert!(NESTED_SWAY.contains("exec rmac-lock-provider-evidence-launch"));

        assert!(RECOVERY_GATE.contains("NESTED-LOCK-RECOVERY"));
        assert!(RECOVERY_GATE.contains("WLR_BACKENDS=wayland"));
        assert!(RECOVERY_GATE.contains("--signal=KILL"));
        assert!(RECOVERY_GATE.contains("rmac-lock-fallback-evidence.service"));
        assert!(RECOVERY_GATE.contains("SetLockedHint b false"));
        assert!(RECOVERY_GATE.contains("target/linux-evidence"));
        assert!(RECOVERY_GATE.contains("custom_restart_count"));
        assert!(!RECOVERY_GATE.contains("journalctl"));
        assert!(!RECOVERY_GATE.contains("rmac-lock.service"));
    }
}
