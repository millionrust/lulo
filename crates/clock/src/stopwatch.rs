//! Stopwatch arithmetic. Times are Unix milliseconds so a running stopwatch
//! survives quitting and reopening Clock, as it does on the Mac.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Stopwatch {
    /// Milliseconds accumulated before the current run.
    pub accumulated: u64,
    /// When the current run started, if running.
    pub started_at: Option<u64>,
    /// Total elapsed time at each completed lap, oldest first.
    pub laps: Vec<u64>,
}

/// What the two buttons under the digits say and do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Never started or reset: "Lap" (disabled) and "Start".
    Idle,
    /// "Lap" and "Stop".
    Running,
    /// "Reset" and "Start".
    Paused,
}

/// How a lap row is tinted: the fastest green, the slowest red, once there
/// are at least two completed laps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LapMark {
    Plain,
    Fastest,
    Slowest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LapRow {
    /// 1-based lap number.
    pub number: usize,
    pub split: u64,
    pub total: u64,
    pub mark: LapMark,
}

impl Stopwatch {
    pub fn phase(&self) -> Phase {
        match (self.started_at, self.accumulated) {
            (Some(_), _) => Phase::Running,
            (None, 0) if self.laps.is_empty() => Phase::Idle,
            (None, _) => Phase::Paused,
        }
    }

    pub fn elapsed(&self, now: u64) -> u64 {
        self.accumulated
            + self
                .started_at
                .map_or(0, |started| now.saturating_sub(started))
    }

    pub fn start(&mut self, now: u64) {
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }
    }

    pub fn stop(&mut self, now: u64) {
        if let Some(started) = self.started_at.take() {
            self.accumulated += now.saturating_sub(started);
        }
    }

    pub fn lap(&mut self, now: u64) {
        if self.started_at.is_some() {
            self.laps.push(self.elapsed(now));
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Rows newest first: the running lap, then completed laps.
    pub fn rows(&self, now: u64) -> Vec<LapRow> {
        let splits = self
            .laps
            .iter()
            .scan(0, |previous, &total| {
                let split = total - *previous;
                *previous = total;
                Some(split)
            })
            .collect::<Vec<_>>();
        let marked = splits.len() >= 2;
        let fastest = splits.iter().copied().min();
        let slowest = splits.iter().copied().max();
        let mut rows = splits
            .iter()
            .zip(&self.laps)
            .enumerate()
            .map(|(index, (&split, &total))| LapRow {
                number: index + 1,
                split,
                total,
                mark: if marked && fastest != slowest && Some(split) == fastest {
                    LapMark::Fastest
                } else if marked && fastest != slowest && Some(split) == slowest {
                    LapMark::Slowest
                } else {
                    LapMark::Plain
                },
            })
            .collect::<Vec<_>>();
        if self.phase() != Phase::Idle {
            let total = self.elapsed(now);
            let previous = self.laps.last().copied().unwrap_or(0);
            rows.push(LapRow {
                number: self.laps.len() + 1,
                split: total.saturating_sub(previous),
                total,
                mark: LapMark::Plain,
            });
        }
        rows.reverse();
        rows
    }
}

/// "00:00.00" — minutes, seconds and hundredths; hours appear after an hour
/// ("1:02:03.45").
pub fn format(milliseconds: u64) -> String {
    let hundredths = milliseconds / 10 % 100;
    let seconds = milliseconds / 1000 % 60;
    let minutes = milliseconds / 60_000 % 60;
    let hours = milliseconds / 3_600_000;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}.{hundredths:02}")
    } else {
        format!("{minutes:02}:{seconds:02}.{hundredths:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_the_mac() {
        assert_eq!(format(0), "00:00.00");
        assert_eq!(format(12_870), "00:12.87");
        assert_eq!(format(61_009), "01:01.00");
        assert_eq!(format(3_723_450), "1:02:03.45");
    }

    #[test]
    fn start_stop_resume_accumulates() {
        let mut watch = Stopwatch::default();
        assert_eq!(watch.phase(), Phase::Idle);
        watch.start(1_000);
        assert_eq!(watch.phase(), Phase::Running);
        assert_eq!(watch.elapsed(3_500), 2_500);
        watch.stop(4_000);
        assert_eq!(watch.phase(), Phase::Paused);
        assert_eq!(watch.elapsed(99_000), 3_000);
        watch.start(10_000);
        watch.start(11_000); // a second start is ignored
        assert_eq!(watch.elapsed(12_000), 5_000);
        watch.reset();
        assert_eq!(watch.phase(), Phase::Idle);
        assert_eq!(watch.elapsed(20_000), 0);
    }

    #[test]
    fn laps_split_and_mark_extremes() {
        let mut watch = Stopwatch::default();
        watch.lap(0); // not running: ignored
        watch.start(0);
        watch.lap(4_710);
        watch.lap(8_660);
        watch.lap(12_870);
        let rows = watch.rows(13_000);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].number, 4);
        assert_eq!(rows[0].split, 130);
        assert_eq!(rows[0].mark, LapMark::Plain);
        assert_eq!((rows[1].split, rows[1].total), (4_210, 12_870));
        assert_eq!(rows[2].split, 3_950);
        assert_eq!(rows[2].mark, LapMark::Fastest);
        assert_eq!(rows[3].split, 4_710);
        assert_eq!(rows[3].mark, LapMark::Slowest);
        assert_eq!(rows[1].mark, LapMark::Plain);
    }

    #[test]
    fn one_lap_is_not_marked_and_idle_has_no_rows() {
        assert!(Stopwatch::default().rows(0).is_empty());
        let mut watch = Stopwatch::default();
        watch.start(0);
        watch.lap(1_000);
        assert!(watch
            .rows(2_000)
            .iter()
            .all(|row| row.mark == LapMark::Plain));
    }

    #[test]
    fn persisted_state_round_trips() {
        let mut watch = Stopwatch::default();
        watch.start(5);
        watch.lap(10);
        let json = serde_json::to_string(&watch).unwrap();
        assert_eq!(serde_json::from_str::<Stopwatch>(&json).unwrap(), watch);
        assert_eq!(
            serde_json::from_str::<Stopwatch>("{}").unwrap(),
            Stopwatch::default()
        );
    }
}
