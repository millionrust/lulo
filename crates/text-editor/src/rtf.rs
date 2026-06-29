//! Read-only RTF support: parse an `.rtf` file into styled text runs using
//! macOS's own `NSAttributedString` RTF reader, so the editor can render a
//! faithful formatted preview (bold / italic / underline / color).
//!
//! GPUI has no editable rich-text widget, so this is a *viewer* — editing still
//! happens on the plain-text body. Per-run font size isn't carried by GPUI's
//! `TextRun`, so size variation is not reflected; weight/style/underline/color
//! are.

/// One contiguous run of same-styled text from an RTF document.
#[derive(Clone, Debug, PartialEq)]
pub struct RtfRun {
    pub text: String,
    /// The run's font family (e.g. "Helvetica"), so bold/italic faces render —
    /// GPUI won't synthesize a missing weight, so we use the document's own font.
    pub family: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Option<(u8, u8, u8)>,
}

/// Parse RTF bytes into styled runs. Returns `None` if the data isn't valid RTF
/// (or on non-macOS platforms, where the system reader is unavailable).
#[cfg(target_os = "macos")]
pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
    imp::parse_rtf(bytes)
}

#[cfg(not(target_os = "macos"))]
pub fn parse_rtf(_bytes: &[u8]) -> Option<Vec<RtfRun>> {
    None
}

#[cfg(target_os = "macos")]
mod imp {
    use super::RtfRun;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSAttributedStringAppKitDocumentFormats, NSColor, NSColorSpace, NSFont,
        NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSForegroundColorAttributeName,
        NSUnderlineStyleAttributeName,
    };
    use objc2_foundation::{NSAttributedString, NSData, NSNumber, NSRange, NSString};

    pub fn parse_rtf(bytes: &[u8]) -> Option<Vec<RtfRun>> {
        let data = NSData::with_bytes(bytes);
        let attr: Retained<NSAttributedString> = unsafe {
            NSAttributedString::initWithRTF_documentAttributes(
                NSAttributedString::alloc(),
                &data,
                None,
            )
        }?;

        let ns = attr.string();
        let total = attr.length();
        if total == 0 {
            return Some(Vec::new());
        }

        // Accessing these AppKit extern statics is unsafe; bind once.
        let (font_name, underline_name, color_name) = unsafe {
            (
                NSFontAttributeName,
                NSUnderlineStyleAttributeName,
                NSForegroundColorAttributeName,
            )
        };

        let mut runs: Vec<RtfRun> = Vec::new();
        let mut i: usize = 0;
        while i < total {
            let limit = NSRange { location: i, length: total - i };
            let mut eff = NSRange { location: 0, length: 0 };

            // Split on the *whole* attribute dictionary so a run is constant in
            // font, color, and underline together — otherwise a color change
            // inside one font would be missed.
            let _attrs = unsafe { attr.attributesAtIndex_longestEffectiveRange_inRange(i, &mut eff, limit) };
            let run_range = if eff.length == 0 {
                NSRange { location: i, length: total - i }
            } else {
                eff
            };

            let font_obj = attr_at(&attr, font_name, i, limit);

            let text = ns.substringWithRange(run_range).to_string();

            let (mut bold, mut italic, mut family) = (false, false, None);
            if let Some(obj) = font_obj {
                if let Ok(font) = obj.downcast::<NSFont>() {
                    let traits = font.fontDescriptor().symbolicTraits();
                    bold = traits.contains(NSFontDescriptorSymbolicTraits::TraitBold);
                    italic = traits.contains(NSFontDescriptorSymbolicTraits::TraitItalic);
                    family = font.familyName().map(|n| n.to_string());
                }
            }

            let underline = attr_at(&attr, underline_name, i, limit)
                .and_then(|o| o.downcast::<NSNumber>().ok())
                .map(|n| n.integerValue() != 0)
                .unwrap_or(false);

            let color = attr_at(&attr, color_name, i, limit)
                .and_then(|o| o.downcast::<NSColor>().ok())
                .and_then(|c| color_rgb(&c));

            runs.push(RtfRun { text, family, bold, italic, underline, color });
            i += run_range.length.max(1);
        }
        Some(runs)
    }

    /// Fetch a single attribute object at `i`, ignoring its effective range.
    fn attr_at(
        attr: &NSAttributedString,
        name: &NSString,
        i: usize,
        limit: NSRange,
    ) -> Option<Retained<AnyObject>> {
        let mut scratch = NSRange { location: 0, length: 0 };
        unsafe { attr.attribute_atIndex_longestEffectiveRange_inRange(name, i, &mut scratch, limit) }
    }

    fn color_rgb(c: &NSColor) -> Option<(u8, u8, u8)> {
        let space = NSColorSpace::sRGBColorSpace();
        let rgb = c.colorUsingColorSpace(&space)?;
        let to_u8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some((to_u8(rgb.redComponent()), to_u8(rgb.greenComponent()), to_u8(rgb.blueComponent())))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn parses_bold_italic_runs() {
        // Minimal RTF: plain "Hi " then bold "bold" then plain " and " then italic "it".
        let rtf = br"{\rtf1\ansi {Hi \b bold\b0  and \i it\i0 }}";
        let runs = parse_rtf(rtf).expect("valid rtf");
        let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert!(joined.contains("bold"), "text preserved: {joined:?}");
        assert!(runs.iter().any(|r| r.bold && r.text.contains("bold")), "{runs:?}");
        assert!(runs.iter().any(|r| r.italic && r.text.contains("it")), "{runs:?}");
    }

    #[test]
    fn rejects_non_rtf() {
        assert!(parse_rtf(b"this is not rtf at all").is_none());
    }
}
