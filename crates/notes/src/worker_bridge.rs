//! Bounded blocking-worker to asynchronous-UI event bridges.

use std::io;
use std::sync::Arc;
use std::thread;

use gpui::RenderImage;
use rmac_notes_runtime::{
    MarkdownPreviewWorkerEvent, NotesMarkdownPreviewWorkerEvents, NotesPreviewWorkerEvents,
    NotesSearchWorkerEvents, NotesWorkerEvents, PreviewWorkerEvent, SearchWorkerEvent, WorkerEvent,
};
use rmac_notes_storage::DecodedImagePreview;

pub(super) struct PreviewBridgeEvent {
    pub(super) event: PreviewWorkerEvent,
    pub(super) rendered: Option<Arc<RenderImage>>,
}

pub(super) fn bridge_worker_events(
    events: NotesWorkerEvents,
    capacity: usize,
) -> io::Result<async_channel::Receiver<WorkerEvent>> {
    let (sender, receiver) = async_channel::bounded(capacity);
    thread::Builder::new()
        .name("rmac-notes-ui-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.send_blocking(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(super) fn bridge_search_events(
    events: NotesSearchWorkerEvents,
    capacity: usize,
) -> io::Result<async_channel::Receiver<SearchWorkerEvent>> {
    let (sender, receiver) = async_channel::bounded(capacity);
    thread::Builder::new()
        .name("rmac-notes-ui-search-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.send_blocking(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(super) fn bridge_preview_events(
    events: NotesPreviewWorkerEvents,
    capacity: usize,
    render: fn(&DecodedImagePreview) -> Option<Arc<RenderImage>>,
) -> io::Result<async_channel::Receiver<PreviewBridgeEvent>> {
    let (sender, receiver) = async_channel::bounded(capacity);
    thread::Builder::new()
        .name("rmac-notes-ui-preview-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                let rendered = match &event {
                    PreviewWorkerEvent::Ready { image, .. } => render(image),
                    _ => None,
                };
                if sender
                    .send_blocking(PreviewBridgeEvent { event, rendered })
                    .is_err()
                {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(super) fn bridge_markdown_preview_events(
    events: NotesMarkdownPreviewWorkerEvents,
    capacity: usize,
) -> io::Result<async_channel::Receiver<MarkdownPreviewWorkerEvent>> {
    let (sender, receiver) = async_channel::bounded(capacity);
    thread::Builder::new()
        .name("rmac-notes-ui-markdown-preview-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.send_blocking(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(super) fn render_preview_image(preview: &DecodedImagePreview) -> Option<Arc<RenderImage>> {
    let expected = u64::from(preview.width())
        .checked_mul(u64::from(preview.height()))?
        .checked_mul(4)?;
    if expected != preview.rgba().len() as u64 {
        return None;
    }
    let mut bgra = Vec::with_capacity(preview.rgba().len());
    let mut pixels = preview.rgba().chunks_exact(4);
    for pixel in &mut pixels {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    if !pixels.remainder().is_empty() {
        return None;
    }
    let buffer = image::RgbaImage::from_raw(preview.width(), preview.height(), bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}
