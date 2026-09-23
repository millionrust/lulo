//! The Welcome page's greeting: typeset words in several scripts that fade
//! in, rise slightly, hold and fade out (design-lab/setup-assistant.html,
//! all values S). rmac's own design, not Apple's handwritten artwork.

/// The words, in the order they appear.
pub const WORDS: [&str; 14] = [
    "hello",
    "bonjour",
    "hola",
    "hallo",
    "ciao",
    "olá",
    "नमस्ते",
    "こんにちは",
    "你好",
    "안녕하세요",
    "привет",
    "merhaba",
    "hej",
    "γεια σου",
];

/// One word's whole appearance.
pub const WORD_MS: u64 = 2_800;
pub const FADE_MS: u64 = 700;
/// How far a word rises while it fades in, in points.
pub const RISE: f32 = 12.0;

/// The gradient the words walk through (#6FB6FF → #B38CFF → #FF8FB1). GPUI
/// cannot fill text with a gradient, so each word takes the colour at its
/// position along it.
const STOPS: [u32; 3] = [0x6FB6FF, 0xB38CFF, 0xFF8FB1];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub word: usize,
    pub opacity: f32,
    /// Downward offset in points; 0 once the word has risen into place.
    pub offset: f32,
}

/// The greeting at `elapsed_ms` since the page appeared. With
/// `spatial_motion` off (Reduce Motion) words only crossfade.
pub fn frame(elapsed_ms: u64, spatial_motion: bool) -> Frame {
    let word = ((elapsed_ms / WORD_MS) % WORDS.len() as u64) as usize;
    let t = elapsed_ms % WORD_MS;
    let (opacity, offset) = if t < FADE_MS {
        let p = ease_out(t as f32 / FADE_MS as f32);
        (p, RISE * (1.0 - p))
    } else if t < WORD_MS - FADE_MS {
        (1.0, 0.0)
    } else {
        let p = (WORD_MS - t) as f32 / FADE_MS as f32;
        (p, 0.0)
    };
    Frame {
        word,
        opacity,
        offset: if spatial_motion { offset } else { 0.0 },
    }
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// The colour (0xRRGGBB) of word `index`.
pub fn color(index: usize) -> u32 {
    let last = (WORDS.len() - 1) as f32;
    let position = (index % WORDS.len()) as f32 / last * (STOPS.len() - 1) as f32;
    let segment = (position.floor() as usize).min(STOPS.len() - 2);
    let local = position - segment as f32;
    mix(STOPS[segment], STOPS[segment + 1], local)
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let from = ((a >> shift) & 0xFF) as f32;
        let to = ((b >> shift) & 0xFF) as f32;
        ((from + (to - from) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}
