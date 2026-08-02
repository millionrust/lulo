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
            shm_version = Some(shm_version.map_or(version, |current: u32| current.max(version)));
        } else if name == SEAT_INTERFACE {
            seat_version = Some(seat_version.map_or(version, |current: u32| current.max(version)));
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
