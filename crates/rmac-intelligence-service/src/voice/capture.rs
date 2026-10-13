//! Where a capture's audio comes from.
//!
//! - The microphone: `pw-record` (PipeWire's own recorder) at 16 kHz mono
//!   16-bit, raw on its stdout. It is a PipeWire capture stream named
//!   "Lulo Voice", so the menu bar's microphone indicator (BAR-09), which
//!   watches the PipeWire graph, shows while it runs. It is started when
//!   the key goes down and killed when it comes up; nothing listens
//!   otherwise.
//! - A WAV file, played in real time (`RMAC_VOICE_CAPTURE_FILE`): the
//!   behaviour scenario and the laptop measurements, which have no
//!   microphone to speak into.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rmac_intelligence::voice::audio::{self, STEP_SAMPLES};

/// One 50 ms step of audio, or the end of it.
pub trait Source: Send {
    /// Fill `out` with the next step (16 kHz mono). `Ok(false)` at the end.
    fn next_step(&mut self, out: &mut Vec<f32>) -> std::io::Result<bool>;
    /// Stop delivering audio (the microphone closes at once).
    fn close(&mut self);
}

/// The test file named by `RMAC_VOICE_CAPTURE_FILE`, if any.
pub fn capture_file() -> Option<PathBuf> {
    std::env::var_os("RMAC_VOICE_CAPTURE_FILE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// Open the configured source: the test file, or the microphone.
pub fn open() -> std::io::Result<Box<dyn Source>> {
    match capture_file() {
        Some(path) => Ok(Box::new(FileSource::open(&path)?)),
        None => Ok(Box::new(Microphone::open()?)),
    }
}

pub struct Microphone {
    child: Option<Child>,
    stdout: Option<std::process::ChildStdout>,
    buffer: Vec<u8>,
}

impl Microphone {
    pub fn open() -> std::io::Result<Self> {
        let mut child = Command::new("pw-record")
            .args([
                "--raw",
                "--rate",
                "16000",
                "--channels",
                "1",
                "--format",
                "s16",
                "--latency",
                "50ms",
                "--media-category",
                "Capture",
                "--properties",
                "{ application.name = \"Lulo Voice\" node.description = \"Lulo Voice\" media.name = \"Talk to Lulo\" }",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take();
        Ok(Self {
            child: Some(child),
            stdout,
            buffer: vec![0u8; STEP_SAMPLES * 2],
        })
    }
}

impl Source for Microphone {
    fn next_step(&mut self, out: &mut Vec<f32>) -> std::io::Result<bool> {
        let Some(stdout) = self.stdout.as_mut() else {
            return Ok(false);
        };
        let mut filled = 0;
        while filled < self.buffer.len() {
            let read = stdout.read(&mut self.buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        out.clear();
        audio::s16le_to_f32(&self.buffer[..filled - filled % 2], out);
        Ok(filled == self.buffer.len())
    }

    fn close(&mut self) {
        self.stdout = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        self.close();
    }
}

/// A WAV file delivered at the speed it was recorded, then silence (a held
/// key keeps "listening" after the speech ends, as with a real microphone).
pub struct FileSource {
    samples: Vec<f32>,
    position: usize,
    started: Instant,
    steps: u64,
}

impl FileSource {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let wav = audio::read_wav(&bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        Ok(Self {
            samples: audio::resample_to_16k(&wav),
            position: 0,
            started: Instant::now(),
            steps: 0,
        })
    }
}

impl Source for FileSource {
    fn next_step(&mut self, out: &mut Vec<f32>) -> std::io::Result<bool> {
        self.steps += 1;
        let due = self.started + Duration::from_millis(self.steps * 50);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        out.clear();
        let end = (self.position + STEP_SAMPLES).min(self.samples.len());
        out.extend_from_slice(&self.samples[self.position..end]);
        self.position = end;
        out.resize(STEP_SAMPLES, 0.0);
        Ok(true)
    }

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_plays_in_real_time_then_falls_silent() {
        let path =
            std::env::temp_dir().join(format!("rmac-voice-capture-{}.wav", std::process::id()));
        std::fs::write(&path, audio::write_wav(&[0.5f32; STEP_SAMPLES])).unwrap();
        let mut source = FileSource::open(&path).unwrap();
        let started = Instant::now();
        let mut step = Vec::new();
        assert!(source.next_step(&mut step).unwrap());
        assert_eq!(step.len(), STEP_SAMPLES);
        assert!(step.iter().all(|sample| (sample - 0.5).abs() < 1e-3));
        assert!(source.next_step(&mut step).unwrap());
        assert!(step.iter().all(|sample| *sample == 0.0));
        assert!(started.elapsed() >= Duration::from_millis(95));
        std::fs::remove_file(&path).unwrap();
    }
}
