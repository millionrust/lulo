//! Real PDF rendering on Windows through WinRT's `Windows.Data.Pdf` (ADR
//! 0023 phase 2c): poppler-utils has no Windows package, so
//! [`poppler::missing_tool_message`](crate::poppler::missing_tool_message)
//! used to be the end of the line there (PREV-29). Windows ships its own
//! PDF engine, which `render.rs` now calls instead on this platform,
//! feeding its decoded pages into the same [`PdfInfo`]/[`PageBox`]
//! structures and the same `RgbaImage` (BGRA) pipeline poppler's path
//! already produces.
//!
//! Every function here blocks its calling thread on a WinRT async
//! operation's result (`.get()`). `render.rs` already calls `load_pdf_info`
//! and `render_page` from a background thread (`blocking::unblock` or
//! `cx.background_executor()` in `view.rs`), never from GPUI's render
//! loop, so blocking here does not stall the UI thread.
//!
//! Text extraction (`extract_text`, for search and selection) has no
//! WinRT equivalent — `Windows.Data.Pdf` is a renderer, not a text-layer
//! reader — so it stays an honest "not available" error on this platform
//! rather than a fake empty result.

use std::path::Path;

use image::RgbaImage;
use windows::core::HSTRING;
use windows::Data::Pdf::{PdfDocument, PdfPage, PdfPageRenderOptions};
use windows::Storage::{FileAccessMode, StorageFile};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::poppler::{PageBox, PdfInfo};

/// Every public function below calls this before touching WinRT: a
/// `blocking::unblock`/`cx.background_executor()` thread pool thread may be
/// freshly spawned with no WinRT apartment initialised yet. Idempotent —
/// calling it again on an already-initialised thread just returns
/// `S_FALSE`, still a success `HRESULT` — and never undone, since these
/// are long-lived pool threads that may run further WinRT work later.
fn ensure_winrt_apartment() {
    // SAFETY: a plain WinRT bootstrap call with no pointers or borrows; it
    // is documented as safe to call more than once on the same thread.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

fn open_error(path: &Path, detail: &str) -> String {
    format!(
        "“{}” couldn’t be opened. {detail}",
        crate::document::display_name(path)
    )
}

fn windows_error(path: &Path, error: windows::core::Error) -> String {
    open_error(path, &error.message())
}

/// `StorageFile::GetFileFromPathAsync` needs a fully qualified path.
fn absolute(path: &Path) -> std::path::PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn open_document(path: &Path) -> windows::core::Result<PdfDocument> {
    ensure_winrt_apartment();
    let wide = HSTRING::from(absolute(path).as_os_str());
    let file = StorageFile::GetFileFromPathAsync(&wide)?.get()?;
    PdfDocument::LoadFromFileAsync(&file)?.get()
}

/// Page count and each page's displayed size, for Preview's page list and
/// layout — poppler's Windows-free counterpart to `render::load_pdf_info`.
pub fn load_pdf_info(path: &Path) -> Result<PdfInfo, String> {
    let document = open_document(path).map_err(|error| windows_error(path, error))?;
    let count = document
        .PageCount()
        .map_err(|error| windows_error(path, error))?;
    let mut pages = Vec::with_capacity(count as usize);
    for index in 0..count {
        let page = document
            .GetPage(index)
            .map_err(|error| windows_error(path, error))?;
        let size = page.Size().map_err(|error| windows_error(path, error))?;
        pages.push(PageBox {
            width: size.Width.max(1.0),
            height: size.Height.max(1.0),
            // `PdfPage::Size` is already the page's displayed size (it
            // takes the page's own `/Rotate` into account), unlike
            // poppler's `pdfinfo`, which reports the unrotated media box
            // alongside a separate rotation in degrees. Reporting zero
            // here keeps `PageBox::displayed` from rotating an
            // already-rotated size a second time.
            rotation: 0,
        });
    }
    if pages.is_empty() {
        return Err(open_error(path, "the PDF has no pages"));
    }
    Ok(PdfInfo {
        pages,
        ..PdfInfo::default()
    })
}

/// Rasterise one page `pixel_scale` device pixels per point (BGRA, not yet
/// rotated by the viewer's own manual rotation — `render::render_page`
/// applies that on top, exactly as it does for poppler's pages).
pub fn render_page(
    path: &Path,
    page_index: usize,
    page_size: (f32, f32),
    pixel_scale: f32,
) -> Result<RgbaImage, String> {
    let document = open_document(path).map_err(|error| windows_error(path, error))?;
    let page = document
        .GetPage(page_index as u32)
        .map_err(|error| windows_error(path, error))?;
    let dpi = crate::poppler::render_dpi(page_size, pixel_scale);
    let scale = dpi / 72.0;
    let width = ((page_size.0 * scale).round().max(1.0)) as u32;
    let height = ((page_size.1 * scale).round().max(1.0)) as u32;

    // `RenderToStreamWithOptionsAsync` only renders to a stream, and a
    // plain file read-back afterwards needs no further WinRT calls — one
    // less place for a mismatched API to fail silently.
    let temp = std::env::temp_dir().join(format!(
        "rmac-preview-render-{}-{page_index}-{width}x{height}.png",
        std::process::id()
    ));
    let rendered = render_to_file(&page, &temp, width, height);
    let bytes = rendered.and_then(|()| {
        std::fs::read(&temp).map_err(|error| open_error(path, &error.to_string()))
    });
    let _ = std::fs::remove_file(&temp);
    let bytes = bytes?;

    let decoded =
        image::load_from_memory(&bytes).map_err(|error| open_error(path, &error.to_string()))?;
    let mut pixels = decoded.to_rgba8();
    crate::render::swap_red_blue(&mut pixels);
    Ok(pixels)
}

fn render_to_file(page: &PdfPage, temp: &Path, width: u32, height: u32) -> Result<(), String> {
    let fail = |error: windows::core::Error| open_error(temp, &error.message());
    // `StorageFile::GetFileFromPathAsync` needs the file to already exist.
    std::fs::write(temp, []).map_err(|error| open_error(temp, &error.to_string()))?;
    let wide = HSTRING::from(temp.as_os_str());
    let file = StorageFile::GetFileFromPathAsync(&wide)
        .map_err(fail)?
        .get()
        .map_err(fail)?;
    let stream = file
        .OpenAsync(FileAccessMode::ReadWrite)
        .map_err(fail)?
        .get()
        .map_err(fail)?;
    let options = PdfPageRenderOptions::new().map_err(fail)?;
    options.SetDestinationWidth(width).map_err(fail)?;
    options.SetDestinationHeight(height).map_err(fail)?;
    page.RenderToStreamWithOptionsAsync(&stream, &options)
        .map_err(fail)?
        .get()
        .map_err(fail)?;
    stream.FlushAsync().map_err(fail)?.get().map_err(fail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a real PDF, so the error is the honest "couldn't be opened"
    /// kind rather than a panic — the same contract poppler's path keeps.
    #[test]
    fn load_pdf_info_reports_an_error_for_a_missing_file() {
        let error = load_pdf_info(Path::new(
            r"C:\does\not\exist\rmac-preview-winpdf-test.pdf",
        ))
        .unwrap_err();
        assert!(error.contains("couldn’t be opened"), "{error}");
    }
}
