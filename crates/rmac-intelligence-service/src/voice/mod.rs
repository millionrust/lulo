//! `org.rmac.Voice1` (ADR 0024 §8): hold-to-talk speech to text.
//!
//! A sibling of `org.rmac.Intelligence1` with the same model: D-Bus
//! activated by the talk key, the same caller check (only the talk-key HUD
//! may open the microphone), no network, and it exits
//! [`rmac_intelligence::voice::IDLE_EXIT_SECONDS`] after its last capture,
//! which unloads the model.
//!
//! - [`capture`]: the microphone (`pw-record`, 16 kHz mono) while the key is
//!   held, or a WAV file played in real time for tests and measurements.
//! - [`engine`]: the speech-to-text engine behind one small trait, and the
//!   deterministic fixture CI uses.
//! - `moonshine` (Linux): Moonshine through ONNX Runtime.
//! - [`service`]: the bus interface, the capture and model threads and the
//!   idle exit.

pub mod capture;
pub mod engine;
#[cfg(target_os = "linux")]
pub mod moonshine;
pub mod service;
