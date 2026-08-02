use rmac_focus::{Config, Mode, Schedule};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    UnknownMode,
    UnknownSchedule,
    Invalid,
    Limit,
}

pub(crate) fn rebuild(
    original: &Config,
    modes: Vec<Mode>,
    schedules: Vec<Schedule>,
) -> Result<Config, Error> {
    let rebuilt = Config::new(modes, schedules).map_err(|_| Error::Invalid)?;
    if &rebuilt == original {
        Ok(original.clone())
    } else {
        Ok(rebuilt)
    }
}
