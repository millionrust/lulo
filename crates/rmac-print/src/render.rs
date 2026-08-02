use std::io::Write as _;

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::model::{
    Error, PageLayout, ValidLayout, MAX_PDF_BYTES, MAX_RASTER_BYTES, MILLIMETERS_PER_INCH,
    POINTS_PER_INCH,
};
use crate::{MAX_PAGES, MAX_SOURCE_BYTES};

/// Render plain text into a bounded, self-contained PDF.
///
/// Font discovery, shaping, rasterization, and compression are synchronous and
/// should run on a background executor. No source text or path is included in
/// PDF metadata.
pub fn render_pdf(text: &str, layout: PageLayout) -> Result<Vec<u8>, Error> {
    if text.len() > MAX_SOURCE_BYTES {
        return Err(Error::SourceTooLarge);
    }
    let layout = validate_layout(layout)?;
    let mut font_system = FontSystem::new();
    let metrics = Metrics::new(layout.font_size_px, layout.line_height_px as f32);
    let mut buffer = Buffer::new(&mut font_system, metrics);
    buffer.set_size(&mut font_system, Some(layout.content_width_px as f32), None);
    buffer.set_wrap(&mut font_system, Wrap::WordOrGlyph);
    buffer.set_text(
        &mut font_system,
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_system, false);

    let content_height = layout.content_height_px as f32;
    let total_height = buffer
        .layout_runs()
        .map(|run| run.line_top + run.line_height)
        .fold(layout.line_height_px as f32, f32::max);
    let page_count = ((total_height / content_height).ceil() as usize).max(1);
    if page_count > MAX_PAGES {
        return Err(Error::TooManyPages);
    }
    let row_bytes = layout.width_px.div_ceil(8);
    let bytes_per_page = row_bytes
        .checked_mul(layout.height_px)
        .ok_or(Error::RasterLimit)?;
    let raster_bytes = bytes_per_page
        .checked_mul(page_count)
        .ok_or(Error::RasterLimit)?;
    if raster_bytes > MAX_RASTER_BYTES {
        return Err(Error::RasterLimit);
    }
    let mut pages = vec![vec![u8::MAX; bytes_per_page]; page_count];
    let mut cache = SwashCache::new();
    let mut rendered_pixel = false;
    buffer.draw(
        &mut font_system,
        &mut cache,
        Color::rgb(0, 0, 0),
        |x, y, pixel_width, pixel_height, color| {
            if color.a() < 32 {
                return;
            }
            for row in 0..pixel_height {
                for column in 0..pixel_width {
                    let Some(x) = x.checked_add_unsigned(column) else {
                        continue;
                    };
                    let Some(y) = y.checked_add_unsigned(row) else {
                        continue;
                    };
                    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
                        continue;
                    };
                    let page = y / layout.content_height_px;
                    let Some(page_bytes) = pages.get_mut(page) else {
                        continue;
                    };
                    let local_x = match layout.margin_left_px.checked_add(x) {
                        Some(x) if x < layout.width_px => x,
                        _ => continue,
                    };
                    let local_y = match layout
                        .margin_top_px
                        .checked_add(y % layout.content_height_px)
                    {
                        Some(y) if y < layout.height_px => y,
                        _ => continue,
                    };
                    let byte = local_y
                        .checked_mul(row_bytes)
                        .and_then(|row| row.checked_add(local_x / 8));
                    if let Some(byte) = byte.and_then(|index| page_bytes.get_mut(index)) {
                        *byte &= !(1 << (7 - local_x % 8));
                        rendered_pixel = true;
                    }
                }
            }
        },
    );
    if !rendered_pixel && text.chars().any(|character| !character.is_whitespace()) {
        return Err(Error::FontUnavailable);
    }
    encode_pdf(&pages, &layout)
}

pub(crate) fn validate_layout(layout: PageLayout) -> Result<ValidLayout, Error> {
    let values = [
        layout.width_mm,
        layout.height_mm,
        layout.margin_top_mm,
        layout.margin_right_mm,
        layout.margin_bottom_mm,
        layout.margin_left_mm,
    ];
    if values.iter().any(|value| !value.is_finite())
        || !(50.0..=1000.0).contains(&layout.width_mm)
        || !(50.0..=1000.0).contains(&layout.height_mm)
        || values[2..].iter().any(|margin| *margin < 0.0)
        || !(72..=300).contains(&layout.dpi)
    {
        return Err(Error::InvalidPageLayout);
    }
    let content_width_mm = layout.width_mm - layout.margin_left_mm - layout.margin_right_mm;
    let content_height_mm = layout.height_mm - layout.margin_top_mm - layout.margin_bottom_mm;
    if content_width_mm < 20.0 || content_height_mm < 20.0 {
        return Err(Error::InvalidPageLayout);
    }
    let pixels_per_mm = f64::from(layout.dpi) / MILLIMETERS_PER_INCH;
    let to_pixels = |millimeters: f64| -> Result<usize, Error> {
        let pixels = (millimeters * pixels_per_mm).round();
        if pixels < 0.0 || pixels > u32::MAX.into() {
            Err(Error::InvalidPageLayout)
        } else {
            Ok(pixels as usize)
        }
    };
    let width_px = to_pixels(layout.width_mm)?;
    let height_px = to_pixels(layout.height_mm)?;
    let margin_top_px = to_pixels(layout.margin_top_mm)?;
    let margin_left_px = to_pixels(layout.margin_left_mm)?;
    let content_width_px = to_pixels(content_width_mm)?.max(1);
    let raw_content_height_px = to_pixels(content_height_mm)?.max(1);
    let font_size_px = 11.0 * f32::from(layout.dpi) / POINTS_PER_INCH as f32;
    let line_height_px = (15.0 * f64::from(layout.dpi) / POINTS_PER_INCH)
        .round()
        .max(1.0) as usize;
    let content_height_px = (raw_content_height_px / line_height_px)
        .max(1)
        .checked_mul(line_height_px)
        .ok_or(Error::InvalidPageLayout)?;
    Ok(ValidLayout {
        width_px,
        height_px,
        margin_top_px,
        margin_left_px,
        content_width_px,
        content_height_px,
        line_height_px,
        font_size_px,
        width_points: layout.width_mm * POINTS_PER_INCH / MILLIMETERS_PER_INCH,
        height_points: layout.height_mm * POINTS_PER_INCH / MILLIMETERS_PER_INCH,
    })
}

fn encode_pdf(pages: &[Vec<u8>], layout: &ValidLayout) -> Result<Vec<u8>, Error> {
    let page_count = pages.len();
    let object_count = 2_usize
        .checked_add(page_count.checked_mul(3).ok_or(Error::Encode)?)
        .ok_or(Error::Encode)?;
    let page_ids = (0..page_count)
        .map(|index| 3 + index * 3)
        .collect::<Vec<_>>();
    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = Vec::with_capacity(object_count);
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(format!("<< /Type /Pages /Count {} /Kids [{}] >>", page_count, kids).into_bytes());
    for (index, page) in pages.iter().enumerate() {
        let page_id = page_ids[index];
        let image_id = page_id + 1;
        let content_id = page_id + 2;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.3} {:.3}] /Resources << /XObject << /Im{index} {image_id} 0 R >> >> /Contents {content_id} 0 R >>",
                layout.width_points, layout.height_points
            )
            .into_bytes(),
        );
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(page).map_err(|_| Error::Encode)?;
        let compressed = encoder.finish().map_err(|_| Error::Encode)?;
        let mut image = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /FlateDecode /Length {} >>\nstream\n",
            layout.width_px,
            layout.height_px,
            compressed.len()
        )
        .into_bytes();
        image.extend_from_slice(&compressed);
        image.extend_from_slice(b"\nendstream");
        objects.push(image);
        let command = format!(
            "q\n{:.3} 0 0 {:.3} 0 0 cm\n/Im{index} Do\nQ\n",
            layout.width_points, layout.height_points
        );
        objects.push(stream_object(command.as_bytes()));
    }
    debug_assert_eq!(objects.len(), object_count);

    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(object_count);
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(&mut pdf, "{} 0 obj", index + 1).map_err(|_| Error::Encode)?;
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
        if pdf.len() > MAX_PDF_BYTES {
            return Err(Error::Encode);
        }
    }
    let xref = pdf.len();
    write!(&mut pdf, "xref\n0 {}\n", object_count + 1).map_err(|_| Error::Encode)?;
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        writeln!(&mut pdf, "{offset:010} 00000 n ").map_err(|_| Error::Encode)?;
    }
    write!(
        &mut pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        object_count + 1,
        xref
    )
    .map_err(|_| Error::Encode)?;
    (pdf.len() <= MAX_PDF_BYTES)
        .then_some(pdf)
        .ok_or(Error::Encode)
}

fn stream_object(contents: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", contents.len()).into_bytes();
    stream.extend_from_slice(contents);
    stream.extend_from_slice(b"endstream");
    stream
}
