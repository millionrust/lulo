use std::collections::BTreeSet;

use rmac_focus::{Config, ModeId, Schedule, ScheduleId, Weekday};

use crate::model::rebuild;
use crate::Error;

pub fn create_schedule(
    configuration: &Config,
    mode_id: &ModeId,
) -> Result<(Config, ScheduleId), Error> {
    if configuration.mode(mode_id).is_none() {
        return Err(Error::UnknownMode);
    }
    if configuration.schedules().count() >= 64 {
        return Err(Error::Limit);
    }
    let schedule_id = (1..=64)
        .map(|index| ScheduleId::parse(format!("schedule-{index}")))
        .filter_map(Result::ok)
        .find(|candidate| {
            configuration
                .schedules()
                .all(|schedule| &schedule.id != candidate)
        })
        .ok_or(Error::Limit)?;
    let schedule = Schedule {
        id: schedule_id.clone(),
        mode: mode_id.clone(),
        days: BTreeSet::from([
            Weekday::Monday,
            Weekday::Tuesday,
            Weekday::Wednesday,
            Weekday::Thursday,
            Weekday::Friday,
        ]),
        start_minute: 9 * 60,
        end_minute: 17 * 60,
        priority: 1,
        enabled: true,
    };
    let rebuilt = upsert_schedule(configuration, schedule)?;
    Ok((rebuilt, schedule_id))
}

pub fn set_schedule_enabled(
    configuration: &Config,
    schedule_id: &ScheduleId,
    enabled: bool,
) -> Result<Config, Error> {
    replace_schedule(configuration, schedule_id, |schedule| {
        schedule.enabled = enabled;
        Ok(())
    })
}

pub fn set_schedule_day(
    configuration: &Config,
    schedule_id: &ScheduleId,
    day: Weekday,
    enabled: bool,
) -> Result<Config, Error> {
    replace_schedule(configuration, schedule_id, |schedule| {
        if enabled {
            schedule.days.insert(day);
        } else {
            schedule.days.remove(&day);
        }
        if schedule.days.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(())
    })
}

pub fn set_schedule_start(
    configuration: &Config,
    schedule_id: &ScheduleId,
    minute: u16,
) -> Result<Config, Error> {
    replace_schedule(configuration, schedule_id, |schedule| {
        schedule.start_minute = minute;
        Ok(())
    })
}

pub fn set_schedule_end(
    configuration: &Config,
    schedule_id: &ScheduleId,
    minute: u16,
) -> Result<Config, Error> {
    replace_schedule(configuration, schedule_id, |schedule| {
        schedule.end_minute = minute;
        Ok(())
    })
}

pub fn upsert_schedule(configuration: &Config, schedule: Schedule) -> Result<Config, Error> {
    let mut schedules = configuration.schedules().cloned().collect::<Vec<_>>();
    if let Some(existing) = schedules
        .iter_mut()
        .find(|existing| existing.id == schedule.id)
    {
        *existing = schedule;
    } else {
        schedules.push(schedule);
    }
    rebuild(
        configuration,
        configuration.modes().cloned().collect(),
        schedules,
    )
}

pub fn remove_schedule(configuration: &Config, schedule_id: &ScheduleId) -> Result<Config, Error> {
    let mut schedules = configuration.schedules().cloned().collect::<Vec<_>>();
    let before = schedules.len();
    schedules.retain(|schedule| &schedule.id != schedule_id);
    if schedules.len() == before {
        return Err(Error::UnknownSchedule);
    }
    rebuild(
        configuration,
        configuration.modes().cloned().collect(),
        schedules,
    )
}

fn replace_schedule(
    configuration: &Config,
    schedule_id: &ScheduleId,
    replacement: impl FnOnce(&mut Schedule) -> Result<(), Error>,
) -> Result<Config, Error> {
    let mut schedules = configuration.schedules().cloned().collect::<Vec<_>>();
    let schedule = schedules
        .iter_mut()
        .find(|schedule| &schedule.id == schedule_id)
        .ok_or(Error::UnknownSchedule)?;
    replacement(schedule)?;
    rebuild(
        configuration,
        configuration.modes().cloned().collect(),
        schedules,
    )
}
