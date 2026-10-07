//! The user's Lulo Intelligence settings, `$XDG_CONFIG_HOME/rmac/intelligence.json`.
//!
//! Off by default: a missing, unreadable or corrupt file reads as off, so
//! nothing ever loads a model the user did not turn on. System Settings
//! writes it; the service and Spotlight only read it.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::gate::{self, Calibration, Facts};
use crate::manifest::Tier;
use crate::paths;

const MAX_BYTES: usize = 16 * 1024;
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub enabled: bool,
    /// The chosen model tier ("tiny" or "standard"); unset means the gate's
    /// default.
    pub tier: Option<String>,
    pub calibration: Option<CalibrationRecord>,
}

/// A measured decode speed, and the hardware it was measured on: a new CPU
/// or a memory upgrade makes it stale.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct CalibrationRecord {
    pub tiny_decode_tok_s: f64,
    pub standard_decode_tok_s: Option<f64>,
    pub cpu_model: String,
    pub memory_total_mib: u64,
}

impl Config {
    pub fn load() -> Self {
        paths::config_file()
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        rmac_storage::read_bounded_no_follow(path, MAX_BYTES)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = paths::config_file()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no configuration directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            rmac_storage::create_dir_all_private(parent)?;
        }
        let mut saved = self.clone();
        saved.version = VERSION;
        let bytes = serde_json::to_vec_pretty(&saved)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        rmac_storage::atomic_write_private(path, &bytes)
    }

    /// The calibration, if it was measured on this hardware.
    pub fn calibration_for(&self, facts: &Facts) -> Option<Calibration> {
        self.calibration
            .as_ref()
            .filter(|record| {
                record.cpu_model == facts.cpu_model
                    && record.memory_total_mib == facts.memory_total_mib
                    && record.tiny_decode_tok_s > 0.0
            })
            .map(|record| Calibration {
                tiny_decode_tok_s: record.tiny_decode_tok_s,
                standard_decode_tok_s: record.standard_decode_tok_s,
            })
    }

    pub fn decision(&self, facts: &Facts) -> gate::Decision {
        gate::decide(facts, self.calibration_for(facts))
    }

    /// The tier to load: the user's choice when the gate allows it,
    /// otherwise the gate's default. `None` when nothing is offered.
    pub fn tier(&self, facts: &Facts) -> Option<Tier> {
        let decision = self.decision(facts);
        let gate::Decision::Offered { default, .. } = decision else {
            return None;
        };
        Some(
            self.tier
                .as_deref()
                .and_then(Tier::parse)
                .filter(|tier| decision.tier_allowed(*tier))
                .unwrap_or(default),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            memory_total_mib: 6862,
            physical_cores: 2,
            logical_cpus: 4,
            vector_unit: true,
            cpu_model: "Intel(R) Core(TM) i5-5300U CPU @ 2.30GHz".into(),
        }
    }

    #[test]
    fn missing_or_corrupt_settings_are_off() {
        let directory =
            std::env::temp_dir().join(format!("rmac-intelligence-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("rmac/intelligence.json");
        assert!(!Config::load_from(&path).enabled);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(!Config::load_from(&path).enabled);
        let on = Config {
            enabled: true,
            ..Config::default()
        };
        on.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert!(loaded.enabled);
        assert_eq!(loaded.version, VERSION);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_choice_the_gate_forbids_falls_back_to_tiny() {
        let mut config = Config {
            enabled: true,
            tier: Some("standard".into()),
            ..Config::default()
        };
        assert_eq!(config.tier(&facts()), Some(Tier::Tiny));
        config.calibration = Some(CalibrationRecord {
            tiny_decode_tok_s: 40.0,
            standard_decode_tok_s: None,
            cpu_model: facts().cpu_model,
            memory_total_mib: facts().memory_total_mib,
        });
        assert_eq!(config.tier(&facts()), Some(Tier::Standard));
        // Measured on other hardware: ignored.
        let mut upgraded = facts();
        upgraded.memory_total_mib = 16 * 1024;
        assert_eq!(config.tier(&upgraded), Some(Tier::Tiny));
    }
}
