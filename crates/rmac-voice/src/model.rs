//! The talk key's state, apart from drawing it.

use rmac_intelligence::voice::client::VoiceError;

/// How many bars the level meter draws.
pub const BARS: usize = 9;

/// How the key is being used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// The pill has not had the keyboard yet: we do not know whether ⌥ is
    /// still down.
    Undecided,
    /// ⌥ is held: listening ends when it is let go.
    Held,
    /// The key was tapped (let go before the pill had the keyboard, or the
    /// compositor binding only reports presses): listening ends after the
    /// speech and a pause, or on a second press.
    HandsFree,
}

/// What a change of the modifier keys means.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyChange {
    Nothing,
    /// First news of the keyboard: ⌥ is still down.
    Holding,
    /// First news of the keyboard: ⌥ was already let go.
    Tapped,
    /// ⌥ came up while held: stop and transcribe.
    Released,
}

impl Mode {
    /// The mode after the modifiers changed to `alt_down`, and what to do.
    pub fn on_modifiers(self, alt_down: bool) -> (Mode, KeyChange) {
        match (self, alt_down) {
            (Mode::Undecided, true) => (Mode::Held, KeyChange::Holding),
            (Mode::Undecided, false) => (Mode::HandsFree, KeyChange::Tapped),
            (Mode::Held, false) => (Mode::HandsFree, KeyChange::Released),
            (mode, _) => (mode, KeyChange::Nothing),
        }
    }
}

/// The level meter: the newest level in the middle bar, older ones spread
/// outwards, so speech reads as a wave. Updated only when the service sends
/// a level (about 20 times a second while listening); never animated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meter {
    history: [f32; BARS],
}

impl Meter {
    pub fn push(&mut self, level: f64) {
        self.history.rotate_right(1);
        self.history[0] = level.clamp(0.0, 1.0) as f32;
    }

    /// Each bar's height as a fraction of the tallest, 0.15 at rest.
    pub fn bars(&self) -> [f32; BARS] {
        let middle = BARS / 2;
        std::array::from_fn(|index| {
            let age = index.abs_diff(middle);
            let level = self.history[age.min(BARS - 1)];
            // Outer bars are a little shorter, like a voice-memo meter.
            let taper = 1.0 - age as f32 * 0.12;
            (0.15 + 0.85 * level * taper).clamp(0.15, 1.0)
        })
    }
}

/// What the pill says when the capture could not start or found nothing.
/// `None` when nothing needs saying (the user cancelled).
pub fn message_for(error: &VoiceError) -> Option<&'static str> {
    Some(match error {
        VoiceError::Cancelled => return None,
        VoiceError::Off => "Turn on Talk to Lulo in System Settings ▸ Lulo Intelligence.",
        VoiceError::NotDownloaded => {
            "Download the voice model in System Settings ▸ Lulo Intelligence."
        }
        VoiceError::NoRuntime => "Talk to Lulo needs ONNX Runtime (libonnxruntime) installed.",
        VoiceError::NoMicrophone => "Lulo could not open the microphone.",
        VoiceError::Busy => "Lulo is already listening.",
        VoiceError::LowMemory => "Not enough free memory right now.",
        VoiceError::Unavailable | VoiceError::Refused => "Talk to Lulo is not available.",
        VoiceError::Failed => "Lulo could not understand that.",
    })
}

/// What the pill says when nothing was heard.
pub const NOTHING_HEARD: &str = "Lulo didn’t catch that.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_then_letting_go_stops_once() {
        let (mode, change) = Mode::Undecided.on_modifiers(true);
        assert_eq!((mode, change), (Mode::Held, KeyChange::Holding));
        // Other modifiers moving while ⌥ stays down change nothing.
        assert_eq!(mode.on_modifiers(true), (Mode::Held, KeyChange::Nothing));
        let (mode, change) = mode.on_modifiers(false);
        assert_eq!(change, KeyChange::Released);
        // After the release nothing the keys do stops it again.
        assert_eq!(mode.on_modifiers(false).1, KeyChange::Nothing);
        assert_eq!(mode.on_modifiers(true).1, KeyChange::Nothing);
    }

    #[test]
    fn a_tap_listens_hands_free() {
        assert_eq!(
            Mode::Undecided.on_modifiers(false),
            (Mode::HandsFree, KeyChange::Tapped)
        );
        assert_eq!(
            Mode::HandsFree.on_modifiers(true),
            (Mode::HandsFree, KeyChange::Nothing)
        );
    }

    #[test]
    fn the_meter_puts_the_newest_level_in_the_middle() {
        let mut meter = Meter::default();
        assert!(meter.bars().iter().all(|bar| (*bar - 0.15).abs() < 1e-6));
        meter.push(1.0);
        let bars = meter.bars();
        assert!((bars[BARS / 2] - 1.0).abs() < 1e-6);
        assert!((bars[0] - 0.15).abs() < 1e-6);
        meter.push(0.0);
        let bars = meter.bars();
        // The loud step moved one bar out on each side.
        assert!(bars[BARS / 2 - 1] > 0.8 && bars[BARS / 2 + 1] > 0.8);
        assert!((bars[BARS / 2] - 0.15).abs() < 1e-6);
        meter.push(7.0);
        assert!(meter.bars().iter().all(|bar| *bar <= 1.0));
    }

    #[test]
    fn every_failure_has_plain_words_except_cancelling() {
        assert_eq!(message_for(&VoiceError::Cancelled), None);
        for error in [
            VoiceError::Off,
            VoiceError::NotDownloaded,
            VoiceError::NoRuntime,
            VoiceError::NoMicrophone,
            VoiceError::Busy,
            VoiceError::LowMemory,
            VoiceError::Unavailable,
            VoiceError::Refused,
            VoiceError::Failed,
        ] {
            let message = message_for(&error).unwrap();
            assert!(message.ends_with('.'), "{message}");
        }
    }
}
