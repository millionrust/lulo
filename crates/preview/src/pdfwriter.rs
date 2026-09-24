//! A minimal, hand-written single-page PDF wrapping one JPEG image — the
//! shared building block behind printing an image and exporting one as PDF.
//! rmac has no PDF-writing library (ADR: `rmac_print` only rasterises text),
//! and a JPEG-in-PDF page is a small, standards-compliant document to build
//! by hand: one `/XObject /Image` with `/Filter /DCTDecode` holding the JPEG
//! bytes unchanged, and one content stream that paints it to fill the page.
//! Pixels are treated as points 1:1 (72 dpi), which is what Preview shows on
//! screen; Preview does not read a source image's own DPI metadata.

/// Build a one-page PDF that shows `jpeg` (`width`×`height` px) filling the
/// page.
pub fn wrap_jpeg(jpeg: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let mut pdf = Vec::with_capacity(jpeg.len() + 1024);
    let mut offsets = [0usize; 6];

    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    offsets[1] = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets[2] = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    offsets[3] = pdf.len();
    pdf.extend_from_slice(
        format!(
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>\nendobj\n"
        )
        .as_bytes(),
    );

    offsets[4] = pdf.len();
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            width as u32,
            height as u32,
            jpeg.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(jpeg);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    offsets[5] = pdf.len();
    let content = format!("q {width:.3} 0 0 {height:.3} 0 0 cm /Im0 Do Q");
    pdf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
            content.len()
        )
        .as_bytes(),
    );

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF").as_bytes(),
    );
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The byte offset of `needle`'s first match in `haystack`. Byte-exact
    /// (unlike a lossy UTF-8 `str` view, which the PDF's own binary marker
    /// comment — deliberately invalid UTF-8 — would shift out of alignment).
    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    #[test]
    fn wraps_a_jpeg_in_a_well_formed_single_page_pdf() {
        let jpeg = b"\xFF\xD8\xFF\xD9-fake-jpeg-bytes";
        let pdf = wrap_jpeg(jpeg, 600, 400);

        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF"));
        assert!(find(&pdf, b"/MediaBox [0 0 600 400]").is_some());
        assert!(find(&pdf, b"/Filter /DCTDecode").is_some());
        assert_eq!(
            pdf.windows(jpeg.len())
                .filter(|window| *window == &jpeg[..])
                .count(),
            1,
            "the JPEG bytes are embedded exactly once, unmodified"
        );

        // Every "N 0 obj" the xref table points at actually starts there.
        let marker = b"startxref\n";
        let xref_at = find(&pdf, marker).unwrap() + marker.len();
        let digits_end = xref_at + pdf[xref_at..].iter().position(|b| *b == b'\n').unwrap();
        let xref_offset: usize = std::str::from_utf8(&pdf[xref_at..digits_end])
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(&pdf[xref_offset..xref_offset + 4], b"xref");

        // Each of the 6 fixed-width (20-byte) entries follows "xref\n0 6\n".
        let header = b"xref\n0 6\n";
        let entries_at = find(&pdf, header).unwrap() + header.len();
        for object in 1..=5usize {
            let line = &pdf[entries_at + object * 20..entries_at + object * 20 + 20];
            assert!(
                line.ends_with(b" n \n"),
                "entry {object} is not in use: {line:?}"
            );
            let offset: usize = std::str::from_utf8(&line[..10]).unwrap().parse().unwrap();
            let object_marker = format!("{object} 0 obj").into_bytes();
            assert!(
                pdf[offset..].starts_with(&object_marker),
                "object {object} offset does not point at `{object} 0 obj`"
            );
        }
    }

    #[test]
    fn degenerate_sizes_stay_at_least_one_point() {
        let pdf = wrap_jpeg(b"\xFF\xD8\xFF\xD9", 0, 0);
        assert!(find(&pdf, b"/MediaBox [0 0 1 1]").is_some());
    }
}
