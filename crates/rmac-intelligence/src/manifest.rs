//! The models Lulo will load, pinned by size and SHA-256 (ADR 0024 §4
//! "Model download and storage"). The manifest is compiled in, so it is
//! covered by the signed apt repository; the service refuses any file that
//! does not match an entry byte for byte.

/// The model tiers of ADR 0024 §4's hardware gate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Tier {
    /// Qwen3.5-0.8B: the default on 2-core laptops such as the reference PC.
    Tiny,
    /// Qwen3.5-2B: only where the measured decode speed clears its floor.
    Standard,
}

impl Tier {
    pub const ALL: [Tier; 2] = [Tier::Tiny, Tier::Standard];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Standard => "standard",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tier| tier.as_str() == name)
    }

    pub fn model(self) -> &'static Model {
        match self {
            Self::Tiny => &TINY,
            Self::Standard => &STANDARD,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Model {
    pub tier: Tier,
    /// What Settings calls it.
    pub display_name: &'static str,
    /// Revision-pinned download URL (Hugging Face `resolve/<commit>/<file>`).
    pub url: &'static str,
    pub size: u64,
    /// Lower-case hex SHA-256 of the whole file.
    pub sha256: &'static str,
    pub licence: &'static str,
    /// Memory the service may use with this model loaded, in MiB: the
    /// systemd unit's MemoryHigh for this tier, and the free-memory gate's
    /// base. Phase 0 measured 856 MiB (0.8B) and 1,896 MiB (2B) peak RSS.
    pub memory_budget_mib: u64,
}

pub const TINY: Model = Model {
    tier: Tier::Tiny,
    display_name: "Lulo Tiny (Qwen3.5 0.8B)",
    url: "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/6ab461498e2023f6e3c1baea90a8f0fe38ab64d0/Qwen3.5-0.8B-Q4_K_M.gguf",
    size: 532_517_120,
    sha256: "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517",
    licence: "Apache-2.0",
    memory_budget_mib: 1024,
};

pub const STANDARD: Model = Model {
    tier: Tier::Standard,
    display_name: "Lulo Standard (Qwen3.5 2B)",
    url: "https://huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf",
    size: 1_280_835_840,
    sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
    licence: "Apache-2.0",
    memory_budget_mib: 2048,
};

/// Licences a model may carry (ADR 0024 §11 risk 5).
pub const ALLOWED_LICENCES: [&str; 4] = ["Apache-2.0", "MIT", "CC0-1.0", "CC-BY-4.0"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_pinned_and_permissive() {
        for tier in Tier::ALL {
            let model = tier.model();
            assert_eq!(model.tier, tier);
            assert_eq!(Tier::parse(tier.as_str()), Some(tier));
            assert!(model.url.starts_with("https://huggingface.co/"));
            // Pinned to a 40-hex-digit commit, never a branch name.
            let revision = model
                .url
                .split("/resolve/")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .unwrap();
            assert_eq!(revision.len(), 40);
            assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(model.sha256.len(), 64);
            assert!(model
                .sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
            assert!(model.size > 0);
            assert!(ALLOWED_LICENCES.contains(&model.licence));
            assert!(model.memory_budget_mib * 1024 * 1024 > model.size);
        }
    }
}
