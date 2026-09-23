//! PDF support through poppler-utils, run as subprocesses.
//!
//! Why poppler's command-line tools rather than a library or a pure-Rust
//! renderer: `pdftoppm`, `pdfinfo` and `pdftotext` ship with Ubuntu's
//! desktop (poppler-utils) and are the most complete, most battle-tested PDF
//! engine available there (fonts, CJK, encryption, broken files). Running
//! them as child processes keeps a crash or a hostile file out of Preview's
//! process, needs no C build or FFI in rmac, and costs ~10 ms of process
//! start per page — invisible next to rasterising. Pure-Rust renderers
//! (pdf-render, hayro) still miss fonts and features real PDFs use, and
//! pdfium/mupdf bindings add a native build and a large binary. Pages render
//! lazily (only visible pages plus one neighbour) and are cached per page,
//! scale and rotation, so a low-end PC never rasterises more than it shows.
//!
//! This module holds the pure parts: argument lists and parsers for the
//! tools' output. Running them lives in the binary.

use std::ffi::OsString;
use std::path::Path;

use crate::layout::UnitRect;

pub const PDFINFO: &str = "pdfinfo";
pub const PDFTOPPM: &str = "pdftoppm";
pub const PDFTOTEXT: &str = "pdftotext";

/// Largest bitmap side rmac asks pdftoppm for (keeps a page texture within
/// what low-end GPUs accept).
pub const MAX_RENDER_SIDE: f32 = 8192.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfInfo {
    pub version: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
    pub encrypted: bool,
    pub pages: Vec<PageBox>,
}

/// A page's displayed box in points, before the viewer's own rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBox {
    pub width: f32,
    pub height: f32,
    /// The page's /Rotate in degrees (pdftoppm already applies it).
    pub rotation: i32,
}

impl PageBox {
    /// Size as rendered by pdftoppm (with /Rotate applied).
    pub fn displayed(&self) -> (f32, f32) {
        crate::layout::Rotation::from_degrees(self.rotation).apply((self.width, self.height))
    }
}

/// `pdfinfo -isodates -box -f 1 -l N FILE`.
pub fn info_args(path: &Path, pages: Option<usize>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-isodates".into()];
    if let Some(pages) = pages.filter(|pages| *pages > 0) {
        args.extend([
            "-box".into(),
            "-f".into(),
            "1".into(),
            "-l".into(),
            pages.to_string().into(),
        ]);
    }
    args.push(path.as_os_str().to_owned());
    args
}

/// Parse `pdfinfo` output. The first call (no page range) yields metadata
/// and the page count; with a page range each page gains its size, crop box
/// and rotation.
pub fn parse_info(text: &str) -> Result<PdfInfo, String> {
    let mut info = PdfInfo::default();
    let mut count = None;
    let mut default_size = None;
    let mut sizes: Vec<Option<(f32, f32)>> = Vec::new();
    let mut crops: Vec<Option<(f32, f32)>> = Vec::new();
    let mut rotations: Vec<i32> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let optional = || (!value.is_empty()).then(|| value.to_owned());
        let per_page = key.strip_prefix("Page ").and_then(|rest| {
            let mut words = rest.split_whitespace();
            let number = words.next()?.parse::<usize>().ok()?;
            Some((number, words.next()?))
        });
        if let Some((number, field)) = per_page {
            let Some(index) = number.checked_sub(1).filter(|index| *index < 100_000) else {
                continue;
            };
            if sizes.len() <= index {
                sizes.resize(index + 1, None);
                crops.resize(index + 1, None);
                rotations.resize(index + 1, 0);
            }
            match field {
                "size" => sizes[index] = parse_size(value),
                "rot" => rotations[index] = value.parse().unwrap_or(0),
                "CropBox" => crops[index] = parse_box(value),
                _ => {}
            }
            continue;
        }
        match key.trim() {
            "Pages" => count = value.parse::<usize>().ok(),
            "Page size" => default_size = parse_size(value),
            "Page rot" if rotations.is_empty() => rotations.push(value.parse().unwrap_or(0)),
            "PDF version" => info.version = optional(),
            "Title" => info.title = optional(),
            "Author" => info.author = optional(),
            "Subject" => info.subject = optional(),
            "Creator" => info.creator = optional(),
            "Producer" => info.producer = optional(),
            "CreationDate" => info.creation_date = optional(),
            "ModDate" => info.modification_date = optional(),
            "Encrypted" => info.encrypted = value.starts_with("yes"),
            _ => {}
        }
    }
    let count = count.ok_or_else(|| "pdfinfo did not report a page count".to_owned())?;
    if count == 0 {
        return Err("the PDF has no pages".into());
    }
    let fallback = default_size.or_else(|| sizes.iter().flatten().next().copied());
    let fallback = fallback.ok_or_else(|| "pdfinfo did not report a page size".to_owned())?;
    info.pages = (0..count)
        .map(|index| {
            let (width, height) = crops
                .get(index)
                .copied()
                .flatten()
                .or_else(|| sizes.get(index).copied().flatten())
                .unwrap_or(fallback);
            PageBox {
                width,
                height,
                rotation: rotations.get(index).copied().unwrap_or(0),
            }
        })
        .collect();
    Ok(info)
}

/// "612 x 792 pts (letter)" → (612, 792).
fn parse_size(value: &str) -> Option<(f32, f32)> {
    let mut words = value.split_whitespace();
    let width = words.next()?.parse::<f32>().ok()?;
    (words.next()? == "x").then_some(())?;
    let height = words.next()?.parse::<f32>().ok()?;
    positive((width, height))
}

/// "0.00 0.00 612.00 792.00" → (612, 792).
fn parse_box(value: &str) -> Option<(f32, f32)> {
    let numbers: Vec<f32> = value
        .split_whitespace()
        .filter_map(|word| word.parse().ok())
        .collect();
    let [x0, y0, x1, y1] = numbers[..] else {
        return None;
    };
    positive(((x1 - x0).abs(), (y1 - y0).abs()))
}

fn positive(size: (f32, f32)) -> Option<(f32, f32)> {
    (size.0.is_finite() && size.1.is_finite() && size.0 > 0.0 && size.1 > 0.0).then_some(size)
}

/// Resolution for rendering a page at `pixel_scale` device pixels per point,
/// capped so neither side exceeds [`MAX_RENDER_SIDE`].
pub fn render_dpi(page: (f32, f32), pixel_scale: f32) -> f32 {
    let longest = page.0.max(page.1).max(1.0);
    let scale = pixel_scale.min(MAX_RENDER_SIDE / longest).max(0.01);
    72.0 * scale
}

/// `pdftoppm -f N -l N -r DPI -cropbox FILE` (PPM on stdout).
pub fn render_args(path: &Path, page_index: usize, dpi: f32) -> Vec<OsString> {
    let page = (page_index + 1).to_string();
    vec![
        "-f".into(),
        page.clone().into(),
        "-l".into(),
        page.into(),
        "-r".into(),
        format!("{dpi:.3}").into(),
        "-cropbox".into(),
        path.as_os_str().to_owned(),
    ]
}

/// `pdftotext -bbox -enc UTF-8 FILE -` (word boxes as XHTML on stdout).
pub fn text_args(path: &Path) -> Vec<OsString> {
    vec![
        "-bbox".into(),
        "-enc".into(),
        "UTF-8".into(),
        path.as_os_str().to_owned(),
        "-".into(),
    ]
}

/// A binary PPM (P6, maxval 255): width, height and the RGB bytes.
pub fn parse_ppm(bytes: &[u8]) -> Option<(u32, u32, &[u8])> {
    let mut fields = Vec::with_capacity(4);
    let mut index = 0;
    while fields.len() < 4 {
        // Skip whitespace and comments.
        loop {
            match bytes.get(index)? {
                b'#' => {
                    while *bytes.get(index)? != b'\n' {
                        index += 1;
                    }
                }
                byte if byte.is_ascii_whitespace() => index += 1,
                _ => break,
            }
        }
        let start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            index += 1;
        }
        fields.push(std::str::from_utf8(&bytes[start..index]).ok()?);
    }
    // Exactly one whitespace byte separates the header from the raster.
    index += 1;
    if fields[0] != "P6" || fields[3] != "255" {
        return None;
    }
    let width: u32 = fields[1].parse().ok()?;
    let height: u32 = fields[2].parse().ok()?;
    let length = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(3)?;
    let raster = bytes.get(index..index.checked_add(length)?)?;
    (width > 0 && height > 0).then_some((width, height, raster))
}

/// Text of one page with word boxes in unit coordinates (0‥1, top-left).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextPage {
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub rect: UnitRect,
}

/// Parse `pdftotext -bbox` XHTML.
pub fn parse_bbox(xhtml: &str) -> Vec<TextPage> {
    let mut pages = Vec::new();
    let mut size = (1.0_f32, 1.0_f32);
    let mut rest = xhtml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        if let Some(attributes) = tag.strip_prefix("page ") {
            size = (
                attribute(attributes, "width").unwrap_or(1.0).max(1.0),
                attribute(attributes, "height").unwrap_or(1.0).max(1.0),
            );
            pages.push(TextPage::default());
        } else if let Some(attributes) = tag.strip_prefix("word ") {
            let Some(close) = rest.find("</word>") else {
                break;
            };
            let text = decode_entities(&rest[..close]);
            rest = &rest[close + "</word>".len()..];
            let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
                attribute(attributes, "xMin"),
                attribute(attributes, "yMin"),
                attribute(attributes, "xMax"),
                attribute(attributes, "yMax"),
            ) else {
                continue;
            };
            if let Some(page) = pages.last_mut() {
                page.words.push(Word {
                    text,
                    rect: UnitRect {
                        x0: x0 / size.0,
                        y0: y0 / size.1,
                        x1: x1 / size.0,
                        y1: y1 / size.1,
                    },
                });
            }
        }
    }
    pages
}

fn attribute(attributes: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=\"");
    let start = attributes.find(&key)? + key.len();
    let end = attributes[start..].find('"')? + start;
    attributes[start..end].parse().ok()
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// One search hit: its page and the (partial) word boxes it covers.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub page: usize,
    pub rects: Vec<UnitRect>,
}

/// Case-insensitive search across word boundaries (the query's whitespace
/// matches the gap between words).
pub fn search(pages: &[TextPage], query: &str) -> Vec<Match> {
    let needle: Vec<char> = normalise(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        // Page text as lower-case chars with one space between words, and
        // for each char the word it came from and its index in that word.
        let mut haystack: Vec<char> = Vec::new();
        let mut origin: Vec<Option<(usize, usize)>> = Vec::new();
        for (word_index, word) in page.words.iter().enumerate() {
            if word_index > 0 {
                haystack.push(' ');
                origin.push(None);
            }
            for (char_index, character) in word.text.chars().enumerate() {
                for lower in character.to_lowercase() {
                    haystack.push(lower);
                    origin.push(Some((word_index, char_index)));
                }
            }
        }
        let mut start = 0;
        while start + needle.len() <= haystack.len() {
            if haystack[start..start + needle.len()] != needle[..] {
                start += 1;
                continue;
            }
            let mut rects = Vec::new();
            let mut spans: Vec<(usize, usize, usize)> = Vec::new();
            for (word, char_index) in origin[start..start + needle.len()].iter().flatten() {
                match spans.last_mut() {
                    Some((last, _, end)) if *last == *word => *end = *char_index,
                    _ => spans.push((*word, *char_index, *char_index)),
                }
            }
            for (word_index, first, last) in spans {
                let word = &page.words[word_index];
                let length = word.text.chars().count().max(1) as f32;
                let width = word.rect.x1 - word.rect.x0;
                rects.push(UnitRect {
                    x0: word.rect.x0 + width * first as f32 / length,
                    x1: word.rect.x0 + width * (last + 1) as f32 / length,
                    y0: word.rect.y0,
                    y1: word.rect.y1,
                });
            }
            matches.push(Match {
                page: page_index,
                rects,
            });
            start += needle.len();
        }
    }
    matches
}

fn normalise(query: &str) -> Vec<char> {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

/// A user-facing explanation when a poppler tool cannot run.
pub fn missing_tool_message(tool: &str) -> String {
    format!("Preview needs “{tool}” from poppler-utils to open PDF documents.")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = "Title:           Report &amp; Notes\n\
Author:          \n\
Creator:         Writer\n\
Producer:        LibreOffice 7.6\n\
CreationDate:    2026-09-23T13:00:00+05:30\n\
Tagged:          no\n\
Encrypted:       no\n\
Pages:           3\n\
Page size:       612 x 792 pts (letter)\n\
Page rot:        0\n\
PDF version:     1.4\n";

    const PAGES: &str = "Pages:           3\n\
Page    1 size:  612 x 792 pts (letter)\n\
Page    1 rot:   0\n\
Page    1 MediaBox:     0.00     0.00   612.00   792.00\n\
Page    1 CropBox:      0.00     0.00   612.00   792.00\n\
Page    2 size:  842 x 595 pts (A4)\n\
Page    2 rot:   90\n\
Page    3 size:  612 x 792 pts (letter)\n\
Page    3 rot:   0\n\
Page    3 CropBox:     36.00    36.00   576.00   756.00\n";

    #[test]
    fn parses_summary_metadata() {
        let info = parse_info(SUMMARY).unwrap();
        assert_eq!(info.version.as_deref(), Some("1.4"));
        assert_eq!(info.title.as_deref(), Some("Report &amp; Notes"));
        assert_eq!(info.author, None);
        assert_eq!(info.creator.as_deref(), Some("Writer"));
        assert_eq!(info.producer.as_deref(), Some("LibreOffice 7.6"));
        assert_eq!(
            info.creation_date.as_deref(),
            Some("2026-09-23T13:00:00+05:30")
        );
        assert!(!info.encrypted);
        assert_eq!(info.pages.len(), 3);
        assert_eq!(info.pages[2].width, 612.0);
    }

    #[test]
    fn parses_per_page_boxes_and_rotation() {
        let info = parse_info(PAGES).unwrap();
        assert_eq!(info.pages[0].displayed(), (612.0, 792.0));
        assert_eq!(info.pages[1].rotation, 90);
        assert_eq!(info.pages[1].displayed(), (595.0, 842.0));
        // The crop box wins over the media size.
        assert_eq!(info.pages[2].displayed(), (540.0, 720.0));
    }

    #[test]
    fn rejects_output_without_pages() {
        assert!(parse_info("Title: x\n").is_err());
        assert!(parse_info("Pages: 0\nPage size: 1 x 1 pts\n").is_err());
        assert!(parse_info("Pages: 2\n").is_err());
    }

    #[test]
    fn builds_tool_arguments() {
        let path = Path::new("/docs/a.pdf");
        assert_eq!(
            info_args(path, None),
            ["-isodates", "/docs/a.pdf"].map(OsString::from)
        );
        assert_eq!(
            info_args(path, Some(3)),
            ["-isodates", "-box", "-f", "1", "-l", "3", "/docs/a.pdf"].map(OsString::from)
        );
        assert_eq!(
            render_args(path, 1, 144.0),
            [
                "-f",
                "2",
                "-l",
                "2",
                "-r",
                "144.000",
                "-cropbox",
                "/docs/a.pdf"
            ]
            .map(OsString::from)
        );
        assert_eq!(
            text_args(path),
            ["-bbox", "-enc", "UTF-8", "/docs/a.pdf", "-"].map(OsString::from)
        );
    }

    #[test]
    fn render_dpi_caps_the_bitmap() {
        assert_eq!(render_dpi((612.0, 792.0), 2.0), 144.0);
        let capped = render_dpi((612.0, 792.0), 100.0);
        assert!((792.0 * capped / 72.0 - MAX_RENDER_SIDE).abs() < 0.5);
    }

    #[test]
    fn parses_binary_ppm() {
        let mut bytes = b"P6\n# poppler\n2 1\n255\n".to_vec();
        bytes.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
        let (width, height, raster) = parse_ppm(&bytes).unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(raster, &[255, 0, 0, 0, 255, 0]);
        assert!(parse_ppm(b"P6\n2 1\n255\n\x00").is_none());
        assert!(parse_ppm(b"P5\n1 1\n255\n\x00").is_none());
        assert!(parse_ppm(b"").is_none());
    }

    const BBOX: &str = r#"<html><body><doc>
<page width="612.000000" height="792.000000">
    <word xMin="72.000000" yMin="72.000000" xMax="172.000000" yMax="108.000000">Page</word>
    <word xMin="180.000000" yMin="72.000000" xMax="200.000000" yMax="108.000000">1</word>
    <word xMin="72.000000" yMin="120.000000" xMax="120.000000" yMax="134.000000">Fish&amp;Chips</word>
</page>
<page width="612.000000" height="792.000000">
    <word xMin="72.000000" yMin="72.000000" xMax="172.000000" yMax="108.000000">page</word>
</page>
</doc></body></html>"#;

    #[test]
    fn parses_word_boxes_in_unit_coordinates() {
        let pages = parse_bbox(BBOX);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].words.len(), 3);
        assert_eq!(pages[0].words[2].text, "Fish&Chips");
        let rect = pages[0].words[0].rect;
        assert!((rect.x0 - 72.0 / 612.0).abs() < 1e-6);
        assert!((rect.y1 - 108.0 / 792.0).abs() < 1e-6);
    }

    #[test]
    fn search_is_case_insensitive_and_spans_words() {
        let pages = parse_bbox(BBOX);
        let hits = search(&pages, "PAGE");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].page, 0);
        assert_eq!(hits[1].page, 1);
        let across = search(&pages, "page   1");
        assert_eq!(across.len(), 1);
        assert_eq!(across[0].rects.len(), 2);
        // A partial word highlights its share of the box.
        let partial = search(&pages, "chips");
        assert_eq!(partial.len(), 1);
        let rect = partial[0].rects[0];
        let word = pages[0].words[2].rect;
        let fraction = (rect.x0 - word.x0) / (word.x1 - word.x0);
        assert!((fraction - 0.5).abs() < 1e-5);
        assert!((rect.x1 - word.x1).abs() < 1e-6);
        assert!(search(&pages, "  ").is_empty());
        assert!(search(&pages, "absent").is_empty());
    }
}
