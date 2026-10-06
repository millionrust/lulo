//! Notes' open and save choosers: the desktop portal on Lulo OS (Linux),
//! and everywhere else the platform's own dialogs through GPUI, which on
//! Windows are the native Open and Save As dialogs (ADR 0023 phase 2).
//!
//! Each function starts nothing until its future is awaited on Linux; on
//! the other platforms the dialog opens when it is called, so callers ask
//! for the future before they spawn the task that waits for it.

use std::future::Future;
use std::path::PathBuf;

use gpui::App;

/// The chooser could not be shown.
#[derive(Debug)]
pub(crate) struct Unavailable;

/// What an open chooser is for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum OpenKind {
    /// Edit ▸ Attach Photo: one image.
    Image,
    /// File ▸ Import ▸ a Markdown or text note.
    TextNote,
    /// File ▸ Import ▸ an rmac Notes bundle.
    Bundle,
}

/// What an export writes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ExportKind {
    Markdown,
    Bundle,
}

/// Choose one file to open.
pub(crate) fn open(
    kind: OpenKind,
    cx: &mut App,
) -> impl Future<Output = Result<Option<PathBuf>, Unavailable>> + 'static {
    #[cfg(target_os = "linux")]
    {
        let _ = cx;
        async move {
            match kind {
                OpenKind::Image => rmac_portal::choose_notes_image().await,
                OpenKind::TextNote => rmac_portal::choose_notes_text().await,
                OpenKind::Bundle => rmac_portal::choose_notes_bundle().await,
            }
            .map_err(|_| Unavailable)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let prompt = match kind {
            OpenKind::Image => "Attach Photo",
            OpenKind::TextNote => "Import Note",
            OpenKind::Bundle => "Import Notes Bundle",
        };
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        async move {
            match receiver.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                _ => Err(Unavailable),
            }
        }
    }
}

/// Choose any number of files to attach (Edit ▸ Attach File…).
pub(crate) fn attachments(
    cx: &mut App,
) -> impl Future<Output = Result<Vec<PathBuf>, Unavailable>> + 'static {
    #[cfg(target_os = "linux")]
    {
        let _ = cx;
        async move {
            rmac_portal::choose_mail_attachments()
                .await
                .map_err(|_| Unavailable)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        async move {
            match receiver.await {
                Ok(Ok(paths)) => Ok(paths.unwrap_or_default()),
                _ => Err(Unavailable),
            }
        }
    }
}

/// Choose where an export goes, starting from `suggested_name`.
pub(crate) fn export_destination(
    kind: ExportKind,
    suggested_name: &str,
    cx: &mut App,
) -> impl Future<Output = Result<Option<PathBuf>, Unavailable>> + 'static {
    #[cfg(target_os = "linux")]
    {
        let _ = cx;
        let format = match kind {
            ExportKind::Markdown => rmac_portal::NotesExportFormat::Markdown,
            ExportKind::Bundle => rmac_portal::NotesExportFormat::Bundle,
        };
        let suggested_name = suggested_name.to_owned();
        async move {
            rmac_portal::choose_notes_export_destination(format, &suggested_name)
                .await
                .map_err(|_| Unavailable)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = kind;
        let receiver = cx.prompt_for_new_path(&documents_directory(), Some(suggested_name));
        async move {
            match receiver.await {
                Ok(Ok(path)) => Ok(path),
                _ => Err(Unavailable),
            }
        }
    }
}

/// Where a save dialog starts: Documents, else the home folder.
#[cfg(not(target_os = "linux"))]
fn documents_directory() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let documents = home.join("Documents");
    if documents.is_dir() {
        documents
    } else {
        home
    }
}

/// Open `path` with the desktop's default app (a `📎` attachment chip).
pub(crate) fn open_with_default_app(path: PathBuf, cx: &mut App) {
    #[cfg(target_os = "linux")]
    cx.spawn(async move |_| {
        let uri = format!("file://{}", path.display());
        let _ = rmac_portal::open_uri(&uri).await;
    })
    .detach();
    #[cfg(not(target_os = "linux"))]
    cx.open_with_system(&path);
}
