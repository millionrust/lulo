//! Typed Spotlight intents: the one way a plain-language request may change
//! anything (ADR 0024 §1 feature 1, §5, §8's action table).
//!
//! The wire form is a flat JSON object, the same text the model is
//! constrained to write (see [`crate::decode`]):
//!
//! ```text
//! {"intent":"open_app","app":"Notes"}
//! {"intent":"appearance","mode":"dark"}
//! {"intent":"volume","level":30}      {"intent":"volume","change":"mute"}
//! {"intent":"brightness","level":70}  {"intent":"brightness","change":"up"}
//! {"intent":"wifi","on":false}  {"intent":"bluetooth","on":true}
//! {"intent":"do_not_disturb","on":true}
//! {"intent":"timer","amount":10,"unit":"minutes"}
//! {"intent":"search_files","query":"invoice"}
//! {"intent":"none"}
//! ```
//!
//! Parsing is strict: unknown intents, unknown or missing fields, values out
//! of range and stray text are all errors, so a row is only ever built from
//! a request this module fully understands.

use std::fmt;

use serde_json::{Map, Value};

/// The longest app name or file query an intent carries, in bytes.
pub const MAX_TEXT_BYTES: usize = 64;
/// The longest timer, in seconds (Clock's own limit is under a day).
pub const MAX_TIMER_SECONDS: u64 = 23 * 3600 + 59 * 60 + 59;
/// How far "turn it up" moves volume or brightness, in percent.
pub const STEP_PERCENT: u8 = 10;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AppearanceMode {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VolumeChange {
    Up,
    Down,
    Mute,
    Unmute,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BrightnessChange {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Level<C> {
    /// An absolute level, 0–100 percent.
    Percent(u8),
    Change(C),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimeUnit {
    Seconds,
    Minutes,
    Hours,
}

impl TimeUnit {
    fn seconds(self) -> u64 {
        match self {
            Self::Seconds => 1,
            Self::Minutes => 60,
            Self::Hours => 3600,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Intent {
    OpenApp {
        app: String,
    },
    Appearance {
        mode: AppearanceMode,
    },
    Volume(Level<VolumeChange>),
    Brightness(Level<BrightnessChange>),
    Wifi {
        on: bool,
    },
    Bluetooth {
        on: bool,
    },
    DoNotDisturb {
        on: bool,
    },
    Timer {
        amount: u32,
        unit: TimeUnit,
    },
    SearchFiles {
        query: String,
    },
    /// The request is not one of the supported actions (a question, chat,
    /// another setting, something Lulo cannot do). No row is shown.
    None,
}

/// Whether picking the row runs the action at once, or asks first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tier {
    /// Safe and reversible, or read-only: picking the row runs it.
    Runs,
    /// A Settings change: the row asks for a second Return or click.
    Confirm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IntentError {
    NotJson,
    NotAnObject,
    UnknownIntent,
    MissingField(&'static str),
    UnexpectedField,
    InvalidValue(&'static str),
}

impl fmt::Display for IntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => formatter.write_str("the intent is not JSON"),
            Self::NotAnObject => formatter.write_str("the intent is not a JSON object"),
            Self::UnknownIntent => formatter.write_str("the intent is not one Lulo supports"),
            Self::MissingField(field) => write!(formatter, "the intent has no {field}"),
            Self::UnexpectedField => formatter.write_str("the intent has an unexpected field"),
            Self::InvalidValue(field) => write!(formatter, "the intent's {field} is invalid"),
        }
    }
}

impl std::error::Error for IntentError {}

/// The intent names, in the order the prompt lists them.
pub const INTENT_NAMES: [&str; 10] = [
    "open_app",
    "appearance",
    "volume",
    "brightness",
    "wifi",
    "bluetooth",
    "do_not_disturb",
    "timer",
    "search_files",
    "none",
];

impl Intent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenApp { .. } => "open_app",
            Self::Appearance { .. } => "appearance",
            Self::Volume(_) => "volume",
            Self::Brightness(_) => "brightness",
            Self::Wifi { .. } => "wifi",
            Self::Bluetooth { .. } => "bluetooth",
            Self::DoNotDisturb { .. } => "do_not_disturb",
            Self::Timer { .. } => "timer",
            Self::SearchFiles { .. } => "search_files",
            Self::None => "none",
        }
    }

    /// Parse and validate the wire form.
    pub fn parse(json: &str) -> Result<Self, IntentError> {
        let value: Value = serde_json::from_str(json).map_err(|_| IntentError::NotJson)?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self, IntentError> {
        let object = value.as_object().ok_or(IntentError::NotAnObject)?;
        let name = object
            .get("intent")
            .and_then(Value::as_str)
            .ok_or(IntentError::MissingField("intent"))?;
        let fields = Fields(object);
        let intent = match name {
            "open_app" => {
                fields.only(&["app"])?;
                Self::OpenApp {
                    app: fields.text("app")?,
                }
            }
            "appearance" => {
                fields.only(&["mode"])?;
                Self::Appearance {
                    mode: match fields.string("mode")? {
                        "dark" => AppearanceMode::Dark,
                        "light" => AppearanceMode::Light,
                        _ => return Err(IntentError::InvalidValue("mode")),
                    },
                }
            }
            "volume" => Self::Volume(fields.level(|change| match change {
                "up" => Some(VolumeChange::Up),
                "down" => Some(VolumeChange::Down),
                "mute" => Some(VolumeChange::Mute),
                "unmute" => Some(VolumeChange::Unmute),
                _ => None,
            })?),
            "brightness" => Self::Brightness(fields.level(|change| match change {
                "up" => Some(BrightnessChange::Up),
                "down" => Some(BrightnessChange::Down),
                _ => None,
            })?),
            "wifi" => {
                fields.only(&["on"])?;
                Self::Wifi {
                    on: fields.boolean("on")?,
                }
            }
            "bluetooth" => {
                fields.only(&["on"])?;
                Self::Bluetooth {
                    on: fields.boolean("on")?,
                }
            }
            "do_not_disturb" => {
                fields.only(&["on"])?;
                Self::DoNotDisturb {
                    on: fields.boolean("on")?,
                }
            }
            "timer" => {
                fields.only(&["amount", "unit"])?;
                let amount = fields
                    .get("amount")?
                    .as_u64()
                    .filter(|amount| (1..=999).contains(amount))
                    .ok_or(IntentError::InvalidValue("amount"))?;
                let unit = match fields.string("unit")? {
                    "seconds" => TimeUnit::Seconds,
                    "minutes" => TimeUnit::Minutes,
                    "hours" => TimeUnit::Hours,
                    _ => return Err(IntentError::InvalidValue("unit")),
                };
                if amount * unit.seconds() > MAX_TIMER_SECONDS {
                    return Err(IntentError::InvalidValue("amount"));
                }
                Self::Timer {
                    amount: amount as u32,
                    unit,
                }
            }
            "search_files" => {
                fields.only(&["query"])?;
                Self::SearchFiles {
                    query: fields.text("query")?,
                }
            }
            "none" => {
                fields.only(&[])?;
                Self::None
            }
            _ => return Err(IntentError::UnknownIntent),
        };
        Ok(intent)
    }

    /// The canonical wire form. `parse(to_json(x)) == x` for every valid x.
    pub fn to_json(&self) -> String {
        let mut object = Map::new();
        object.insert("intent".into(), Value::from(self.name()));
        match self {
            Self::OpenApp { app } => {
                object.insert("app".into(), Value::from(app.as_str()));
            }
            Self::Appearance { mode } => {
                object.insert(
                    "mode".into(),
                    Value::from(match mode {
                        AppearanceMode::Dark => "dark",
                        AppearanceMode::Light => "light",
                    }),
                );
            }
            Self::Volume(level) => insert_level(
                &mut object,
                level.map_change(|change| match change {
                    VolumeChange::Up => "up",
                    VolumeChange::Down => "down",
                    VolumeChange::Mute => "mute",
                    VolumeChange::Unmute => "unmute",
                }),
            ),
            Self::Brightness(level) => insert_level(
                &mut object,
                level.map_change(|change| match change {
                    BrightnessChange::Up => "up",
                    BrightnessChange::Down => "down",
                }),
            ),
            Self::Wifi { on } | Self::Bluetooth { on } | Self::DoNotDisturb { on } => {
                object.insert("on".into(), Value::from(*on));
            }
            Self::Timer { amount, unit } => {
                object.insert("amount".into(), Value::from(*amount));
                object.insert(
                    "unit".into(),
                    Value::from(match unit {
                        TimeUnit::Seconds => "seconds",
                        TimeUnit::Minutes => "minutes",
                        TimeUnit::Hours => "hours",
                    }),
                );
            }
            Self::SearchFiles { query } => {
                object.insert("query".into(), Value::from(query.as_str()));
            }
            Self::None => {}
        }
        Value::Object(object).to_string()
    }

    pub fn tier(&self) -> Tier {
        match self {
            Self::OpenApp { .. } | Self::Timer { .. } | Self::SearchFiles { .. } | Self::None => {
                Tier::Runs
            }
            Self::Appearance { .. }
            | Self::Volume(_)
            | Self::Brightness(_)
            | Self::Wifi { .. }
            | Self::Bluetooth { .. }
            | Self::DoNotDisturb { .. } => Tier::Confirm,
        }
    }

    /// A timer's length in seconds.
    pub fn timer_seconds(&self) -> Option<u64> {
        match self {
            Self::Timer { amount, unit } => Some(u64::from(*amount) * unit.seconds()),
            _ => None,
        }
    }

    /// The Spotlight row title, in the Mac's title case ("Turn On Dark
    /// Mode"). `app_name` is the installed app's own name for `open_app`,
    /// which the launcher resolves; `None` means no row.
    pub fn title(&self, app_name: Option<&str>) -> Option<String> {
        let on_off = |on: bool| if on { "On" } else { "Off" };
        Some(match self {
            Self::OpenApp { app } => format!("Open {}", app_name.unwrap_or(app)),
            Self::Appearance {
                mode: AppearanceMode::Dark,
            } => "Turn On Dark Mode".into(),
            Self::Appearance {
                mode: AppearanceMode::Light,
            } => "Turn On Light Mode".into(),
            Self::Volume(Level::Percent(level)) => format!("Set Volume to {level}%"),
            Self::Volume(Level::Change(VolumeChange::Up)) => "Turn Volume Up".into(),
            Self::Volume(Level::Change(VolumeChange::Down)) => "Turn Volume Down".into(),
            Self::Volume(Level::Change(VolumeChange::Mute)) => "Mute Sound".into(),
            Self::Volume(Level::Change(VolumeChange::Unmute)) => "Unmute Sound".into(),
            Self::Brightness(Level::Percent(level)) => format!("Set Brightness to {level}%"),
            Self::Brightness(Level::Change(BrightnessChange::Up)) => "Turn Brightness Up".into(),
            Self::Brightness(Level::Change(BrightnessChange::Down)) => {
                "Turn Brightness Down".into()
            }
            Self::Wifi { on } => format!("Turn Wi-Fi {}", on_off(*on)),
            Self::Bluetooth { on } => format!("Turn Bluetooth {}", on_off(*on)),
            Self::DoNotDisturb { on } => format!("Turn {} Do Not Disturb", on_off(*on)),
            Self::Timer { amount, unit } => {
                let unit = match unit {
                    TimeUnit::Seconds => "Second",
                    TimeUnit::Minutes => "Minute",
                    TimeUnit::Hours => "Hour",
                };
                format!("Start a {amount}-{unit} Timer")
            }
            Self::SearchFiles { query } => format!("Search Files for \u{201c}{query}\u{201d}"),
            Self::None => return None,
        })
    }
}

impl<C: Copy> Level<C> {
    fn map_change<T>(self, change: impl FnOnce(C) -> T) -> Level<T> {
        match self {
            Self::Percent(level) => Level::Percent(level),
            Self::Change(value) => Level::Change(change(value)),
        }
    }
}

fn insert_level(object: &mut Map<String, Value>, level: Level<&'static str>) {
    match level {
        Level::Percent(level) => {
            object.insert("level".into(), Value::from(level));
        }
        Level::Change(change) => {
            object.insert("change".into(), Value::from(change));
        }
    }
}

struct Fields<'a>(&'a Map<String, Value>);

impl Fields<'_> {
    /// Exactly `intent` plus `allowed`, nothing else.
    fn only(&self, allowed: &[&str]) -> Result<(), IntentError> {
        if self
            .0
            .keys()
            .all(|key| key == "intent" || allowed.contains(&key.as_str()))
        {
            Ok(())
        } else {
            Err(IntentError::UnexpectedField)
        }
    }

    fn get(&self, field: &'static str) -> Result<&Value, IntentError> {
        self.0.get(field).ok_or(IntentError::MissingField(field))
    }

    fn string(&self, field: &'static str) -> Result<&str, IntentError> {
        self.get(field)?
            .as_str()
            .ok_or(IntentError::InvalidValue(field))
    }

    fn boolean(&self, field: &'static str) -> Result<bool, IntentError> {
        self.get(field)?
            .as_bool()
            .ok_or(IntentError::InvalidValue(field))
    }

    /// Free text: trimmed, non-empty, short, printable.
    fn text(&self, field: &'static str) -> Result<String, IntentError> {
        let text = self.string(field)?.trim();
        if text.is_empty()
            || text.len() > MAX_TEXT_BYTES
            || text.chars().any(|character| character.is_control())
        {
            return Err(IntentError::InvalidValue(field));
        }
        Ok(text.to_owned())
    }

    /// Exactly one of `level` (0–100) or `change`.
    fn level<C>(&self, change: impl Fn(&str) -> Option<C>) -> Result<Level<C>, IntentError> {
        match (self.0.get("level"), self.0.get("change")) {
            (Some(level), None) => {
                self.only(&["level"])?;
                level
                    .as_u64()
                    .filter(|level| *level <= 100)
                    .map(|level| Level::Percent(level as u8))
                    .ok_or(IntentError::InvalidValue("level"))
            }
            (None, Some(value)) => {
                self.only(&["change"])?;
                value
                    .as_str()
                    .and_then(change)
                    .map(Level::Change)
                    .ok_or(IntentError::InvalidValue("change"))
            }
            (None, None) => Err(IntentError::MissingField("level")),
            (Some(_), Some(_)) => Err(IntentError::UnexpectedField),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_shape() -> Vec<Intent> {
        vec![
            Intent::OpenApp {
                app: "Notes".into(),
            },
            Intent::Appearance {
                mode: AppearanceMode::Dark,
            },
            Intent::Appearance {
                mode: AppearanceMode::Light,
            },
            Intent::Volume(Level::Percent(0)),
            Intent::Volume(Level::Percent(100)),
            Intent::Volume(Level::Change(VolumeChange::Up)),
            Intent::Volume(Level::Change(VolumeChange::Down)),
            Intent::Volume(Level::Change(VolumeChange::Mute)),
            Intent::Volume(Level::Change(VolumeChange::Unmute)),
            Intent::Brightness(Level::Percent(70)),
            Intent::Brightness(Level::Change(BrightnessChange::Up)),
            Intent::Brightness(Level::Change(BrightnessChange::Down)),
            Intent::Wifi { on: true },
            Intent::Bluetooth { on: false },
            Intent::DoNotDisturb { on: true },
            Intent::Timer {
                amount: 10,
                unit: TimeUnit::Minutes,
            },
            Intent::SearchFiles {
                query: "invoice".into(),
            },
            Intent::None,
        ]
    }

    #[test]
    fn every_shape_round_trips_through_its_wire_form() {
        for intent in every_shape() {
            assert_eq!(Intent::parse(&intent.to_json()), Ok(intent.clone()));
        }
        // Every name in the prompt's list has a shape.
        let names: std::collections::BTreeSet<_> = every_shape().iter().map(Intent::name).collect();
        assert_eq!(names.len(), INTENT_NAMES.len());
    }

    #[test]
    fn parsing_is_strict() {
        for bad in [
            "",
            "dark mode",
            "[]",
            r#"{"intent":"shutdown"}"#,
            r#"{"intent":"wifi"}"#,
            r#"{"intent":"wifi","on":"yes"}"#,
            r#"{"intent":"wifi","on":true,"extra":1}"#,
            r#"{"intent":"volume","level":101}"#,
            r#"{"intent":"volume","level":-1}"#,
            r#"{"intent":"volume","level":30,"change":"up"}"#,
            r#"{"intent":"volume","change":"louder"}"#,
            r#"{"intent":"brightness","change":"mute"}"#,
            r#"{"intent":"appearance","mode":"blue"}"#,
            r#"{"intent":"timer","amount":0,"unit":"minutes"}"#,
            r#"{"intent":"timer","amount":25,"unit":"hours"}"#,
            r#"{"intent":"timer","amount":10,"unit":"days"}"#,
            r#"{"intent":"open_app","app":"  "}"#,
            r#"{"intent":"open_app","app":"a\nb"}"#,
            r#"{"intent":"none","app":"x"}"#,
            r#"{"intent":"search_files"}"#,
        ] {
            assert!(Intent::parse(bad).is_err(), "{bad}");
        }
        let long = format!(
            r#"{{"intent":"search_files","query":"{}"}}"#,
            "x".repeat(65)
        );
        assert!(Intent::parse(&long).is_err());
    }

    #[test]
    fn rows_say_exactly_what_will_change() {
        let title = |json: &str| Intent::parse(json).unwrap().title(None);
        assert_eq!(
            title(r#"{"intent":"appearance","mode":"dark"}"#).as_deref(),
            Some("Turn On Dark Mode")
        );
        assert_eq!(
            title(r#"{"intent":"volume","level":30}"#).as_deref(),
            Some("Set Volume to 30%")
        );
        assert_eq!(
            title(r#"{"intent":"wifi","on":false}"#).as_deref(),
            Some("Turn Wi-Fi Off")
        );
        assert_eq!(
            title(r#"{"intent":"do_not_disturb","on":true}"#).as_deref(),
            Some("Turn On Do Not Disturb")
        );
        assert_eq!(
            title(r#"{"intent":"timer","amount":10,"unit":"minutes"}"#).as_deref(),
            Some("Start a 10-Minute Timer")
        );
        assert_eq!(
            title(r#"{"intent":"search_files","query":"invoice"}"#).as_deref(),
            Some("Search Files for \u{201c}invoice\u{201d}")
        );
        assert_eq!(
            Intent::OpenApp {
                app: "notes".into()
            }
            .title(Some("Notes"))
            .as_deref(),
            Some("Open Notes")
        );
        assert_eq!(Intent::None.title(None), None);
    }

    #[test]
    fn settings_changes_ask_first() {
        for intent in every_shape() {
            let expected = match intent {
                Intent::OpenApp { .. }
                | Intent::Timer { .. }
                | Intent::SearchFiles { .. }
                | Intent::None => Tier::Runs,
                _ => Tier::Confirm,
            };
            assert_eq!(intent.tier(), expected, "{intent:?}");
        }
    }

    #[test]
    fn timers_convert_to_seconds() {
        let timer = Intent::parse(r#"{"intent":"timer","amount":25,"unit":"minutes"}"#).unwrap();
        assert_eq!(timer.timer_seconds(), Some(1500));
        assert_eq!(Intent::None.timer_seconds(), None);
    }
}
