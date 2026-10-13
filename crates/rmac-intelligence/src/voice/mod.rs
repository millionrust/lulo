//! Lulo voice (ADR 0024 §8, "Phase 3 (Voice)"): hold-to-talk speech to
//! text, everything except the model runtime and the microphone (those live
//! in `rmac-intelligence-service`'s `rmac-voice-service`).
//!
//! - [`VoiceModel`]: the checksum-pinned Moonshine files, one model per
//!   tier, downloaded like the language model from Settings ▸ Lulo
//!   Intelligence.
//! - [`availability`]: whether the talk key works on this PC right now.
//! - [`audio`]: WAV reading, resampling, the level meter and the hands-free
//!   end-of-speech detector; [`tokens`]: Moonshine's vocabulary; [`wer`]:
//!   word error rate for the evaluation.
//! - [`handoff`]: how the transcript reaches Spotlight, which runs it
//!   through the same intent pipeline as typed text.
//! - `client` (feature `client`): the `org.rmac.Voice1` client.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::gate::{Decision, Facts};
use crate::manifest::{Pinned, Tier};
use crate::verify::{self, VerifyError};

pub mod audio;
#[cfg(feature = "client")]
pub mod client;
pub mod handoff;
pub mod tokens;
pub mod wer;

/// The voice service's well-known bus name, object path and interface.
pub const BUS_NAME: &str = "org.rmac.Voice1";
pub const OBJECT_PATH: &str = "/org/rmac/Voice1";
pub const INTERFACE_NAME: &str = "org.rmac.Voice1";

/// The service exits this long after its last capture: the model is
/// unloaded with the process, so idle cost is zero. Short, because a
/// capture starts loading the model at once and the load overlaps speech.
pub const IDLE_EXIT_SECONDS: u64 = 10;

/// The longest a single capture runs, whatever the key does.
pub const MAX_CAPTURE_SECONDS: u64 = 15;

/// Moonshine reads 16 kHz mono audio.
pub const SAMPLE_RATE: u32 = 16_000;

/// The talk key: hold ⌥Space (ADR 0024 §8 "Defaults"). `rmac-shortcuts`
/// binds it; the HUD watches this modifier to know when it is let go.
pub const TALK_KEY_DESCRIPTION: &str = "⌥Space";

/// Moonshine's English models' licence (Useful Sensors, MIT), kept beside
/// the downloaded files.
pub const MOONSHINE_LICENCE: &str = "MIT License

Copyright (c) 2024 Useful Sensors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

/// What each of a Moonshine model's files is (the ONNX export sherpa-onnx
/// loads: a preprocessor, an encoder and the decoder twice, without and
/// with its cache).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Preprocess,
    Encode,
    UncachedDecode,
    CachedDecode,
    Tokens,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Preprocess,
        Role::Encode,
        Role::UncachedDecode,
        Role::CachedDecode,
        Role::Tokens,
    ];
}

/// One pinned file of a voice model.
#[derive(Debug, Eq, PartialEq)]
pub struct VoiceFile {
    pub role: Role,
    pub model: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl Pinned for VoiceFile {
    fn url(&self) -> &str {
        self.url
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn sha256(&self) -> &str {
        self.sha256
    }
    fn extension(&self) -> &str {
        match self.role {
            Role::Tokens => "txt",
            _ => "onnx",
        }
    }
    fn display_name(&self) -> &str {
        self.model
    }
    fn licence(&self) -> &str {
        "MIT"
    }
}

/// A speech-to-text model: Moonshine, English only (its English models are
/// MIT; the non-English ones carry a non-commercial licence and are never
/// offered).
#[derive(Debug, Eq, PartialEq)]
pub struct VoiceModel {
    pub tier: Tier,
    pub display_name: &'static str,
    pub licence: &'static str,
    pub files: [VoiceFile; 5],
    /// The voice service's memory budget with this model loaded, in MiB.
    pub memory_budget_mib: u64,
}

macro_rules! moonshine {
    ($size:literal, $commit:literal, [$($role:ident $file:literal $bytes:literal $sha:literal),+ $(,)?]) => {
        [$(VoiceFile {
            role: Role::$role,
            model: concat!("Moonshine ", $size, " (English)"),
            url: concat!(
                "https://huggingface.co/csukuangfj/sherpa-onnx-moonshine-",
                $size,
                "-en-int8/resolve/",
                $commit,
                "/",
                $file
            ),
            size: $bytes,
            sha256: $sha,
        }),+]
    };
}

/// Moonshine Tiny (27M parameters, int8): the Tiny tier, and the reference
/// laptop. sherpa-onnx's export, revision-pinned on Hugging Face.
pub const TINY: VoiceModel = VoiceModel {
    tier: Tier::Tiny,
    display_name: "Moonshine Tiny (English)",
    licence: "MIT",
    files: moonshine!("tiny", "bf2b762c076d8ea61e2af0b3851c9564fb77552e", [
        Preprocess "preprocess.onnx" 6_800_738 "f33addce61a143460fe753b5ee5b7db255e5140b5b779c065b94f6c83ff0bf4e",
        Encode "encode.int8.onnx" 18_249_187 "8774dfba578de027ec6595c2c654a0836434489bc963a0db124a7f181f571acb",
        UncachedDecode "uncached_decode.int8.onnx" 53_216_096 "216737000dd5881a17aa043f6bbd286add33e4c3b0ae257153e2ec15438bdc41",
        CachedDecode "cached_decode.int8.onnx" 45_264_830 "2aff28bba6a03d8dcf5c9feac45462629bae37317442299f28115ad09da773f6",
        Tokens "tokens.txt" 436_688 "1165c2aeb9f72f457a83be2d459a09054f27490acd9b41bd43794dfd25e296ea",
    ]),
    memory_budget_mib: 512,
};

/// Moonshine Base (61M parameters, int8): PCs that run the Standard model.
pub const BASE: VoiceModel = VoiceModel {
    tier: Tier::Standard,
    display_name: "Moonshine Base (English)",
    licence: "MIT",
    files: moonshine!("base", "052b0798ad1bf046a140fdd4efcd9426530fa3f5", [
        Preprocess "preprocess.onnx" 14_077_290 "ffa630d395c5ccf76f5d4954be5b882df76aaf6491519ec01fd82ea7a3819fb2",
        Encode "encode.int8.onnx" 50_311_494 "7e38770f776f2e5583a53b052936005df2ba5c833d7e09c2a5fd796b94bf73e2",
        UncachedDecode "uncached_decode.int8.onnx" 122_120_451 "c01f4b35093bcac20d352d23a75a539e772964579f9d024a90e5e6f09cae9987",
        CachedDecode "cached_decode.int8.onnx" 99_983_837 "2db74e51cedf64a8b1be3c8192e0bb5e4923af0e90bd9e87f8e8771873f8ea03",
        Tokens "tokens.txt" 436_688 "1165c2aeb9f72f457a83be2d459a09054f27490acd9b41bd43794dfd25e296ea",
    ]),
    memory_budget_mib: 768,
};

impl Tier {
    /// The voice model that goes with this tier's language model.
    pub fn voice_model(self) -> &'static VoiceModel {
        match self {
            Tier::Tiny => &TINY,
            Tier::Standard => &BASE,
        }
    }
}

/// The verified paths of a voice model's files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoicePaths {
    pub preprocess: PathBuf,
    pub encode: PathBuf,
    pub uncached_decode: PathBuf,
    pub cached_decode: PathBuf,
    pub tokens: PathBuf,
}

impl VoiceModel {
    /// Every file's bytes together: what Settings shows as the download.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    pub fn file(&self, role: Role) -> &VoiceFile {
        self.files
            .iter()
            .find(|file| file.role == role)
            .unwrap_or(&self.files[0])
    }

    /// Whether every file is on disk and verified (stamps only, no hashing).
    pub fn is_present(&self, models: &Path) -> bool {
        self.files
            .iter()
            .all(|file| verify::is_present(models, file))
    }

    /// Every file's path, hashing only files whose stamp is stale.
    pub fn verified(&self, models: &Path) -> Result<VoicePaths, VerifyError> {
        let path = |role| verify::verified_model(models, self.file(role));
        Ok(VoicePaths {
            preprocess: path(Role::Preprocess)?,
            encode: path(Role::Encode)?,
            uncached_decode: path(Role::UncachedDecode)?,
            cached_decode: path(Role::CachedDecode)?,
            tokens: path(Role::Tokens)?,
        })
    }

    /// Bytes already downloaded, finished files and partial ones together.
    pub fn downloaded_bytes(&self, models: &Path) -> u64 {
        self.files
            .iter()
            .map(|file| {
                if verify::is_present(models, file) {
                    file.size
                } else {
                    crate::fetch::partial_bytes(file)
                }
            })
            .sum()
    }
}

/// A transcript as Spotlight should read it: Moonshine writes sentences
/// ("Turn on dark mode."), and a request reads better without the closing
/// full stop, which no search name contains. Whitespace is collapsed.
pub fn clean_transcript(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    words
        .trim_end_matches(['.', '!', ',', ';', ':', '\u{2026}'])
        .trim_end()
        .to_owned()
}

/// Whether the talk key can be used now, and why not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Availability {
    /// Lulo Intelligence is off.
    Off,
    /// Lulo Intelligence is on, but "Talk to Lulo" is off.
    VoiceOff,
    /// The hardware gate does not offer Lulo Intelligence here.
    NotOffered(String),
    /// The voice model is not downloaded yet.
    NotDownloaded(&'static VoiceModel),
    Ready(&'static VoiceModel),
}

/// Decide from the user's settings, this PC's facts and the models folder.
pub fn availability(config: &Config, facts: &Facts, models: Option<&Path>) -> Availability {
    if !config.enabled {
        return Availability::Off;
    }
    if !config.voice {
        return Availability::VoiceOff;
    }
    if let Decision::NotOffered(reason) = config.decision(facts) {
        return Availability::NotOffered(reason);
    }
    let model = config.tier(facts).unwrap_or(Tier::Tiny).voice_model();
    if models.is_some_and(|models| model.is_present(models)) {
        Availability::Ready(model)
    } else {
        Availability::NotDownloaded(model)
    }
}

/// [`availability`] for this user and PC.
pub fn current_availability() -> Availability {
    let config = Config::load();
    let facts = crate::gate::current_facts();
    let models = crate::paths::models_dir();
    availability(&config, &facts, models.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_voice_file_is_pinned_and_permissive() {
        for model in [&TINY, &BASE] {
            assert_eq!(model.licence, "MIT");
            assert!(crate::manifest::ALLOWED_LICENCES.contains(&model.licence));
            assert_eq!(model.tier.voice_model(), model);
            for (file, role) in model.files.iter().zip(Role::ALL) {
                assert_eq!(file.role, role);
                assert!(file.url.starts_with("https://huggingface.co/csukuangfj/"));
                let revision = file
                    .url
                    .split("/resolve/")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
                    .unwrap();
                assert_eq!(revision.len(), 40);
                assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
                assert_eq!(file.sha256.len(), 64);
                assert!(file
                    .sha256
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')));
                assert!(file.size > 0);
                assert_eq!(
                    file.extension(),
                    if role == Role::Tokens { "txt" } else { "onnx" }
                );
            }
            assert!(model.memory_budget_mib * 1024 * 1024 > model.size());
        }
        // The Tiny model is what Settings offers the reference laptop.
        assert_eq!(TINY.size(), 123_967_539);
    }

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
    fn transcripts_lose_their_closing_full_stop() {
        assert_eq!(
            clean_transcript(" Turn on dark mode. "),
            "Turn on dark mode"
        );
        assert_eq!(clean_transcript("Open  Notes!"), "Open Notes");
        assert_eq!(clean_transcript("What is the time?"), "What is the time?");
        assert_eq!(clean_transcript("Set volume to 30%."), "Set volume to 30%");
        assert_eq!(clean_transcript("..."), "");
        assert_eq!(clean_transcript(""), "");
    }

    #[test]
    fn the_talk_key_needs_both_switches_and_the_model() {
        let mut config = Config::default();
        assert_eq!(availability(&config, &facts(), None), Availability::Off);
        config.enabled = true;
        assert_eq!(
            availability(&config, &facts(), None),
            Availability::VoiceOff
        );
        config.voice = true;
        assert_eq!(
            availability(&config, &facts(), None),
            Availability::NotDownloaded(&TINY)
        );
        let empty = std::env::temp_dir().join(format!("rmac-voice-empty-{}", std::process::id()));
        assert_eq!(
            availability(&config, &facts(), Some(&empty)),
            Availability::NotDownloaded(&TINY)
        );
        let mut weak = facts();
        weak.vector_unit = false;
        assert!(matches!(
            availability(&config, &weak, None),
            Availability::NotOffered(_)
        ));
    }
}
