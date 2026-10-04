//! Colours a rich document offers: the Colours panel's crayons and Format ▸
//! Font ▸ Highlight's tints. These are document data (they are written into
//! the RTF colour table), not interface colours.

use gpui::Hsla;

use super::model::Rgb;

/// The macOS Colours panel's crayon box, in its own order, by name.
pub const CRAYONS: [(&str, Rgb); 48] = [
    ("Cantaloupe", Rgb::from_u32(0xFFCC66)),
    ("Honeydew", Rgb::from_u32(0xCCFF66)),
    ("Spindrift", Rgb::from_u32(0x66FFCC)),
    ("Sky", Rgb::from_u32(0x66CCFF)),
    ("Lavender", Rgb::from_u32(0xCC66FF)),
    ("Carnation", Rgb::from_u32(0xFF6FCF)),
    ("Licorice", Rgb::from_u32(0x000000)),
    ("Snow", Rgb::from_u32(0xFFFFFF)),
    ("Salmon", Rgb::from_u32(0xFF6666)),
    ("Banana", Rgb::from_u32(0xFFFF66)),
    ("Flora", Rgb::from_u32(0x66FF66)),
    ("Ice", Rgb::from_u32(0x66FFFF)),
    ("Orchid", Rgb::from_u32(0x6666FF)),
    ("Bubblegum", Rgb::from_u32(0xFF66FF)),
    ("Lead", Rgb::from_u32(0x191919)),
    ("Mercury", Rgb::from_u32(0xE6E6E6)),
    ("Tangerine", Rgb::from_u32(0xFF8000)),
    ("Lime", Rgb::from_u32(0x80FF00)),
    ("Sea Foam", Rgb::from_u32(0x00FF80)),
    ("Aqua", Rgb::from_u32(0x0080FF)),
    ("Grape", Rgb::from_u32(0x8000FF)),
    ("Strawberry", Rgb::from_u32(0xFF0080)),
    ("Tungsten", Rgb::from_u32(0x333333)),
    ("Silver", Rgb::from_u32(0xCCCCCC)),
    ("Maraschino", Rgb::from_u32(0xFF0000)),
    ("Lemon", Rgb::from_u32(0xFFFF00)),
    ("Spring", Rgb::from_u32(0x00FF00)),
    ("Turquoise", Rgb::from_u32(0x00FFFF)),
    ("Blueberry", Rgb::from_u32(0x0000FF)),
    ("Magenta", Rgb::from_u32(0xFF00FF)),
    ("Iron", Rgb::from_u32(0x4C4C4C)),
    ("Magnesium", Rgb::from_u32(0xB3B3B3)),
    ("Mocha", Rgb::from_u32(0x804000)),
    ("Fern", Rgb::from_u32(0x408000)),
    ("Moss", Rgb::from_u32(0x008040)),
    ("Ocean", Rgb::from_u32(0x004080)),
    ("Eggplant", Rgb::from_u32(0x400080)),
    ("Maroon", Rgb::from_u32(0x800040)),
    ("Steel", Rgb::from_u32(0x666666)),
    ("Aluminium", Rgb::from_u32(0x999999)),
    ("Cayenne", Rgb::from_u32(0x800000)),
    ("Asparagus", Rgb::from_u32(0x808000)),
    ("Clover", Rgb::from_u32(0x008000)),
    ("Teal", Rgb::from_u32(0x008080)),
    ("Midnight", Rgb::from_u32(0x000080)),
    ("Plum", Rgb::from_u32(0x800080)),
    ("Tin", Rgb::from_u32(0x7F7F7F)),
    ("Nickel", Rgb::from_u32(0x808080)),
];

/// A document colour as a GPUI colour, for swatches.
pub fn to_hsla(color: Rgb) -> Hsla {
    super::layout::hsla(color)
}

/// A highlight tint: `color` laid over white at the strength TextEdit's
/// highlight marker uses, so text stays readable on it.
pub fn highlight_tint(color: Hsla) -> Rgb {
    let rgba = gpui::Rgba::from(color);
    let mix = |channel: f32| ((1.0 - 0.35 + 0.35 * channel.clamp(0.0, 1.0)) * 255.0).round() as u8;
    Rgb::new(mix(rgba.r), mix(rgba.g), mix(rgba.b))
}
