//! Volume keys on Lulo OS and Windows: step the default output (or mute
//! one), then report what the OSD shows, from the audio backend's readback.

use crate::{Command, Error, Kind, Operation, Presentation};

/// One volume key's step, in percent (the Mac's 16 steps, rounded).
pub(crate) const AUDIO_STEP: u8 = 6;

pub(crate) fn execute_audio(command: Command) -> Result<Presentation, Error> {
    use rmac_audio::DeviceKind;

    let kind = if command == Command::ToggleInputMute {
        DeviceKind::Input
    } else {
        DeviceKind::Output
    };
    let before = rmac_audio::default_device(kind).map_err(|_| Error::new(Operation::ReadAudio))?;
    match command {
        Command::VolumeUp | Command::VolumeDown => {
            let volume = match command {
                Command::VolumeUp => before.level.volume.saturating_add(AUDIO_STEP).min(100),
                Command::VolumeDown => before.level.volume.saturating_sub(AUDIO_STEP),
                _ => unreachable!(),
            };
            rmac_audio::set_volume(DeviceKind::Output, volume)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
            if before.level.muted {
                rmac_audio::set_muted(DeviceKind::Output, false)
                    .map_err(|_| Error::new(Operation::ChangeAudio))?;
            }
        }
        Command::ToggleOutputMute => {
            rmac_audio::set_muted(DeviceKind::Output, !before.level.muted)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
        }
        Command::ToggleInputMute => {
            rmac_audio::set_muted(DeviceKind::Input, !before.level.muted)
                .map_err(|_| Error::new(Operation::ChangeAudio))?;
        }
        Command::ShowVolume => {}
        _ => unreachable!(),
    }

    let after = rmac_audio::default_device(kind).map_err(|_| Error::new(Operation::ReadAudio))?;
    if matches!(command, Command::VolumeUp | Command::VolumeDown) {
        let _ = rmac_sound::play(rmac_sound::Cue::VolumeTick);
    }
    Presentation::new(
        if kind == DeviceKind::Input {
            Kind::Input
        } else {
            Kind::Output
        },
        truncate_utf8(&after.name, 128),
        after.level.volume,
        after.level.muted,
    )
}

pub(crate) fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_owned()
}
