use crate::{Error, ErrorKind};

pub const MAX_TIMEZONES: usize = 1024;
const MAX_TIMEZONE_BYTES: usize = 128;

pub fn normalize_timezones(values: Vec<String>) -> (Vec<String>, bool) {
    let mut timezones = values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim();
            (validate_timezone_syntax(value).is_ok()).then(|| value.to_string())
        })
        .collect::<Vec<_>>();
    timezones.sort_unstable();
    timezones.dedup();
    let truncated = timezones.len() > MAX_TIMEZONES;
    timezones.truncate(MAX_TIMEZONES);
    (timezones, truncated)
}

pub fn validate_timezone_syntax(timezone: &str) -> Result<(), Error> {
    if timezone.is_empty() || timezone.len() > MAX_TIMEZONE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidTimezone,
            "enter a time zone such as Asia/Kolkata",
        ));
    }
    if timezone.starts_with('/')
        || timezone.starts_with('.')
        || timezone.ends_with('/')
        || timezone.contains("..")
        || timezone.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '+'))
        })
    {
        return Err(Error::new(
            ErrorKind::InvalidTimezone,
            "use a system time-zone name such as Asia/Kolkata",
        ));
    }
    Ok(())
}
