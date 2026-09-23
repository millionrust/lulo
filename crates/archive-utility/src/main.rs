//! rmac Archive Utility: what other applications' "Open" reaches for zip and
//! tar archives. Like macOS it has no main window: it expands each archive
//! next to itself (crates/rmac-archive), shows a progress window only when a
//! job runs longer than a second, shows the Mac's alert for an archive it
//! cannot expand, and quits when there is nothing left on screen.

mod view;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, App, AppContext as _, AssetSource, Entity, Result, SharedString, WindowBounds,
    WindowHandle,
};
use gpui_component::Root;

use crate::view::{AlertView, ProgressView, ALERT_SIZE, PROGRESS_SIZE};

const APP_ID: &str = "org.rmac.ArchiveUtility";
/// Quick jobs finish without any window, as measured on the Mac (S: delay).
const PROGRESS_DELAY: Duration = Duration::from_secs(1);

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct ArchiveUtilityAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = ArchiveUtilityAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = ArchiveUtilityAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

enum Update {
    Started(usize),
    Progress(rmac_archive::Progress),
}

type ProgressWindow = (WindowHandle<Root>, Entity<ProgressView>);

fn main() {
    let archives: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|argument| !argument.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
        .collect();
    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            let running = Rc::new(Cell::new(true));
            cx.on_window_closed({
                let running = running.clone();
                move |cx, _| {
                    if !running.get() && cx.windows().is_empty() {
                        cx.quit();
                    }
                }
            })
            .detach();
            expand_all(archives, running, cx);
        });
}

fn label_for(archive: &std::path::Path) -> SharedString {
    let name = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("Expanding “{name}”").into()
}

fn expand_all(archives: Vec<PathBuf>, running: Rc<Cell<bool>>, cx: &mut App) {
    if archives.is_empty() {
        cx.quit();
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (sender, receiver) = async_channel::unbounded::<Update>();
    let job = {
        let archives = archives.clone();
        let cancel = cancel.clone();
        cx.background_executor().spawn(async move {
            let mut failures = Vec::new();
            for (index, archive) in archives.iter().enumerate() {
                let _ = sender.try_send(Update::Started(index));
                let mut report = |progress: rmac_archive::Progress| {
                    let _ = sender.try_send(Update::Progress(progress));
                };
                match rmac_archive::expand(archive, &cancel, &mut report) {
                    Ok(_) => {}
                    Err(rmac_archive::Error::Cancelled) => break,
                    Err(error) => {
                        failures.extend(rmac_archive::expand_error_message(archive, &error))
                    }
                }
            }
            failures
        })
    };

    let window: Rc<RefCell<Option<ProgressWindow>>> = Rc::new(RefCell::new(None));
    let finished = Rc::new(Cell::new(false));
    let current = Rc::new(Cell::new(0usize));

    // Show the progress window only once the job has run for a while.
    cx.spawn({
        let window = window.clone();
        let finished = finished.clone();
        let current = current.clone();
        let archives = archives.clone();
        let cancel = cancel.clone();
        async move |cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(PROGRESS_DELAY).await;
            cx.update(|cx| {
                if finished.get() {
                    return;
                }
                let label = archives
                    .get(current.get())
                    .map(|archive| label_for(archive))
                    .unwrap_or_default();
                *window.borrow_mut() = open_progress(label, cancel, started, cx);
            });
        }
    })
    .detach();

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        // The channel closes when the job finishes and drops its sender.
        while let Ok(update) = receiver.recv().await {
            cx.update(|cx| {
                let shown = window.borrow().as_ref().map(|(_, view)| view.clone());
                match update {
                    Update::Started(index) => {
                        current.set(index);
                        if let (Some(view), Some(archive)) = (shown, archives.get(index)) {
                            view.update(cx, |view, cx| {
                                view.label = label_for(archive);
                                view.progress = rmac_archive::Progress::default();
                                cx.notify();
                            });
                        }
                    }
                    Update::Progress(progress) => {
                        if let Some(view) = shown {
                            view.update(cx, |view, cx| {
                                view.progress = progress;
                                cx.notify();
                            });
                        }
                    }
                }
            });
        }
        let failures = job.await;
        cx.update(|cx| {
            finished.set(true);
            running.set(false);
            if let Some((handle, _)) = window.borrow_mut().take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            if failures.is_empty() {
                cx.quit();
                return;
            }
            for message in failures {
                open_alert(message.into(), cx);
            }
        });
    })
    .detach();
}

fn fixed_window_options(
    title: &'static str,
    (width, height): (f32, f32),
    cx: &App,
) -> gpui::WindowOptions {
    let mut options = rmac_ui::window_options_for_app_with_title(APP_ID, title, width, height, cx);
    let fixed = size(px(width), px(height));
    options.window_bounds = Some(WindowBounds::centered(fixed, cx));
    options.window_min_size = Some(fixed);
    options.is_resizable = false;
    options.is_minimizable = false;
    options
}

fn open_progress(
    label: SharedString,
    cancel: Arc<AtomicBool>,
    started: Instant,
    cx: &mut App,
) -> Option<ProgressWindow> {
    let options = fixed_window_options("Archive Utility", PROGRESS_SIZE, cx);
    let mut created = None;
    let handle = cx
        .open_window(options, |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|_| ProgressView::new(label, cancel, started));
            created = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .ok()?;
    created.map(|view| (handle, view))
}

fn open_alert(message: SharedString, cx: &mut App) {
    let options = fixed_window_options("Archive Utility", ALERT_SIZE, cx);
    let text = message.clone();
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| AlertView::new(text, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    });
    if opened.is_err() {
        eprintln!("rmac-archive-utility: {message}");
    }
    cx.activate(true);
}
