use rmac_focus::{Config, Mode, ModeId};
use rmac_notifications::AppId;

use crate::model::rebuild;
use crate::Error;

pub fn set_mode_urgent(
    configuration: &Config,
    mode_id: &ModeId,
    allow_urgent: bool,
) -> Result<Config, Error> {
    replace_mode(configuration, mode_id, |mode| {
        Mode::new(
            mode.id().clone(),
            mode.name(),
            mode.allowed_apps().clone(),
            allow_urgent,
        )
        .map_err(|_| Error::Invalid)
    })
}

pub fn set_allowed_app(
    configuration: &Config,
    mode_id: &ModeId,
    app_id: AppId,
    allowed: bool,
) -> Result<Config, Error> {
    replace_mode(configuration, mode_id, |mode| {
        let mut applications = mode.allowed_apps().clone();
        if allowed {
            applications.insert(app_id);
        } else {
            applications.remove(&app_id);
        }
        Mode::new(
            mode.id().clone(),
            mode.name(),
            applications,
            mode.allow_urgent(),
        )
        .map_err(|_| Error::Invalid)
    })
}

fn replace_mode(
    configuration: &Config,
    mode_id: &ModeId,
    replacement: impl FnOnce(&Mode) -> Result<Mode, Error>,
) -> Result<Config, Error> {
    let selected = configuration.mode(mode_id).ok_or(Error::UnknownMode)?;
    let replacement = replacement(selected)?;
    let modes = configuration
        .modes()
        .map(|mode| {
            if mode.id() == mode_id {
                replacement.clone()
            } else {
                mode.clone()
            }
        })
        .collect();
    rebuild(
        configuration,
        modes,
        configuration.schedules().cloned().collect(),
    )
}
