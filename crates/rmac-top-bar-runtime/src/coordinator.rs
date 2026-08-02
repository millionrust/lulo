use chrono::{DateTime, FixedOffset};

const FALLBACK_HOUR_CYCLE: rmac_top_bar::LocaleHourCycle =
    rmac_top_bar::LocaleHourCycle::TwentyFourHour;

/// Pure reconciliation state. Wall-clock samples are injected so clock jumps,
/// locale changes, and timer behavior can be verified without sleeping.
#[derive(Default)]
pub struct Coordinator {
    presentation: rmac_top_bar::State,
    status: Option<rmac_shell_status::Snapshot>,
    locale_hour_cycle: Option<rmac_top_bar::LocaleHourCycle>,
    time_source_ready: bool,
}

impl Coordinator {
    pub fn apply_shell(
        &mut self,
        update: rmac_shell_runtime::Update,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        self.status = Some(update.snapshot.status);
        self.evaluate(now)
    }

    pub fn apply_locale(
        &mut self,
        hour_cycle: Result<rmac_locale::HourCycle, ()>,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        match hour_cycle {
            Ok(hour_cycle) => self.locale_hour_cycle = Some(top_bar_hour_cycle(hour_cycle)),
            Err(()) if self.locale_hour_cycle.is_none() => {
                self.locale_hour_cycle = Some(FALLBACK_HOUR_CYCLE);
            }
            Err(()) => {}
        }
        self.evaluate(now)
    }

    pub fn apply_time_signal(
        &mut self,
        now: DateTime<FixedOffset>,
    ) -> Option<rmac_top_bar::Update> {
        self.time_source_ready = true;
        self.evaluate(now)
    }

    pub fn clock_deadline(&mut self, now: DateTime<FixedOffset>) -> Option<rmac_top_bar::Update> {
        self.evaluate(now)
    }

    fn evaluate(&mut self, now: DateTime<FixedOffset>) -> Option<rmac_top_bar::Update> {
        let status = self.status.as_ref()?;
        let locale_hour_cycle = self.locale_hour_cycle?;
        self.time_source_ready
            .then(|| self.presentation.apply(status, now, locale_hour_cycle))
    }
}

fn top_bar_hour_cycle(value: rmac_locale::HourCycle) -> rmac_top_bar::LocaleHourCycle {
    match value {
        rmac_locale::HourCycle::TwelveHour => rmac_top_bar::LocaleHourCycle::TwelveHour,
        rmac_locale::HourCycle::TwentyFourHour => rmac_top_bar::LocaleHourCycle::TwentyFourHour,
    }
}
