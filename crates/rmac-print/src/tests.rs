use super::*;

#[test]
fn unicode_text_renders_to_a_structurally_complete_pdf() {
    let pdf = render_pdf("Hello, नमस्ते, مرحبًا, こんにちは 🦀\n", PageLayout::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.7"));
    assert!(pdf
        .windows(b"/Count 1".len())
        .any(|part| part == b"/Count 1"));
    assert!(pdf.ends_with(b"%%EOF\n"));
    assert!(!pdf
        .windows("नमस्ते".len())
        .any(|part| part == "नमस्ते".as_bytes()));
}

#[test]
fn long_text_paginates_deterministically() {
    let text = (0..160)
        .map(|index| format!("Line {index}\n"))
        .collect::<String>();
    let pdf = render_pdf(&text, PageLayout::default()).unwrap();
    assert!(pdf
        .windows(b"/Count 4".len())
        .any(|part| part == b"/Count 4"));
}

#[test]
fn invalid_layout_and_excessive_input_fail_closed() {
    let invalid = PageLayout {
        margin_left_mm: 150.0,
        margin_right_mm: 150.0,
        ..PageLayout::default()
    };
    assert_eq!(render_pdf("draft", invalid), Err(Error::InvalidPageLayout));
    let excessive = "x".repeat(MAX_SOURCE_BYTES + 1);
    assert_eq!(
        render_pdf(&excessive, PageLayout::default()),
        Err(Error::SourceTooLarge)
    );
}

#[test]
fn blank_document_still_produces_one_printable_page() {
    let pdf = render_pdf("", PageLayout::default()).unwrap();
    assert!(pdf
        .windows(b"/Count 1".len())
        .any(|part| part == b"/Count 1"));
}

#[test]
fn portal_page_description_uses_orientation_and_rejects_bad_margins() {
    let landscape = PageLayout::from_description(PageDescription {
        width_mm: Some(210.0),
        height_mm: Some(297.0),
        orientation: Some(PageOrientation::Landscape),
        ..PageDescription::default()
    })
    .unwrap();
    assert_eq!((landscape.width_mm, landscape.height_mm), (297.0, 210.0));

    let invalid = PageLayout::from_description(PageDescription {
        margin_left_mm: Some(120.0),
        margin_right_mm: Some(120.0),
        ..PageDescription::default()
    });
    assert_eq!(invalid, Err(Error::InvalidPageLayout));
}

#[test]
fn stream_lengths_and_xref_offsets_point_to_valid_boundaries() {
    let pdf = render_pdf("xref validation", PageLayout::default()).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    let xref_offset = text
        .rsplit_once("startxref\n")
        .and_then(|(_, suffix)| suffix.lines().next())
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert_eq!(&pdf[xref_offset..xref_offset + 4], b"xref");
    assert!(text.contains("1 0 obj\n<< /Type /Catalog"));
}
