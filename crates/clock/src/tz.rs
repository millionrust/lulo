//! Time-zone offsets from the system tz database (`/usr/share/zoneinfo`).
//!
//! Ubuntu always ships tzdata, so rmac reads the compiled TZif files instead
//! of vendoring a second copy of the database. Transitions come from the
//! file's 64-bit block; instants after the last transition follow the POSIX
//! TZ rule in the file's footer, exactly as glibc does.

use std::path::{Path, PathBuf};

const ZONEINFO: &str = "/usr/share/zoneinfo";
const MAX_TZIF_BYTES: u64 = 512 * 1024;

/// One compiled zone: UTC offsets (seconds east of Greenwich) over time.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    transitions: Vec<i64>,
    /// Index into `types` in force from the matching transition on.
    indices: Vec<u8>,
    types: Vec<LocalType>,
    rule: Option<Rule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalType {
    offset: i32,
    dst: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidName,
    Unreadable,
    Malformed,
}

impl Zone {
    /// A zone that is always UTC.
    pub fn utc() -> Self {
        Self::fixed("UTC", 0)
    }

    /// A zone with one constant offset.
    pub fn fixed(name: &str, offset: i32) -> Self {
        Self {
            name: name.to_owned(),
            transitions: Vec::new(),
            indices: Vec::new(),
            types: vec![LocalType { offset, dst: false }],
            rule: None,
        }
    }

    /// Load an IANA zone such as `Asia/Kolkata` from the system database.
    pub fn load(name: &str) -> Result<Self, Error> {
        Self::load_from(Path::new(ZONEINFO), name)
    }

    pub fn load_from(root: &Path, name: &str) -> Result<Self, Error> {
        if !valid_name(name) {
            return Err(Error::InvalidName);
        }
        let path = root.join(name);
        let metadata = std::fs::metadata(&path).map_err(|_| Error::Unreadable)?;
        if !metadata.is_file() || metadata.len() > MAX_TZIF_BYTES {
            return Err(Error::Unreadable);
        }
        let bytes = std::fs::read(&path).map_err(|_| Error::Unreadable)?;
        Self::parse(name, &bytes)
    }

    /// Parse TZif data (versions 1–4).
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self, Error> {
        let header = Header::read(bytes, 0)?;
        let (header, start, wide) = if header.version >= b'2' {
            let second = header.v1_block_len(4);
            (Header::read(bytes, second)?, second + 44, true)
        } else {
            (header, 44, false)
        };
        let time_size = if wide { 8 } else { 4 };
        let mut cursor = start;
        let take = |cursor: &mut usize, len: usize| -> Result<&[u8], Error> {
            let slice = bytes.get(*cursor..*cursor + len).ok_or(Error::Malformed)?;
            *cursor += len;
            Ok(slice)
        };
        let raw_times = take(&mut cursor, header.timecnt * time_size)?;
        let transitions = raw_times
            .chunks_exact(time_size)
            .map(|chunk| {
                if wide {
                    i64::from_be_bytes(chunk.try_into().unwrap_or([0; 8]))
                } else {
                    i64::from(i32::from_be_bytes(chunk.try_into().unwrap_or([0; 4])))
                }
            })
            .collect::<Vec<_>>();
        let indices = take(&mut cursor, header.timecnt)?.to_vec();
        let raw_types = take(&mut cursor, header.typecnt * 6)?;
        let types = raw_types
            .chunks_exact(6)
            .map(|chunk| LocalType {
                offset: i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                dst: chunk[4] != 0,
            })
            .collect::<Vec<_>>();
        if types.is_empty()
            || indices
                .iter()
                .any(|&index| usize::from(index) >= types.len())
        {
            return Err(Error::Malformed);
        }
        cursor +=
            header.charcnt + header.leapcnt * (time_size + 4) + header.isstdcnt + header.isutcnt;
        let rule = if wide {
            bytes
                .get(cursor..)
                .and_then(|footer| footer.strip_prefix(b"\n"))
                .and_then(|footer| footer.split(|&byte| byte == b'\n').next())
                .and_then(|footer| std::str::from_utf8(footer).ok())
                .filter(|footer| !footer.is_empty())
                .and_then(Rule::parse)
        } else {
            None
        };
        Ok(Self {
            name: name.to_owned(),
            transitions,
            indices,
            types,
            rule,
        })
    }

    /// Seconds east of UTC in force at `utc` (Unix seconds).
    pub fn offset_at(&self, utc: i64) -> i32 {
        self.local_type_at(utc).0
    }

    /// Whether daylight saving time is in force at `utc`.
    pub fn is_dst_at(&self, utc: i64) -> bool {
        self.local_type_at(utc).1
    }

    fn local_type_at(&self, utc: i64) -> (i32, bool) {
        if let (Some(&last), Some(rule)) = (self.transitions.last(), self.rule.as_ref()) {
            if utc >= last {
                return rule.offset_at(utc);
            }
        }
        if self.transitions.is_empty() {
            if let Some(rule) = &self.rule {
                return rule.offset_at(utc);
            }
        }
        match self.transitions.partition_point(|&time| time <= utc) {
            0 => {
                // Before the first transition: the first standard-time type.
                let first = self
                    .types
                    .iter()
                    .find(|kind| !kind.dst)
                    .unwrap_or(&self.types[0]);
                (first.offset, first.dst)
            }
            after => {
                let kind = self.types[usize::from(self.indices[after - 1])];
                (kind.offset, kind.dst)
            }
        }
    }
}

struct Header {
    version: u8,
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

impl Header {
    fn read(bytes: &[u8], at: usize) -> Result<Self, Error> {
        let header = bytes.get(at..at + 44).ok_or(Error::Malformed)?;
        if &header[..4] != b"TZif" {
            return Err(Error::Malformed);
        }
        let count = |index: usize| {
            let start = 20 + index * 4;
            u32::from_be_bytes([
                header[start],
                header[start + 1],
                header[start + 2],
                header[start + 3],
            ]) as usize
        };
        let parsed = Self {
            version: header[4],
            isutcnt: count(0),
            isstdcnt: count(1),
            leapcnt: count(2),
            timecnt: count(3),
            typecnt: count(4),
            charcnt: count(5),
        };
        // Bound every count so a hostile file cannot request huge buffers.
        if parsed.timecnt > 5_000 || parsed.typecnt > 256 || parsed.charcnt > 4_096 {
            return Err(Error::Malformed);
        }
        Ok(parsed)
    }

    fn v1_block_len(&self, time_size: usize) -> usize {
        44 + self.timecnt * time_size
            + self.timecnt
            + self.typecnt * 6
            + self.charcnt
            + self.leapcnt * (time_size + 4)
            + self.isstdcnt
            + self.isutcnt
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+'))
}

/// The system's own zone name, from `$TZ`, `/etc/timezone` or the
/// `/etc/localtime` link. `None` when it cannot be named.
pub fn local_zone_name() -> Option<String> {
    if let Ok(value) = std::env::var("TZ") {
        let value = value.trim_start_matches(':');
        if valid_name(value) {
            return Some(value.to_owned());
        }
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        if let Some(name) = zone_name_from_path(&target) {
            return Some(name);
        }
    }
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| valid_name(value))
}

/// `…/zoneinfo/Asia/Kolkata` → `Asia/Kolkata`.
pub fn zone_name_from_path(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let (_, name) = text.rsplit_once("zoneinfo/")?;
    valid_name(name).then(|| name.to_owned())
}

/// The system zone, or UTC when it cannot be read.
pub fn local_zone() -> Zone {
    local_zone_name()
        .and_then(|name| Zone::load(&name).ok())
        .or_else(|| Zone::parse("localtime", &std::fs::read("/etc/localtime").ok()?).ok())
        .unwrap_or_else(Zone::utc)
}

pub fn zoneinfo_root() -> PathBuf {
    PathBuf::from(ZONEINFO)
}

// ------------------------------------------------------------ POSIX TZ rules

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    /// Seconds east of UTC.
    standard: i32,
    dst: Option<Dst>,
}

#[derive(Clone, Debug, PartialEq)]
struct Dst {
    offset: i32,
    start: (DateRule, i32),
    end: (DateRule, i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DateRule {
    /// `Jn`: day 1–365, February 29 never counted.
    Julian1(u16),
    /// `n`: day 0–365, February 29 counted in leap years.
    Julian0(u16),
    /// `Mm.w.d`: day `d` (0 = Sunday) of week `w` (5 = last) of month `m`.
    MonthWeekDay(u8, u8, u8),
}

impl Rule {
    fn parse(text: &str) -> Option<Self> {
        let mut rest = text;
        skip_name(&mut rest)?;
        let standard = -parse_offset(&mut rest)?;
        if rest.is_empty() {
            return Some(Self {
                standard,
                dst: None,
            });
        }
        skip_name(&mut rest)?;
        let offset = if rest.starts_with(',') {
            standard + 3600
        } else {
            -parse_offset(&mut rest)?
        };
        let rest = rest.strip_prefix(',')?;
        let (start, end) = rest.split_once(',')?;
        Some(Self {
            standard,
            dst: Some(Dst {
                offset,
                start: parse_transition(start)?,
                end: parse_transition(end)?,
            }),
        })
    }

    fn offset_at(&self, utc: i64) -> (i32, bool) {
        let Some(dst) = &self.dst else {
            return (self.standard, false);
        };
        let local_year = civil_from_days((utc + i64::from(self.standard)).div_euclid(86_400)).0;
        // Transition instants in UTC: the start is given in standard local
        // time, the end in daylight local time.
        let start = rule_instant(local_year, dst.start) - i64::from(self.standard);
        let end = rule_instant(local_year, dst.end) - i64::from(dst.offset);
        let in_dst = if start < end {
            utc >= start && utc < end
        } else {
            // Southern hemisphere: DST spans the new year.
            utc >= start || utc < end
        };
        if in_dst {
            (dst.offset, true)
        } else {
            (self.standard, false)
        }
    }
}

fn skip_name(rest: &mut &str) -> Option<()> {
    if let Some(quoted) = rest.strip_prefix('<') {
        let (_, after) = quoted.split_once('>')?;
        *rest = after;
        return Some(());
    }
    let len = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
    if len < 3 {
        return None;
    }
    *rest = &rest[len..];
    Some(())
}

/// `[+-]hh[:mm[:ss]]` in seconds, POSIX sign (positive = west of UTC).
fn parse_offset(rest: &mut &str) -> Option<i32> {
    let (sign, body) = match rest.as_bytes().first()? {
        b'-' => (-1, &rest[1..]),
        b'+' => (1, &rest[1..]),
        _ => (1, *rest),
    };
    let len = body
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || *byte == b':')
        .count();
    let seconds = parse_hms(&body[..len])?;
    *rest = &body[len..];
    Some(sign * seconds)
}

fn parse_hms(text: &str) -> Option<i32> {
    let mut parts = text.split(':');
    let hours: i32 = parts.next()?.parse().ok()?;
    let minutes: i32 = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    let seconds: i32 = parts.next().map_or(Some(0), |part| part.parse().ok())?;
    (hours <= 167 && minutes < 60 && seconds < 60).then_some(hours * 3600 + minutes * 60 + seconds)
}

fn parse_transition(text: &str) -> Option<(DateRule, i32)> {
    let (date, time) = match text.split_once('/') {
        Some((date, time)) => {
            let (sign, time) = match time.strip_prefix('-') {
                Some(time) => (-1, time),
                None => (1, time.trim_start_matches('+')),
            };
            (date, sign * parse_hms(time)?)
        }
        None => (text, 7200),
    };
    let rule = if let Some(month) = date.strip_prefix('M') {
        let mut parts = month.split('.');
        let month: u8 = parts.next()?.parse().ok()?;
        let week: u8 = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=5).contains(&week) || day > 6 {
            return None;
        }
        DateRule::MonthWeekDay(month, week, day)
    } else if let Some(day) = date.strip_prefix('J') {
        let day: u16 = day.parse().ok()?;
        (1..=365).contains(&day).then_some(DateRule::Julian1(day))?
    } else {
        let day: u16 = date.parse().ok()?;
        (day <= 365).then_some(DateRule::Julian0(day))?
    };
    Some((rule, time))
}

/// Seconds since the epoch of the rule's local wall time in `year`
/// (not yet corrected by any offset).
fn rule_instant(year: i64, (rule, time): (DateRule, i32)) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    let day = match rule {
        DateRule::Julian1(day) => {
            let day = i64::from(day) - 1;
            // Day 59 is March 1 whether or not the year is leap.
            jan1 + day + i64::from(is_leap(year) && day >= 59)
        }
        DateRule::Julian0(day) => jan1 + i64::from(day),
        DateRule::MonthWeekDay(month, week, weekday) => {
            let first = days_from_civil(year, u32::from(month), 1);
            let first_weekday = (first + 4).rem_euclid(7); // 1970-01-01 was a Thursday
            let mut day = first
                + (i64::from(weekday) - first_weekday).rem_euclid(7)
                + 7 * (i64::from(week) - 1);
            let next_month = if month == 12 {
                days_from_civil(year + 1, 1, 1)
            } else {
                days_from_civil(year, u32::from(month) + 1, 1)
            };
            while day >= next_month {
                day -= 7;
            }
            day
        }
    };
    day * 86_400 + i64::from(time)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The inverse of [`days_from_civil`]: (year, month, day).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal TZif v2 file with the given 64-bit transitions.
    fn tzif(transitions: &[(i64, u8)], types: &[(i32, bool)], footer: &str) -> Vec<u8> {
        let header = |version: u8, timecnt: usize, typecnt: usize| {
            let mut bytes = b"TZif".to_vec();
            bytes.push(version);
            bytes.extend([0; 15]);
            for count in [0, 0, 0, timecnt, typecnt, 4] {
                bytes.extend((count as u32).to_be_bytes());
            }
            bytes
        };
        // An empty v1 block with one type, then the v2 block.
        let mut bytes = header(b'2', 0, 1);
        bytes.extend([0, 0, 0, 0, 0, 0]);
        bytes.extend(b"UTC\0");
        bytes.extend(header(b'2', transitions.len(), types.len()));
        for (time, _) in transitions {
            bytes.extend(time.to_be_bytes());
        }
        for (_, index) in transitions {
            bytes.push(*index);
        }
        for (offset, dst) in types {
            bytes.extend(offset.to_be_bytes());
            bytes.push(u8::from(*dst));
            bytes.push(0);
        }
        bytes.extend(b"ABC\0");
        bytes.push(b'\n');
        bytes.extend(footer.as_bytes());
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn civil_days_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for days in [-1_000_000, -1, 0, 59, 11_016, 20_719, 1_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(20_719), (2026, 9, 23));
    }

    #[test]
    fn fixed_offset_zone() {
        let zone = Zone::parse("Asia/Kolkata", &tzif(&[], &[(19_800, false)], "IST-5:30")).unwrap();
        assert_eq!(zone.offset_at(1_790_000_000), 19_800);
        assert!(!zone.is_dst_at(1_790_000_000));
    }

    #[test]
    fn transitions_then_footer_rule() {
        // New York: one historic transition, then the US rule.
        let zone = Zone::parse(
            "America/New_York",
            &tzif(
                &[(-2_717_650_800, 1)],
                &[(-17_762, false), (-18_000, false), (-14_400, true)],
                "EST5EDT,M3.2.0,M11.1.0",
            ),
        )
        .unwrap();
        // 2026-01-15 12:00 UTC: standard time.
        let winter = days_from_civil(2026, 1, 15) * 86_400 + 43_200;
        assert_eq!(zone.offset_at(winter), -18_000);
        // 2026-07-01: daylight time.
        let summer = days_from_civil(2026, 7, 1) * 86_400;
        assert_eq!(zone.offset_at(summer), -14_400);
        assert!(zone.is_dst_at(summer));
        // DST starts 2026-03-08 02:00 EST = 07:00 UTC.
        let start = days_from_civil(2026, 3, 8) * 86_400 + 7 * 3600;
        assert_eq!(zone.offset_at(start - 1), -18_000);
        assert_eq!(zone.offset_at(start), -14_400);
        // DST ends 2026-11-01 02:00 EDT = 06:00 UTC.
        let end = days_from_civil(2026, 11, 1) * 86_400 + 6 * 3600;
        assert_eq!(zone.offset_at(end - 1), -14_400);
        assert_eq!(zone.offset_at(end), -18_000);
        // Before the first transition: local mean time.
        assert_eq!(zone.offset_at(-3_000_000_000), -17_762);
    }

    #[test]
    fn southern_hemisphere_rule_spans_new_year() {
        let zone = Zone::parse(
            "Australia/Sydney",
            &tzif(&[], &[(36_000, false)], "AEST-10AEDT,M10.1.0,M4.1.0/3"),
        )
        .unwrap();
        let january = days_from_civil(2027, 1, 10) * 86_400;
        let june = days_from_civil(2027, 6, 10) * 86_400;
        assert_eq!(zone.offset_at(january), 39_600);
        assert_eq!(zone.offset_at(june), 36_000);
    }

    #[test]
    fn quoted_names_and_julian_rules() {
        let rule = Rule::parse("<+0330>-3:30").unwrap();
        assert_eq!(rule.offset_at(0), (12_600, false));
        let rule = Rule::parse("XXX3YYY,J60/0,J300/0").unwrap();
        // J60 is March 1 in every year.
        let march1 = days_from_civil(2028, 3, 1) * 86_400 + 3 * 3600;
        assert!(!rule.offset_at(march1 - 1).1);
        assert!(rule.offset_at(march1 + 1).1);
    }

    #[test]
    fn last_week_rule_picks_the_final_sunday() {
        // Europe/London: last Sunday in March 2026 is the 29th.
        let rule = Rule::parse("GMT0BST,M3.5.0/1,M10.5.0").unwrap();
        let start = days_from_civil(2026, 3, 29) * 86_400 + 3600;
        assert_eq!(rule.offset_at(start - 1), (0, false));
        assert_eq!(rule.offset_at(start), (3600, true));
    }

    #[test]
    fn names_are_confined_to_the_database() {
        assert!(valid_name("Asia/Kolkata"));
        assert!(valid_name("Etc/GMT+5"));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("/etc/localtime"));
        assert!(!valid_name("Asia//Kolkata"));
        assert_eq!(Zone::load("../../etc/passwd"), Err(Error::InvalidName));
        assert_eq!(
            zone_name_from_path(Path::new("/usr/share/zoneinfo/Europe/Paris")).as_deref(),
            Some("Europe/Paris")
        );
        assert_eq!(
            zone_name_from_path(Path::new("../usr/share/zoneinfo/UTC")).as_deref(),
            Some("UTC")
        );
    }

    #[test]
    fn malformed_data_is_rejected() {
        assert_eq!(Zone::parse("x", b"nope"), Err(Error::Malformed));
        let mut bytes = tzif(&[(0, 5)], &[(0, false)], "UTC0");
        bytes.truncate(60);
        assert!(Zone::parse("x", &bytes).is_err());
        // A transition naming a missing type.
        assert_eq!(
            Zone::parse("x", &tzif(&[(0, 5)], &[(0, false)], "UTC0")),
            Err(Error::Malformed)
        );
    }

    #[test]
    fn system_database_when_present() {
        let root = zoneinfo_root();
        if let Ok(zone) = Zone::load_from(&root, "Asia/Kolkata") {
            assert_eq!(zone.offset_at(1_790_000_000), 19_800);
        }
        if let Ok(zone) = Zone::load_from(&root, "Europe/Berlin") {
            let summer = days_from_civil(2030, 7, 1) * 86_400;
            assert_eq!(zone.offset_at(summer), 7_200);
        }
    }
}
