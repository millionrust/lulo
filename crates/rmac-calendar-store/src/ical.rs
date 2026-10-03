use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub start: TimeValue,
    pub end: TimeValue,
    pub rrules: Vec<String>,
    pub rdates: Vec<TimeValue>,
    pub exdates: Vec<TimeValue>,
    pub recurrence_id: Option<TimeValue>,
    pub cancelled: bool,
    /// Unmodelled VEVENT properties are kept for round trips (e.g. attendees and alarms).
    pub other_properties: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Date,
    Floating,
    Utc,
    Iana(Tz),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeValue {
    pub local: NaiveDateTime,
    pub zone: Zone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarError(pub String);

impl fmt::Display for CalendarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for CalendarError {}

impl TimeValue {
    pub fn resolve(self, floating_zone: Tz) -> Result<DateTime<Utc>, CalendarError> {
        let zone = match self.zone {
            Zone::Date | Zone::Floating => floating_zone,
            Zone::Utc => chrono_tz::UTC,
            Zone::Iana(zone) => zone,
        };
        match zone.from_local_datetime(&self.local) {
            LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
            // RFC 5545 resolves an ambiguous local time using the first occurrence.
            LocalResult::Ambiguous(first, _) => Ok(first.with_timezone(&Utc)),
            LocalResult::None => Err(CalendarError(format!(
                "local time {} does not exist in {zone}",
                self.local
            ))),
        }
    }

    pub fn date(date: NaiveDate) -> Result<Self, CalendarError> {
        let local = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| CalendarError("invalid date".into()))?;
        Ok(Self {
            local,
            zone: Zone::Date,
        })
    }
}

impl Calendar {
    pub fn parse(input: &str) -> Result<Self, CalendarError> {
        let lines = unfold(input)?;
        let mut events = Vec::new();
        let mut current: Option<Vec<String>> = None;
        let mut in_calendar = false;
        let mut depth = 0usize;
        for line in lines {
            match line.as_str() {
                "BEGIN:VCALENDAR" if !in_calendar => in_calendar = true,
                "END:VCALENDAR" if in_calendar && depth == 0 => in_calendar = false,
                "BEGIN:VEVENT" if in_calendar && current.is_none() => current = Some(Vec::new()),
                "END:VEVENT" if current.is_some() && depth == 0 => {
                    if let Some(properties) = current.take() {
                        events.push(parse_event(&properties)?);
                    }
                }
                _ if current.is_some() => {
                    if line.starts_with("BEGIN:") {
                        depth += 1;
                    } else if line.starts_with("END:") {
                        depth = depth.saturating_sub(1);
                    }
                    if let Some(properties) = &mut current {
                        properties.push(line);
                    }
                }
                _ => {}
            }
        }
        if in_calendar || current.is_some() || events.is_empty() {
            return Err(CalendarError("incomplete or empty VCALENDAR".into()));
        }
        Ok(Self { events })
    }

    pub fn to_ical(&self) -> String {
        let mut lines = vec![
            "BEGIN:VCALENDAR".to_string(),
            "VERSION:2.0".to_string(),
            "PRODID:-//Lulo//Calendar Store//EN".to_string(),
        ];
        for event in &self.events {
            lines.push("BEGIN:VEVENT".into());
            lines.push(format!("UID:{}", event.uid));
            lines.push(format!("SUMMARY:{}", escape_text(&event.summary)));
            lines.push(format_time("DTSTART", event.start));
            lines.push(format_time("DTEND", event.end));
            for rule in &event.rrules {
                lines.push(format!("RRULE:{rule}"));
            }
            for date in &event.rdates {
                lines.push(format_time("RDATE", *date));
            }
            for date in &event.exdates {
                lines.push(format_time("EXDATE", *date));
            }
            if let Some(id) = event.recurrence_id {
                lines.push(format_time("RECURRENCE-ID", id));
            }
            if event.cancelled {
                lines.push("STATUS:CANCELLED".into());
            }
            lines.extend(event.other_properties.iter().cloned());
            lines.push("END:VEVENT".into());
        }
        lines.push("END:VCALENDAR".into());
        let mut output = String::new();
        for line in lines {
            fold_line(&line, &mut output);
        }
        output
    }
}

fn unfold(input: &str) -> Result<Vec<String>, CalendarError> {
    if input.len() > 8 * 1024 * 1024 {
        return Err(CalendarError("calendar exceeds 8 MiB".into()));
    }
    let mut lines: Vec<String> = Vec::new();
    for raw in input.replace("\r\n", "\n").split('\n') {
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            let previous = lines
                .last_mut()
                .ok_or_else(|| CalendarError("folded line without predecessor".into()))?;
            previous.push_str(rest);
        } else if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    Ok(lines)
}

fn parse_event(lines: &[String]) -> Result<Event, CalendarError> {
    let mut uid = None;
    let mut summary = String::new();
    let mut start = None;
    let mut end = None;
    let mut rrules = Vec::new();
    let mut rdates = Vec::new();
    let mut exdates = Vec::new();
    let mut recurrence_id = None;
    let mut cancelled = false;
    let mut other_properties = Vec::new();
    let mut nested = 0usize;
    for line in lines {
        if line.starts_with("BEGIN:") {
            nested += 1;
            other_properties.push(line.clone());
            continue;
        }
        if nested > 0 {
            if line.starts_with("END:") {
                nested -= 1;
            }
            other_properties.push(line.clone());
            continue;
        }
        let (name_and_params, value) = line
            .split_once(':')
            .ok_or_else(|| CalendarError(format!("property lacks colon: {line}")))?;
        let name = name_and_params.split(';').next().unwrap_or_default();
        match name {
            "UID" => uid = Some(value.to_string()),
            "SUMMARY" => summary = unescape_text(value)?,
            "DTSTART" => start = Some(parse_time(name_and_params, value)?),
            "DTEND" => end = Some(parse_time(name_and_params, value)?),
            "RRULE" => rrules.push(value.to_string()),
            "RDATE" => parse_dates(name_and_params, value, &mut rdates)?,
            "EXDATE" => parse_dates(name_and_params, value, &mut exdates)?,
            "RECURRENCE-ID" => recurrence_id = Some(parse_time(name_and_params, value)?),
            "STATUS" if value == "CANCELLED" => cancelled = true,
            "STATUS" => other_properties.push(line.clone()),
            _ => other_properties.push(line.clone()),
        }
    }
    let uid = uid
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CalendarError("VEVENT missing UID".into()))?;
    let start = start.ok_or_else(|| CalendarError("VEVENT missing DTSTART".into()))?;
    let end = match end {
        Some(end) => end,
        None if start.zone == Zone::Date => TimeValue {
            local: start.local + Duration::days(1),
            zone: Zone::Date,
        },
        None => start,
    };
    if start.zone == Zone::Date && end.zone != Zone::Date
        || start.zone != Zone::Date && end.zone == Zone::Date
    {
        return Err(CalendarError("DTSTART and DTEND value types differ".into()));
    }
    if end.local < start.local && end.zone == start.zone {
        return Err(CalendarError("DTEND precedes DTSTART".into()));
    }
    Ok(Event {
        uid,
        summary,
        start,
        end,
        rrules,
        rdates,
        exdates,
        recurrence_id,
        cancelled,
        other_properties,
    })
}

fn parse_dates(
    property: &str,
    value: &str,
    output: &mut Vec<TimeValue>,
) -> Result<(), CalendarError> {
    for value in value.split(',') {
        output.push(parse_time(property, value)?);
    }
    Ok(())
}

fn parse_time(property: &str, value: &str) -> Result<TimeValue, CalendarError> {
    let mut zone = None;
    let mut date_only = false;
    for parameter in property.split(';').skip(1) {
        let (key, value) = parameter
            .split_once('=')
            .ok_or_else(|| CalendarError(format!("invalid parameter: {parameter}")))?;
        match key {
            "TZID" => {
                zone =
                    Some(Zone::Iana(value.trim_matches('"').parse::<Tz>().map_err(
                        |_| CalendarError(format!("unknown TZID: {value}")),
                    )?))
            }
            "VALUE" if value == "DATE" => date_only = true,
            "VALUE" if value == "DATE-TIME" => {}
            "VALUE" => return Err(CalendarError(format!("unsupported VALUE={value}"))),
            _ => {}
        }
    }
    if date_only {
        if zone.is_some() {
            return Err(CalendarError("DATE must not have TZID".into()));
        }
        let date = NaiveDate::parse_from_str(value, "%Y%m%d")
            .map_err(|_| CalendarError(format!("invalid DATE: {value}")))?;
        return TimeValue::date(date);
    }
    let (raw, utc) = match value.strip_suffix('Z') {
        Some(raw) => (raw, true),
        None => (value, false),
    };
    if utc && zone.is_some() {
        return Err(CalendarError("UTC value must not have TZID".into()));
    }
    let local = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S")
        .map_err(|_| CalendarError(format!("invalid DATE-TIME: {value}")))?;
    Ok(TimeValue {
        local,
        zone: if utc {
            Zone::Utc
        } else {
            zone.unwrap_or(Zone::Floating)
        },
    })
}

fn format_time(name: &str, value: TimeValue) -> String {
    match value.zone {
        Zone::Date => format!("{name};VALUE=DATE:{}", value.local.format("%Y%m%d")),
        Zone::Floating => format!("{name}:{}", value.local.format("%Y%m%dT%H%M%S")),
        Zone::Utc => format!("{name}:{}Z", value.local.format("%Y%m%dT%H%M%S")),
        Zone::Iana(zone) => format!("{name};TZID={zone}:{}", value.local.format("%Y%m%dT%H%M%S")),
    }
}

fn unescape_text(value: &str) -> Result<String, CalendarError> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => output.push('\n'),
                Some('\\') => output.push('\\'),
                Some(',') => output.push(','),
                Some(';') => output.push(';'),
                _ => return Err(CalendarError("invalid TEXT escape".into())),
            }
        } else {
            output.push(ch);
        }
    }
    Ok(output)
}

fn escape_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn fold_line(line: &str, output: &mut String) {
    let mut width = 0usize;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if width + len > 75 {
            output.push_str("\r\n ");
            width = 1;
        }
        output.push(ch);
        width += len;
    }
    output.push_str("\r\n");
}
