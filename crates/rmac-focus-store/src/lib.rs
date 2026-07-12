//! Private, versioned persistence for Focus modes and manual activation.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_focus::{Config, ManualActivation, Mode, ModeId, Schedule, ScheduleId, Weekday};
use rmac_notifications::AppId;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Recovery {
    #[default]
    None,
    LastGood,
    Defaults,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Snapshot {
    pub config: Config,
    pub manual: Option<ManualActivation>,
    pub recovery: Recovery,
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Snapshot")
            .field("config", &self.config)
            .field("manual", &self.manual)
            .field("recovery", &self.recovery)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Resolve,
    Read,
    Parse,
    Validate,
    CreateDirectory,
    Serialize,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io(io::ErrorKind),
    Invalid,
    UnsupportedVersion,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    pub kind: ErrorKind,
}

impl Error {
    fn new(operation: Operation, kind: ErrorKind) -> Self {
        Self { operation, kind }
    }

    fn io(operation: Operation, error: io::Error) -> Self {
        Self::new(operation, ErrorKind::Io(error.kind()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Focus settings operation failed ({:?})",
            self.operation
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Store(<redacted path>)")
    }
}

impl Store {
    pub fn from_environment() -> Result<Self, Error> {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        Ok(Self::at(config_home.join("rmac/focus.json")))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Snapshot, Error> {
        match self.read(&self.path) {
            Ok((config, manual)) => Ok(Snapshot {
                config,
                manual,
                recovery: Recovery::None,
            }),
            Err(error) if error.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                match self.read(&self.backup_path()) {
                    Ok((config, manual)) => Ok(Snapshot {
                        config,
                        manual,
                        recovery: Recovery::LastGood,
                    }),
                    Err(backup) if backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) => {
                        Ok(Snapshot {
                            config: default_config()?,
                            manual: None,
                            recovery: Recovery::None,
                        })
                    }
                    Err(backup) => Err(backup),
                }
            }
            Err(error) if recoverable(error) => match self.read(&self.backup_path()) {
                Ok((config, manual)) => Ok(Snapshot {
                    config,
                    manual,
                    recovery: Recovery::LastGood,
                }),
                Err(backup)
                    if recoverable(backup)
                        || backup.kind == ErrorKind::Io(io::ErrorKind::NotFound) =>
                {
                    Ok(Snapshot {
                        config: default_config()?,
                        manual: None,
                        recovery: Recovery::Defaults,
                    })
                }
                Err(backup) => Err(backup),
            },
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, config: &Config, manual: Option<&ManualActivation>) -> Result<(), Error> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::new(Operation::Resolve, ErrorKind::Invalid))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| Error::io(Operation::CreateDirectory, error))?;
        set_private_directory(parent)?;
        let file = StoredFile::from_domain(config, manual);
        let bytes = serde_json::to_vec(&file)
            .map_err(|_| Error::new(Operation::Serialize, ErrorKind::Invalid))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Serialize, ErrorKind::Limit));
        }
        rmac_storage::atomic_write_private(&self.backup_path(), &bytes)
            .map_err(|error| Error::io(Operation::Save, error))?;
        rmac_storage::atomic_write_private(&self.path, &bytes)
            .map_err(|error| Error::io(Operation::Save, error))
    }

    fn read(&self, path: &Path) -> Result<(Config, Option<ManualActivation>), Error> {
        let metadata =
            std::fs::metadata(path).map_err(|error| Error::io(Operation::Read, error))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(Error::new(Operation::Read, ErrorKind::Limit));
        }
        let bytes = std::fs::read(path).map_err(|error| Error::io(Operation::Read, error))?;
        let file: StoredFile = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new(Operation::Parse, ErrorKind::Invalid))?;
        file.into_domain()
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_extension("last-good.json")
    }
}

fn recoverable(error: Error) -> bool {
    matches!(
        error.kind,
        ErrorKind::Invalid | ErrorKind::UnsupportedVersion | ErrorKind::Limit
    )
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| Error::io(Operation::CreateDirectory, error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<(), Error> {
    Ok(())
}

pub fn default_config() -> Result<Config, Error> {
    Config::new(
        vec![
            Mode::new(
                ModeId::parse("do-not-disturb").map_err(domain_error)?,
                "Do Not Disturb",
                BTreeSet::new(),
                false,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("personal").map_err(domain_error)?,
                "Personal",
                BTreeSet::new(),
                true,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("work").map_err(domain_error)?,
                "Work",
                BTreeSet::new(),
                true,
            )
            .map_err(domain_error)?,
            Mode::new(
                ModeId::parse("sleep").map_err(domain_error)?,
                "Sleep",
                BTreeSet::new(),
                false,
            )
            .map_err(domain_error)?,
        ],
        Vec::new(),
    )
    .map_err(domain_error)
}

fn domain_error(_error: rmac_focus::Error) -> Error {
    Error::new(Operation::Validate, ErrorKind::Invalid)
}

#[derive(Deserialize, Serialize)]
struct StoredFile {
    version: u32,
    modes: Vec<StoredMode>,
    schedules: Vec<StoredSchedule>,
    manual: Option<StoredManual>,
}

impl StoredFile {
    fn from_domain(config: &Config, manual: Option<&ManualActivation>) -> Self {
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

    fn into_domain(self) -> Result<(Config, Option<ManualActivation>), Error> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn path(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "rmac-focus-store-{}-{label}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ))
            .join("focus.json")
    }

    fn custom() -> (Config, ManualActivation) {
        let mode = Mode::new(
            ModeId::parse("private-work-8472").unwrap(),
            "Private Work 8472",
            [AppId::parse("org.private.App8472").unwrap()]
                .into_iter()
                .collect(),
            false,
        )
        .unwrap();
        let schedule = Schedule {
            id: ScheduleId::parse("private-schedule-8472").unwrap(),
            mode: mode.id().clone(),
            days: [Weekday::Friday].into_iter().collect(),
            start_minute: 22 * 60,
            end_minute: 7 * 60,
            priority: 4,
            enabled: true,
        };
        let config = Config::new(vec![mode], vec![schedule]).unwrap();
        let manual = ManualActivation {
            mode: ModeId::parse("private-work-8472").unwrap(),
            until_unix_ms: Some(9_000_000),
        };
        (config, manual)
    }

    #[test]
    fn private_round_trip_preserves_modes_schedules_manual_and_permissions() {
        let path = path("roundtrip");
        let store = Store::at(path.clone());
        let (config, manual) = custom();
        store.save(&config, Some(&manual)).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.recovery, Recovery::None);
        assert_eq!(loaded.config, config);
        assert_eq!(loaded.manual, Some(manual));
        assert!(!format!("{store:?}").contains(path.to_string_lossy().as_ref()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn corrupt_primary_uses_last_good_and_redacts_debug() {
        let path = path("recovery");
        let store = Store::at(path.clone());
        let (config, manual) = custom();
        store.save(&config, Some(&manual)).unwrap();
        std::fs::write(&path, b"private corrupt data 8472").unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.recovery, Recovery::LastGood);
        let debug = format!("{loaded:?}");
        assert!(!debug.contains("8472"));
        assert!(!debug.contains("org.private"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_or_double_corrupt_files_use_safe_defaults() {
        let path = path("defaults");
        let store = Store::at(path.clone());
        let missing = store.load().unwrap();
        assert_eq!(missing.recovery, Recovery::None);
        assert_eq!(missing.config.modes().count(), 4);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"bad").unwrap();
        std::fs::write(store.backup_path(), b"also bad").unwrap();
        let recovered = store.load().unwrap();
        assert_eq!(recovered.recovery, Recovery::Defaults);
        assert_eq!(recovered.config.modes().count(), 4);
        assert!(recovered.manual.is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_mode_references_fail_validation_and_recover() {
        let path = path("invalid-reference");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let invalid = serde_json::json!({
            "version": 1,
            "modes": [{
                "id": "known",
                "name": "Known",
                "allowed_apps": [],
                "allow_urgent": false
            }],
            "schedules": [{
                "id": "bad",
                "mode": "missing",
                "days": ["monday"],
                "start_minute": 10,
                "end_minute": 20,
                "priority": 1,
                "enabled": true
            }],
            "manual": null
        });
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let loaded = Store::at(path.clone()).load().unwrap();
        assert_eq!(loaded.recovery, Recovery::Defaults);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
