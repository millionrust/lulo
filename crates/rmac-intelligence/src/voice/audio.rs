//! Audio for the talk key: 16 kHz mono samples, the HUD's level meter and
//! the hands-free end-of-speech detector. Everything here is plain
//! arithmetic on short buffers; nothing allocates per sample.

use super::SAMPLE_RATE;

/// Little-endian signed 16-bit PCM (what `pw-record --format s16` writes)
/// as samples in [-1, 1].
pub fn s16le_to_f32(bytes: &[u8], out: &mut Vec<f32>) {
    out.extend(
        bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0),
    );
}

/// A decoded WAV file: mono samples at its own rate.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WavError {
    NotWav,
    /// Only 16-bit PCM and 32-bit float are read.
    Unsupported(String),
    Truncated,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotWav => formatter.write_str("not a WAV file"),
            Self::Unsupported(what) => write!(formatter, "unsupported WAV: {what}"),
            Self::Truncated => formatter.write_str("the WAV file is truncated"),
        }
    }
}

impl std::error::Error for WavError {}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a RIFF WAV (PCM 16-bit or IEEE float 32-bit, any channel count,
/// mixed down to mono).
pub fn read_wav(bytes: &[u8]) -> Result<Wav, WavError> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(WavError::NotWav);
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).ok_or(WavError::Truncated)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            let tag = u16_at(bytes, body).ok_or(WavError::Truncated)?;
            let channels = u16_at(bytes, body + 2).ok_or(WavError::Truncated)?;
            let rate = u32_at(bytes, body + 4).ok_or(WavError::Truncated)?;
            let bits = u16_at(bytes, body + 14).ok_or(WavError::Truncated)?;
            // WAVE_FORMAT_EXTENSIBLE carries the real tag in its sub-format.
            let tag = if tag == 0xFFFE {
                u16_at(bytes, body + 24).ok_or(WavError::Truncated)?
            } else {
                tag
            };
            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or(WavError::NotWav)?;
            if channels == 0 || rate == 0 {
                return Err(WavError::Unsupported("no channels".into()));
            }
            let end = body.saturating_add(size).min(bytes.len());
            let data = &bytes[body..end];
            let channels = usize::from(channels);
            let frame: Vec<f32> = match (tag, bits) {
                (1, 16) => data
                    .chunks_exact(2)
                    .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
                    .collect(),
                (3, 32) => data
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect(),
                (tag, bits) => {
                    return Err(WavError::Unsupported(format!("format {tag}, {bits} bits")))
                }
            };
            let samples = frame
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect();
            return Ok(Wav {
                sample_rate: rate,
                samples,
            });
        }
        at = body + size + (size & 1);
    }
    Err(WavError::Truncated)
}

/// Write 16 kHz mono 16-bit PCM as a WAV file's bytes (for test clips).
pub fn write_wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Resample to 16 kHz by linear interpolation. Speech recognisers are
/// insensitive to the small high-frequency loss; used only for files at
/// another rate (the microphone is opened at 16 kHz).
pub fn resample_to_16k(wav: &Wav) -> Vec<f32> {
    if wav.sample_rate == SAMPLE_RATE || wav.samples.is_empty() {
        return wav.samples.clone();
    }
    let ratio = f64::from(wav.sample_rate) / f64::from(SAMPLE_RATE);
    let length = (wav.samples.len() as f64 / ratio).floor() as usize;
    let last = wav.samples.len() - 1;
    (0..length)
        .map(|index| {
            let position = index as f64 * ratio;
            let left = (position.floor() as usize).min(last);
            let right = (left + 1).min(last);
            let fraction = (position - left as f64) as f32;
            wav.samples[left] * (1.0 - fraction) + wav.samples[right] * fraction
        })
        .collect()
}

/// Root-mean-square of a buffer.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The part of `samples` with speech in it, plus `margin` samples of the
/// quiet on each side: hold-to-talk records the pause before the first
/// word and after the last, and Moonshine (trained on speech segments a
/// voice-activity detector cut) often ends at once when a clip starts with
/// half a second of silence. A step is speech when it is louder than both
/// 0.01 RMS and four times the quietest step. Nothing loud: all of it.
pub fn trim_silence(samples: &[f32], margin: usize) -> &[f32] {
    let steps: Vec<f32> = samples.chunks(STEP_SAMPLES).map(rms).collect();
    let floor = steps.iter().copied().fold(f32::INFINITY, f32::min);
    let threshold = (floor * 4.0).max(0.01);
    let first = steps.iter().position(|level| *level > threshold);
    let last = steps.iter().rposition(|level| *level > threshold);
    let (Some(first), Some(last)) = (first, last) else {
        return samples;
    };
    let start = (first * STEP_SAMPLES).saturating_sub(margin);
    let end = ((last + 1) * STEP_SAMPLES + margin).min(samples.len());
    &samples[start..end]
}

/// Scale `samples` so the loudest one reaches `peak`, by at most
/// `max_gain`: a quiet microphone and a loud one give the recogniser the
/// same level (Moonshine ends early on quiet input).
pub fn normalise_peak(samples: &[f32], peak: f32, max_gain: f32) -> Vec<f32> {
    let loudest = samples
        .iter()
        .fold(0.0f32, |top, sample| top.max(sample.abs()));
    let gain = if loudest > 0.0 {
        (peak / loudest).min(max_gain)
    } else {
        1.0
    };
    samples.iter().map(|sample| sample * gain).collect()
}

/// Quiet put back around the speech (see [`prepare_for_recognition`]).
pub const SPEECH_PADDING: usize = (SAMPLE_RATE / 5) as usize;

/// A capture as Moonshine reads it best: raised to one level, cut to the
/// speech, with [`SPEECH_PADDING`] of silence on each side. Measured on the
/// reference laptop's 30-clip set (ADR 0024 "Phase 3 (Voice) results"):
/// unpadded speech, or speech after half a second of quiet, often decoded
/// to nothing. Empty when there is under 0.1 s of speech.
pub fn prepare_for_recognition(samples: &[f32]) -> Vec<f32> {
    let level = normalise_peak(samples, 0.9, 30.0);
    let speech = trim_silence(&level, 0);
    if speech.len() < (SAMPLE_RATE / 10) as usize
        || (speech.len() == level.len() && rms(speech) < 0.01)
    {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(speech.len() + 2 * SPEECH_PADDING);
    out.resize(SPEECH_PADDING, 0.0);
    out.extend_from_slice(speech);
    out.resize(out.len() + SPEECH_PADDING, 0.0);
    out
}

/// The HUD's meter, 0 to 1: loudness from -55 dBFS (silence) to -10 dBFS
/// (a raised voice close to the microphone), as the Mac's level meter
/// moves with speech rather than with peaks.
pub fn meter_level(samples: &[f32]) -> f64 {
    let level = f64::from(rms(samples));
    if level <= 0.0 {
        return 0.0;
    }
    let decibels = 20.0 * level.log10();
    ((decibels + 55.0) / 45.0).clamp(0.0, 1.0)
}

/// How long a meter or endpoint step is: 50 ms of audio.
pub const STEP_SAMPLES: usize = (SAMPLE_RATE / 20) as usize;

/// Hands-free end of speech: the talk key was tapped rather than held
/// (niri's fallback binding sees only the press, ADR 0024 §8), so the
/// capture ends after speech followed by a pause. An energy detector with
/// an adaptive noise floor: a few multiplications per 50 ms step.
#[derive(Clone, Debug)]
pub struct Endpointer {
    noise: f32,
    speech_steps: u32,
    silence_steps: u32,
    steps: u32,
}

/// Speech must last this many steps (200 ms) before a pause can end it.
const MIN_SPEECH_STEPS: u32 = 4;
/// A pause this long (900 ms) after speech ends the capture.
const END_SILENCE_STEPS: u32 = 18;
/// With no speech at all, give up after 6 s.
const NO_SPEECH_STEPS: u32 = 120;

impl Default for Endpointer {
    fn default() -> Self {
        Self {
            noise: 0.003,
            speech_steps: 0,
            silence_steps: 0,
            steps: 0,
        }
    }
}

impl Endpointer {
    /// Feed one step; true once the capture should end.
    pub fn step(&mut self, samples: &[f32]) -> bool {
        self.steps += 1;
        let level = rms(samples);
        let speaking = level > (self.noise * 4.0).max(0.01);
        if speaking {
            self.speech_steps += 1;
            self.silence_steps = 0;
        } else {
            // The floor follows quiet steps only, slowly.
            self.noise = self.noise * 0.9 + level * 0.1;
            if self.speech_steps > 0 {
                self.silence_steps += 1;
            }
        }
        (self.speech_steps >= MIN_SPEECH_STEPS && self.silence_steps >= END_SILENCE_STEPS)
            || (self.speech_steps == 0 && self.steps >= NO_SPEECH_STEPS)
    }

    /// Whether any speech was heard.
    pub fn heard_speech(&self) -> bool {
        self.speech_steps >= MIN_SPEECH_STEPS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f32, amplitude: f32) -> Vec<f32> {
        let count = (seconds * SAMPLE_RATE as f32) as usize;
        (0..count)
            .map(|index| {
                amplitude * (index as f32 * 2.0 * std::f32::consts::PI * 220.0 / 16_000.0).sin()
            })
            .collect()
    }

    #[test]
    fn wav_round_trips_and_mixes_down() {
        let samples = tone(0.25, 0.5);
        let wav = read_wav(&write_wav(&samples)).unwrap();
        assert_eq!(wav.sample_rate, SAMPLE_RATE);
        assert_eq!(wav.samples.len(), samples.len());
        for (read, written) in wav.samples.iter().zip(&samples) {
            assert!((read - written).abs() < 1e-3);
        }
        // A stereo 22.05 kHz float file: mixed to mono, resampled to 16 kHz.
        let mut stereo = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        stereo.extend_from_slice(&16u32.to_le_bytes());
        stereo.extend_from_slice(&3u16.to_le_bytes());
        stereo.extend_from_slice(&2u16.to_le_bytes());
        stereo.extend_from_slice(&22_050u32.to_le_bytes());
        stereo.extend_from_slice(&(22_050u32 * 8).to_le_bytes());
        stereo.extend_from_slice(&8u16.to_le_bytes());
        stereo.extend_from_slice(&32u16.to_le_bytes());
        stereo.extend_from_slice(b"LIST");
        stereo.extend_from_slice(&3u32.to_le_bytes());
        stereo.extend_from_slice(b"abc\0");
        stereo.extend_from_slice(b"data");
        stereo.extend_from_slice(&(22_050u32 * 8).to_le_bytes());
        for _ in 0..22_050 {
            stereo.extend_from_slice(&0.25f32.to_le_bytes());
            stereo.extend_from_slice(&0.75f32.to_le_bytes());
        }
        let wav = read_wav(&stereo).unwrap();
        assert_eq!(wav.sample_rate, 22_050);
        assert!(wav.samples.iter().all(|sample| (sample - 0.5).abs() < 1e-6));
        let resampled = resample_to_16k(&wav);
        assert!((15_999..=16_000).contains(&resampled.len()));
        assert!(resampled.iter().all(|sample| (sample - 0.5).abs() < 1e-5));
        assert_eq!(read_wav(b"not a wav file at all"), Err(WavError::NotWav));
    }

    #[test]
    fn silence_around_speech_is_trimmed_to_a_margin() {
        let quiet = vec![0.001f32; STEP_SAMPLES * 10];
        let speech = tone(0.5, 0.2);
        let clip: Vec<f32> = quiet.iter().chain(&speech).chain(&quiet).copied().collect();
        let trimmed = trim_silence(&clip, 1_600);
        assert_eq!(trimmed.len(), speech.len() + 3_200);
        assert!(trimmed[1_600..1_600 + 800]
            .iter()
            .any(|sample| sample.abs() > 0.1));
        // All quiet, or all loud: nothing to cut.
        assert_eq!(trim_silence(&quiet, 1_600).len(), quiet.len());
        assert_eq!(trim_silence(&speech, 1_600).len(), speech.len());
        assert!(trim_silence(&[], 1_600).is_empty());
    }

    #[test]
    fn recognition_gets_level_speech_between_fixed_pauses() {
        let quiet = vec![0.0005f32; STEP_SAMPLES * 12];
        let speech = tone(0.5, 0.05);
        let clip: Vec<f32> = quiet.iter().chain(&speech).chain(&quiet).copied().collect();
        let prepared = prepare_for_recognition(&clip);
        assert_eq!(prepared.len(), speech.len() + 2 * SPEECH_PADDING);
        assert!(prepared[..SPEECH_PADDING]
            .iter()
            .all(|sample| *sample == 0.0));
        let peak = prepared
            .iter()
            .fold(0.0f32, |top, sample| top.max(sample.abs()));
        assert!((peak - 0.9).abs() < 1e-3, "{peak}");
        // Under 0.1 s of speech, or none: nothing to recognise.
        assert!(prepare_for_recognition(&tone(0.05, 0.2)).is_empty());
        assert!(prepare_for_recognition(&[0.0; 16_000]).is_empty());
    }

    #[test]
    fn quiet_speech_is_raised_to_one_level() {
        let louder = normalise_peak(&[0.1, -0.2, 0.05], 0.9, 30.0);
        assert!((louder[1] + 0.9).abs() < 1e-6);
        assert!((louder[0] - 0.45).abs() < 1e-6);
        // Near-silence is not blown up into noise.
        let hiss = normalise_peak(&[0.001, -0.001], 0.9, 30.0);
        assert!((hiss[0] - 0.03).abs() < 1e-6);
        assert_eq!(normalise_peak(&[0.0, 0.0], 0.9, 30.0), [0.0, 0.0]);
    }

    #[test]
    fn pcm_bytes_and_meter() {
        let mut out = Vec::new();
        s16le_to_f32(&[0x00, 0x80, 0xFF, 0x7F, 0x00, 0x00], &mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], -1.0);
        assert!((out[1] - 32767.0 / 32768.0).abs() < 1e-6);
        assert_eq!(meter_level(&[0.0; 800]), 0.0);
        assert_eq!(meter_level(&[]), 0.0);
        let quiet = meter_level(&tone(0.05, 0.003));
        let loud = meter_level(&tone(0.05, 0.3));
        assert!(quiet < 0.2, "{quiet}");
        assert!(loud > 0.8, "{loud}");
        assert!(meter_level(&tone(0.05, 1.0)) <= 1.0);
    }

    #[test]
    fn hands_free_capture_ends_after_speech_and_a_pause() {
        let mut endpointer = Endpointer::default();
        let silence = vec![0.001f32; STEP_SAMPLES];
        let speech = tone(0.05, 0.2);
        let mut ended_at = None;
        let script: Vec<&[f32]> = std::iter::repeat_n(silence.as_slice(), 10)
            .chain(std::iter::repeat_n(speech.as_slice(), 20))
            .chain(std::iter::repeat_n(silence.as_slice(), 40))
            .collect();
        for (index, step) in script.iter().enumerate() {
            if endpointer.step(step) {
                ended_at = Some(index);
                break;
            }
        }
        assert!(endpointer.heard_speech());
        // 10 quiet + 20 speech + 18 quiet steps.
        assert_eq!(ended_at, Some(47));

        // A short pause mid-sentence does not end it.
        let mut endpointer = Endpointer::default();
        for step in std::iter::repeat_n(speech.as_slice(), 10)
            .chain(std::iter::repeat_n(silence.as_slice(), 10))
            .chain(std::iter::repeat_n(speech.as_slice(), 10))
        {
            assert!(!endpointer.step(step));
        }

        // Nothing said: it gives up after 6 s.
        let mut endpointer = Endpointer::default();
        let steps = (0..200).position(|_| endpointer.step(&silence)).unwrap();
        assert_eq!(steps + 1, 120);
        assert!(!endpointer.heard_speech());
    }
}
