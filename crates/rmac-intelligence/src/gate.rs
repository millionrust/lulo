//! The hardware gate (ADR 0024 §4, revised by the phase 0 measurements).
//!
//! | Tier | Static requirements | Measured requirement |
//! |---|---|---|
//! | Not offered | no AVX2+FMA+F16C (x86-64), < 2 physical cores, or < 3.5 GiB | — |
//! | Tiny (0.8B, default) | the above | decode ≥ 10 tok/s on the Tiny model |
//! | Standard (2B) | ≥ 6 GiB and ≥ 2 physical cores | 2B decode ≥ 8 tok/s |
//!
//! The 2B decode rate is measured directly when the 2B model is on disk, and
//! otherwise predicted from the Tiny calibration: phase 0 measured 2B at
//! 0.51× the 0.8B rate on the reference laptop (7.3 against 14.4 tok/s),
//! which follows from decode being memory-bandwidth bound and the files'
//! size ratio. The reference laptop therefore gets Tiny, and Standard is
//! offered only on PCs that would decode it at 8 tok/s or better.

use crate::manifest::Tier;

pub const MIN_MEMORY_MIB: u64 = 3584;
pub const STANDARD_MIN_MEMORY_MIB: u64 = 6 * 1024;
pub const MIN_PHYSICAL_CORES: u32 = 2;
/// Tiny's decode floor on 2 threads, tokens per second.
pub const TINY_FLOOR_TOK_S: f64 = 10.0;
/// Standard's decode floor, tokens per second (ADR 0024 phase 0 budget).
pub const STANDARD_FLOOR_TOK_S: f64 = 8.0;
/// 2B decode speed as a share of 0.8B's, measured in phase 0.
pub const STANDARD_SPEED_RATIO: f64 = 0.51;
/// Extra free memory the desktop keeps on top of a model's budget.
pub const MEMORY_HEADROOM_MIB: u64 = 768;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Facts {
    pub memory_total_mib: u64,
    pub physical_cores: u32,
    pub logical_cpus: u32,
    /// AVX2, FMA, F16C and BMI2 on x86-64 (the llama.cpp build's baseline), or
    /// NEON on 64-bit ARM.
    pub vector_unit: bool,
    pub cpu_model: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    /// Measured decode tokens per second on the Tiny model.
    pub tiny_decode_tok_s: f64,
    /// Measured on the Standard model, when it was on disk.
    pub standard_decode_tok_s: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Lulo Intelligence cannot run here; the reason is for Settings.
    NotOffered(String),
    /// Offered. `standard` says whether the 2B model may be chosen.
    Offered {
        default: Tier,
        standard: bool,
        reason: String,
    },
}

impl Decision {
    pub fn tier_allowed(&self, tier: Tier) -> bool {
        match self {
            Self::NotOffered(_) => false,
            Self::Offered { standard, .. } => tier == Tier::Tiny || *standard,
        }
    }
}

pub fn decide(facts: &Facts, calibration: Option<Calibration>) -> Decision {
    let gib = facts.memory_total_mib as f64 / 1024.0;
    if !facts.vector_unit {
        return Decision::NotOffered(
            "This PC's processor lacks the vector instructions (AVX2) the model needs.".into(),
        );
    }
    if facts.physical_cores < MIN_PHYSICAL_CORES {
        return Decision::NotOffered("This PC needs at least two processor cores.".into());
    }
    if facts.memory_total_mib < MIN_MEMORY_MIB {
        return Decision::NotOffered(format!(
            "This PC has {gib:.1} GB of memory; Lulo Intelligence needs 3.5 GB."
        ));
    }
    let Some(calibration) = calibration else {
        return Decision::Offered {
            default: Tier::Tiny,
            standard: false,
            reason: format!(
                "This PC has {gib:.1} GB of memory; Lulo uses the Tiny model until it has measured this PC's speed."
            ),
        };
    };
    if calibration.tiny_decode_tok_s < TINY_FLOOR_TOK_S {
        return Decision::NotOffered(format!(
            "This PC ran the Tiny model at {:.1} tokens a second; Lulo Intelligence needs {:.0}.",
            calibration.tiny_decode_tok_s, TINY_FLOOR_TOK_S
        ));
    }
    let standard_speed = calibration
        .standard_decode_tok_s
        .unwrap_or(calibration.tiny_decode_tok_s * STANDARD_SPEED_RATIO);
    let standard =
        facts.memory_total_mib >= STANDARD_MIN_MEMORY_MIB && standard_speed >= STANDARD_FLOOR_TOK_S;
    let reason = if standard {
        format!("This PC has {gib:.1} GB of memory and is fast enough for the Standard model.")
    } else {
        format!(
            "This PC has {gib:.1} GB of memory; Lulo uses the Tiny model, which answers fastest here."
        )
    };
    Decision::Offered {
        default: Tier::Tiny,
        standard,
        reason,
    }
}

/// Free memory needed before a model of `budget_mib` may load.
pub fn memory_needed_mib(budget_mib: u64) -> u64 {
    budget_mib + MEMORY_HEADROOM_MIB
}

/// Parse `/proc/cpuinfo` and `/proc/meminfo` text.
pub fn facts_from(cpuinfo: &str, meminfo: &str) -> Facts {
    let mut flags = String::new();
    let mut cpu_model = String::new();
    let mut logical = 0u32;
    let mut cores = std::collections::BTreeSet::new();
    let mut physical = None;
    let mut core = None;
    for line in cpuinfo.lines().chain(std::iter::once("")) {
        let (key, value) = line
            .split_once(':')
            .map(|(key, value)| (key.trim(), value.trim()))
            .unwrap_or(("", ""));
        match key {
            "processor" => logical += 1,
            "flags" | "Features" if flags.is_empty() => flags = value.to_owned(),
            "model name" if cpu_model.is_empty() => cpu_model = value.to_owned(),
            "physical id" => physical = Some(value.to_owned()),
            "core id" => core = Some(value.to_owned()),
            "" => {
                if let Some(core) = core.take() {
                    cores.insert((physical.take().unwrap_or_default(), core));
                }
            }
            _ => {}
        }
    }
    let flag = |name: &str| flags.split_whitespace().any(|flag| flag == name);
    let vector_unit = if cfg!(target_arch = "aarch64") {
        flag("asimd")
    } else {
        // The packaged llama.cpp build's baseline (build-native-inputs.sh).
        flag("avx2") && flag("fma") && flag("f16c") && flag("bmi2")
    };
    let memory_total_mib = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .map(|kib| kib / 1024)
        .unwrap_or(0);
    let physical_cores = if cores.is_empty() {
        logical
    } else {
        cores.len() as u32
    };
    Facts {
        memory_total_mib,
        physical_cores,
        logical_cpus: logical,
        vector_unit,
        cpu_model,
    }
}

/// `MemAvailable` in MiB.
pub fn memory_available_mib(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .map(|kib| kib / 1024)
}

/// This PC's facts (Linux; elsewhere nothing is offered).
pub fn current_facts() -> Facts {
    if !cfg!(target_os = "linux") {
        return Facts::default();
    }
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    facts_from(&cpuinfo, &meminfo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference laptop: i5-5300U, 2 cores / 4 threads, 6.7 GiB.
    fn reference_laptop() -> Facts {
        let mut cpuinfo = String::new();
        for (processor, core) in [(0, 0), (1, 0), (2, 1), (3, 1)] {
            cpuinfo.push_str(&format!(
                "processor\t: {processor}\nmodel name\t: Intel(R) Core(TM) i5-5300U CPU @ 2.30GHz\n\
                 physical id\t: 0\ncore id\t\t: {core}\n\
                 flags\t\t: fpu sse2 avx f16c fma avx2 bmi2\n\n"
            ));
        }
        facts_from(
            &cpuinfo,
            "MemTotal:        7027332 kB\nMemAvailable:    3000000 kB\n",
        )
    }

    #[test]
    fn cpuinfo_gives_physical_cores_and_vector_support() {
        let facts = reference_laptop();
        assert_eq!(facts.physical_cores, 2);
        assert_eq!(facts.logical_cpus, 4);
        assert_eq!(facts.memory_total_mib, 6862);
        assert!(facts.cpu_model.contains("i5-5300U"));
        if cfg!(not(target_arch = "aarch64")) {
            assert!(facts.vector_unit);
        }
        assert_eq!(
            memory_available_mib("MemAvailable:    3000000 kB\n"),
            Some(2929)
        );
    }

    #[test]
    fn the_reference_laptop_gets_tiny_and_not_standard() {
        let mut facts = reference_laptop();
        facts.vector_unit = true;
        // Phase 0: 0.8B decoded at 14.4 tok/s, 2B at 7.3.
        let measured = Calibration {
            tiny_decode_tok_s: 14.4,
            standard_decode_tok_s: None,
        };
        match decide(&facts, Some(measured)) {
            Decision::Offered {
                default, standard, ..
            } => {
                assert_eq!(default, Tier::Tiny);
                assert!(!standard, "predicted 2B speed is {:.1}", 14.4 * 0.51);
            }
            other => panic!("{other:?}"),
        }
        let direct = Calibration {
            tiny_decode_tok_s: 14.4,
            standard_decode_tok_s: Some(7.3),
        };
        assert!(!decide(&facts, Some(direct)).tier_allowed(Tier::Standard));
        // Before calibration only Tiny is offered.
        assert!(!decide(&facts, None).tier_allowed(Tier::Standard));
        assert!(decide(&facts, None).tier_allowed(Tier::Tiny));
    }

    #[test]
    fn a_faster_pc_may_choose_standard() {
        let facts = Facts {
            memory_total_mib: 16 * 1024,
            physical_cores: 6,
            logical_cpus: 12,
            vector_unit: true,
            cpu_model: "fast".into(),
        };
        let calibration = Calibration {
            tiny_decode_tok_s: 30.0,
            standard_decode_tok_s: None,
        };
        assert!(decide(&facts, Some(calibration)).tier_allowed(Tier::Standard));
        // ...but not with too little memory.
        let small = Facts {
            memory_total_mib: 4 * 1024,
            ..facts
        };
        assert!(!decide(&small, Some(calibration)).tier_allowed(Tier::Standard));
    }

    #[test]
    fn weak_pcs_are_not_offered() {
        let mut facts = reference_laptop();
        facts.vector_unit = false;
        assert!(matches!(decide(&facts, None), Decision::NotOffered(_)));
        let mut facts = reference_laptop();
        facts.vector_unit = true;
        facts.memory_total_mib = 3000;
        assert!(matches!(decide(&facts, None), Decision::NotOffered(_)));
        let mut facts = reference_laptop();
        facts.vector_unit = true;
        facts.physical_cores = 1;
        assert!(matches!(decide(&facts, None), Decision::NotOffered(_)));
        let mut facts = reference_laptop();
        facts.vector_unit = true;
        let slow = Calibration {
            tiny_decode_tok_s: 6.0,
            standard_decode_tok_s: None,
        };
        assert!(matches!(
            decide(&facts, Some(slow)),
            Decision::NotOffered(_)
        ));
    }
}
