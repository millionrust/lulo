//! Document identity, navigation and the text Preview shows about documents:
//! the toolbar subtitle and the Get Info values.

use std::cmp::Ordering;
use std::path::Path;

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};

/// What a file holds, recognised from its first bytes (never the name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Image(ImageKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Tiff,
}

impl ImageKind {
    /// Get Info "Document Type".
    pub fn document_type(self) -> &'static str {
        match self {
            ImageKind::Png => "PNG image",
            ImageKind::Jpeg => "JPEG image",
            ImageKind::Gif => "GIF image",
            ImageKind::Webp => "WebP image",
            ImageKind::Bmp => "BMP image",
            ImageKind::Tiff => "TIFF image",
        }
    }
}

impl Kind {
    pub fn document_type(self) -> &'static str {
        match self {
            Kind::Pdf => "PDF document",
            Kind::Image(kind) => kind.document_type(),
        }
    }

    pub fn is_pdf(self) -> bool {
        matches!(self, Kind::Pdf)
    }
}

/// Recognise a supported document from its leading bytes.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    let image = match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageKind::Png),
        [0xFF, 0xD8, 0xFF, ..] => Some(ImageKind::Jpeg),
        [b'G', b'I', b'F', b'8', ..] => Some(ImageKind::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageKind::Webp),
        [b'B', b'M', ..] => Some(ImageKind::Bmp),
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => Some(ImageKind::Tiff),
        _ => None,
    };
    if let Some(kind) = image {
        return Some(Kind::Image(kind));
    }
    // PDF readers accept the header anywhere in the first 1 KiB.
    let head = &bytes[..bytes.len().min(1024)];
    head.windows(5)
        .any(|window| window == b"%PDF-")
        .then_some(Kind::Pdf)
}

/// Extensions of the images Preview steps through in a folder. The file is
/// still sniffed before it is shown.
pub fn has_image_extension(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    )
}

/// Finder's name order: case-insensitive, with runs of digits compared by
/// value so "img2" sorts before "img10".
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut a = left.chars().peekable();
    let mut b = right.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return left.cmp(right),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (x, y) = (take(&mut a), take(&mut b));
                let (xt, yt) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let order = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Sort folder entries in Finder order and keep the images.
pub fn folder_images(mut names: Vec<String>) -> Vec<String> {
    names.retain(|name| !name.starts_with('.') && has_image_extension(name));
    names.sort_by(|a, b| natural_cmp(a, b));
    names
}

/// Move `delta` items from `current` among `len`, stopping at the ends like
/// Preview's Go ▸ Next / Previous Item.
pub fn step(current: usize, len: usize, delta: isize) -> Option<usize> {
    let target = current as isize + delta;
    (target >= 0 && (target as usize) < len && target as usize != current)
        .then_some(target as usize)
}

/// The toolbar's second title line.
pub fn subtitle(
    documents: usize,
    total_pages: usize,
    pdf_pages: Option<(usize, usize)>,
) -> Option<String> {
    if documents > 1 {
        return Some(format!(
            "{} documents, {} total {}",
            documents,
            total_pages,
            if total_pages == 1 { "page" } else { "pages" }
        ));
    }
    match pdf_pages {
        Some((_, 1)) => Some("1 page".into()),
        Some((current, count)) => Some(format!("Page {} of {}", current + 1, count)),
        None => None,
    }
}

/// Get Info file size: "4 KB (3,916 bytes)", in decimal units like Finder.
pub fn format_file_size(bytes: u64) -> String {
    let exact = format!("{} bytes", group_thousands(bytes));
    if bytes < 1_000 {
        return exact;
    }
    let value = bytes as f64;
    let short = if bytes < 1_000_000 {
        format!("{} KB", (value / 1e3).round())
    } else if bytes < 1_000_000_000 {
        format!("{:.1} MB", value / 1e6)
    } else {
        format!("{:.2} GB", value / 1e9)
    };
    format!("{short} ({exact})")
}

pub fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Get Info date: "23 Sep 2026 at 1:28 PM".
pub fn format_date(value: NaiveDateTime) -> String {
    value.format("%-d %b %Y at %-I:%M %p").to_string()
}

pub fn format_local(value: DateTime<Local>) -> String {
    format_date(value.naive_local())
}

pub fn format_system_time(value: std::time::SystemTime) -> String {
    format_local(DateTime::<Local>::from(value))
}

/// A poppler `-isodates` date shown in local time.
pub fn format_iso_date(value: &str) -> Option<String> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(format_local(date.with_timezone(&Local)));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S").ok()?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(format_local)
}

/// Get Info page size in centimetres: "21.59 × 27.94 cm".
pub fn format_page_size_cm(width_points: f32, height_points: f32) -> String {
    let cm = |points: f32| points / 72.0 * 2.54;
    format!("{:.2} × {:.2} cm", cm(width_points), cm(height_points))
}

/// Get Info image size: "600 × 800 pixels".
pub fn format_pixels(width: u32, height: u32) -> String {
    format!("{width} × {height} pixels")
}

/// The file name shown as the title.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_documents_by_content() {
        assert_eq!(sniff(b"%PDF-1.4\n..."), Some(Kind::Pdf));
        assert_eq!(sniff(b"\r\n%PDF-1.7"), Some(Kind::Pdf));
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A]),
            Some(Kind::Image(ImageKind::Png))
        );
        assert_eq!(
            sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(Kind::Image(ImageKind::Jpeg))
        );
        assert_eq!(sniff(b"GIF89a"), Some(Kind::Image(ImageKind::Gif)));
        assert_eq!(
            sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(Kind::Image(ImageKind::Webp))
        );
        assert_eq!(sniff(b"BM\0\0"), Some(Kind::Image(ImageKind::Bmp)));
        assert_eq!(sniff(b"II*\0"), Some(Kind::Image(ImageKind::Tiff)));
        assert_eq!(sniff(b"MM\0*"), Some(Kind::Image(ImageKind::Tiff)));
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(Kind::Pdf.document_type(), "PDF document");
        assert_eq!(Kind::Image(ImageKind::Png).document_type(), "PNG image");
    }

    #[test]
    fn folder_images_sort_like_finder() {
        let names = [
            "img10.png",
            "img2.PNG",
            "notes.txt",
            ".hidden.png",
            "Img1.jpg",
            "photo.JPEG",
            "a.webp",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            folder_images(names),
            ["a.webp", "Img1.jpg", "img2.PNG", "img10.png", "photo.JPEG"]
        );
        assert!(!has_image_extension("README"));
    }

    #[test]
    fn natural_order_compares_digit_runs_by_value() {
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("a010", "a9"), Ordering::Greater);
        assert_eq!(natural_cmp("B", "a"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
        assert_ne!(natural_cmp("a1", "A1"), Ordering::Equal);
    }

    #[test]
    fn stepping_stops_at_the_ends() {
        assert_eq!(step(0, 3, 1), Some(1));
        assert_eq!(step(2, 3, 1), None);
        assert_eq!(step(0, 3, -1), None);
        assert_eq!(step(1, 3, -1), Some(0));
        assert_eq!(step(0, 0, 1), None);
    }

    #[test]
    fn subtitles_match_preview() {
        assert_eq!(subtitle(1, 1, None), None);
        assert_eq!(subtitle(1, 1, Some((0, 1))).as_deref(), Some("1 page"));
        assert_eq!(subtitle(1, 3, Some((0, 3))).as_deref(), Some("Page 1 of 3"));
        assert_eq!(
            subtitle(3, 3, None).as_deref(),
            Some("3 documents, 3 total pages")
        );
        assert_eq!(
            subtitle(2, 7, Some((4, 5))).as_deref(),
            Some("2 documents, 7 total pages")
        );
    }

    #[test]
    fn file_sizes_match_finder() {
        assert_eq!(format_file_size(3_916), "4 KB (3,916 bytes)");
        assert_eq!(format_file_size(999), "999 bytes");
        assert_eq!(format_file_size(1_234_567), "1.2 MB (1,234,567 bytes)");
        assert_eq!(
            format_file_size(12_345_678_901),
            "12.35 GB (12,345,678,901 bytes)"
        );
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(100_000), "100,000");
    }

    #[test]
    fn dates_and_sizes_format_like_get_info() {
        let date =
            NaiveDateTime::parse_from_str("2026-09-23 13:28:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(format_date(date), "23 Sep 2026 at 1:28 PM");
        assert_eq!(format_page_size_cm(612.0, 792.0), "21.59 × 27.94 cm");
        assert_eq!(format_pixels(600, 800), "600 × 800 pixels");
        assert!(format_iso_date("2026-09-23T13:00:00+05:30").is_some());
        assert!(format_iso_date("2026-09-23T13:00:00").is_some());
        assert!(format_iso_date("yesterday").is_none());
        assert_eq!(display_name(Path::new("/tmp/a b.pdf")), "a b.pdf");
    }
}
