pub(super) enum SoundChange {
    Volume(rmac_audio::DeviceKind, u8),
    Muted(rmac_audio::DeviceKind, bool),
    DefaultDevice(rmac_audio::DeviceKind, rmac_audio::Device),
    Profile(rmac_audio::HardwareDevice, rmac_audio::Profile),
    Route(
        rmac_audio::DeviceKind,
        rmac_audio::Device,
        rmac_audio::Route,
    ),
    Balance(rmac_audio::Device, i8),
}

impl SoundChange {
    pub(super) fn apply(self) -> std::result::Result<rmac_audio::Snapshot, rmac_audio::Error> {
        match self {
            Self::Volume(kind, volume) => rmac_audio::set_volume(kind, volume)?,
            Self::Muted(kind, muted) => rmac_audio::set_muted(kind, muted)?,
            Self::DefaultDevice(kind, device) => {
                return rmac_audio::set_default_device(kind, &device);
            }
            Self::Profile(device, profile) => {
                return rmac_audio::set_profile(&device, &profile);
            }
            Self::Route(kind, device, route) => {
                return rmac_audio::set_route(kind, &device, &route);
            }
            Self::Balance(device, value) => {
                return rmac_audio::set_balance(&device, value);
            }
        }
        rmac_audio::snapshot()
    }
}

pub(super) fn choice_is_actionable(
    is_active: bool,
    availability: rmac_audio::Availability,
    busy: bool,
) -> bool {
    !is_active && availability.can_select() && !busy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_require_selectable_inactive_idle_authority() {
        assert!(choice_is_actionable(
            false,
            rmac_audio::Availability::Available,
            false,
        ));
        assert!(choice_is_actionable(
            false,
            rmac_audio::Availability::Unknown,
            false,
        ));
        assert!(!choice_is_actionable(
            false,
            rmac_audio::Availability::Unavailable,
            false,
        ));
        assert!(!choice_is_actionable(
            true,
            rmac_audio::Availability::Available,
            false,
        ));
        assert!(!choice_is_actionable(
            false,
            rmac_audio::Availability::Available,
            true,
        ));
    }
}
