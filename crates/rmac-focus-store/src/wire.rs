use std::collections::BTreeSet;

use rmac_focus::{Config, ManualActivation, Mode, ModeId, Schedule, ScheduleId, Weekday};
use rmac_notifications::AppId;
use serde::{Deserialize, Serialize};

use crate::model::VERSION;
use crate::store::domain_error;
use crate::{Error, ErrorKind, Operation};

#[derive(Deserialize, Serialize)]
pub(crate) struct StoredFile {
    version: u32,
    modes: Vec<StoredMode>,
    schedules: Vec<StoredSchedule>,
    manual: Option<StoredManual>,
}

impl StoredFile {
    pub(crate) fn from_domain(config: &Config, manual: Option<&ManualActivation>) -> Self {
        Self {
            version: VERSION,
            modes: config.modes().map(StoredMode::from_domain).collect(),
            schedules: config
                .schedules()
                .map(StoredSchedule::from_domain)
                .collect(),
            manual: manual.map(StoredManual::from_domain),
        }
    }

    pub(crate) fn into_domain(self) -> Result<(Config, Option<ManualActivation>), Error> {
        if self.version != VERSION {
            return Err(Error::new(
                Operation::Validate,
                ErrorKind::UnsupportedVersion,
            ));
        }
        let modes = self
            .modes
            .into_iter()
            .map(StoredMode::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let schedules = self
            .schedules
            .into_iter()
            .map(StoredSchedule::into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let config = Config::new(modes, schedules).map_err(domain_error)?;
        let manual = self.manual.map(StoredManual::into_domain).transpose()?;
        if manual
            .as_ref()
            .is_some_and(|manual| config.mode(&manual.mode).is_none())
        {
            return Err(Error::new(Operation::Validate, ErrorKind::Invalid));
        }
        Ok((config, manual))
    }
}

#[derive(Deserialize, Serialize)]
struct StoredMode {
    id: String,
    name: String,
    allowed_apps: Vec<String>,
    allow_urgent: bool,
}

impl StoredMode {
    fn from_domain(mode: &Mode) -> Self {
        Self {
            id: mode.id().as_str().to_owned(),
            name: mode.name().to_owned(),
            allowed_apps: mode
                .allowed_apps()
                .iter()
                .map(|app_id| app_id.as_str().to_owned())
                .collect(),
            allow_urgent: mode.allow_urgent(),
        }
    }

    fn into_domain(self) -> Result<Mode, Error> {
        let allowed_apps = self
            .allowed_apps
            .into_iter()
            .map(AppId::parse)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|_| Error::new(Operation::Validate, ErrorKind::Invalid))?;
        Mode::new(
            ModeId::parse(self.id).map_err(domain_error)?,
            self.name,
            allowed_apps,
            self.allow_urgent,
        )
        .map_err(domain_error)
    }
}

#[derive(Deserialize, Serialize)]
struct StoredSchedule {
    id: String,
    mode: String,
    days: Vec<StoredWeekday>,
    start_minute: u16,
    end_minute: u16,
    priority: u8,
    enabled: bool,
}

impl StoredSchedule {
    fn from_domain(schedule: &Schedule) -> Self {
        Self {
            id: schedule.id.as_str().to_owned(),
            mode: schedule.mode.as_str().to_owned(),
            days: schedule.days.iter().copied().map(Into::into).collect(),
            start_minute: schedule.start_minute,
            end_minute: schedule.end_minute,
            priority: schedule.priority,
            enabled: schedule.enabled,
        }
    }

    fn into_domain(self) -> Result<Schedule, Error> {
        Ok(Schedule {
            id: ScheduleId::parse(self.id).map_err(domain_error)?,
            mode: ModeId::parse(self.mode).map_err(domain_error)?,
            days: self.days.into_iter().map(Into::into).collect(),
            start_minute: self.start_minute,
            end_minute: self.end_minute,
            priority: self.priority,
            enabled: self.enabled,
        })
    }
}

#[derive(Deserialize, Serialize)]
struct StoredManual {
    mode: String,
    until_unix_ms: Option<u64>,
}

impl StoredManual {
    fn from_domain(manual: &ManualActivation) -> Self {
        Self {
            mode: manual.mode.as_str().to_owned(),
            until_unix_ms: manual.until_unix_ms,
        }
    }

    fn into_domain(self) -> Result<ManualActivation, Error> {
        Ok(ManualActivation {
            mode: ModeId::parse(self.mode).map_err(domain_error)?,
            until_unix_ms: self.until_unix_ms,
        })
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum StoredWeekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl From<Weekday> for StoredWeekday {
    fn from(day: Weekday) -> Self {
        match day {
            Weekday::Monday => Self::Monday,
            Weekday::Tuesday => Self::Tuesday,
            Weekday::Wednesday => Self::Wednesday,
            Weekday::Thursday => Self::Thursday,
            Weekday::Friday => Self::Friday,
            Weekday::Saturday => Self::Saturday,
            Weekday::Sunday => Self::Sunday,
        }
    }
}

impl From<StoredWeekday> for Weekday {
    fn from(day: StoredWeekday) -> Self {
        match day {
            StoredWeekday::Monday => Self::Monday,
            StoredWeekday::Tuesday => Self::Tuesday,
            StoredWeekday::Wednesday => Self::Wednesday,
            StoredWeekday::Thursday => Self::Thursday,
            StoredWeekday::Friday => Self::Friday,
            StoredWeekday::Saturday => Self::Saturday,
            StoredWeekday::Sunday => Self::Sunday,
        }
    }
}
