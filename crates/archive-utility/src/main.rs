//! rmac Archive Utility: what other applications' "Open" reaches for zip and
//! tar archives. Like macOS it has no main window: it expands each archive
//! next to itself (crates/rmac-archive), shows a progress window only when a
//! job runs longer than a second, shows the Mac's alert for an archive it
//! cannot expand, and quits when there is nothing left on screen. Its menus
//! are the Mac's: Settings… (⌘,), File ▸ Create Archive… (⌘K) and Expand
//! Archive… (⌘U).

// A GUI app on Windows: no console behind it (ADR 0023). Without this
// Windows made a console host for the process before `main` ran, about
// 200 ms of every launch on the reference laptop (WIN-OS-99).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod settings;
mod settings_window;
mod view;

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, App, AppContext as _, AssetSource, Entity, Global, KeyBinding, PathPromptOptions,
    Result, SharedString, WindowBounds, WindowHandle,
};
use gpui_component::Root;

use crate::view::{AlertView, ProgressView, ALERT_SIZE, PROGRESS_SIZE};

pub const APP_ID: &str = rmac_ui::app_id::ARCHIVE_UTILITY;
/// Quick jobs finish without any window, as measured on the Mac (S: delay).
const PROGRESS_DELAY: Duration = Duration::from_secs(1);

gpui::actions!(
    archive_utility,
    [CreateArchive, ExpandArchive, ShowSettings, CloseWindow]
);

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

/// The saved Settings, read once at launch and changed by the Settings
/// window; each job reads it when it starts.
pub struct ArchiveSettings(pub settings::Settings);

impl Global for ArchiveSettings {}

/// Jobs and Open panels still in flight. Archive Utility has no main
/// window, so it quits once nothing is running and no window is left.
#[derive(Default)]
struct Activity {
    busy: usize,
}

impl Global for Activity {}

fn begin(cx: &mut App) {
    cx.default_global::<Activity>().busy += 1;
}

fn end(cx: &mut App) {
    let activity = cx.default_global::<Activity>();
    activity.busy = activity.busy.saturating_sub(1);
    quit_if_idle(cx);
}

fn quit_if_idle(cx: &mut App) {
    if cx.default_global::<Activity>().busy == 0 && cx.windows().is_empty() {
        cx.quit();
    }
}

/// One run of the job: expand each archive, or put every item in one zip.
#[derive(Clone)]
enum Job {
    Expand(Vec<PathBuf>),
    Create(Vec<PathBuf>),
}

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
            cx.set_global(ArchiveSettings(settings::load()));
            cx.set_global(Activity::default());
            // App-wide, as on the Mac: ⌘K and ⌘U work from any of its
            // windows, and the menu bar's rows reach these handlers even
            // when the key window (an alert, Settings) has none of its own.
            cx.bind_keys([
                KeyBinding::new("cmd-k", CreateArchive, None),
                KeyBinding::new("cmd-u", ExpandArchive, None),
                KeyBinding::new("cmd-,", ShowSettings, None),
                KeyBinding::new("cmd-w", CloseWindow, None),
            ]);
            cx.on_action(|_: &CreateArchive, cx| choose_and_run(false, cx));
            cx.on_action(|_: &ExpandArchive, cx| choose_and_run(true, cx));
            cx.on_action(|_: &ShowSettings, cx| settings_window::show(cx));
            rmac_ui::install_app_menu(APP_ID, cx);
            cx.on_window_closed(|cx, _| quit_if_idle(cx)).detach();
            if archives.is_empty() {
                // Launched without an archive: the Mac shows only its menu
                // bar, but Lulo's menu bar follows a window, so offer File ▸
                // Expand Archive…'s panel straight away.
                choose_and_run(true, cx);
            } else {
                run(Job::Expand(archives.clone()), cx);
            }
        });
}

/// File ▸ Expand Archive… (⌘U) or Create Archive… (⌘K): the Open panel,
/// then the job on whatever was chosen.
fn choose_and_run(expand: bool, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some(if expand { "Expand" } else { "Archive" }.into()),
    });
    begin(cx);
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let chosen = match receiver.await {
            Ok(Ok(Some(paths))) => paths,
            Ok(Err(error)) => {
                eprintln!("rmac-archive-utility: could not show the Open panel: {error}");
                Vec::new()
            }
            _ => Vec::new(),
        };
        cx.update(|cx| {
            if !chosen.is_empty() {
                run(
                    if expand {
                        Job::Expand(chosen)
                    } else {
                        Job::Create(chosen)
                    },
                    cx,
                );
            }
            end(cx);
        });
    })
    .detach();
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn job_label(job: &Job, index: usize) -> SharedString {
    match job {
        Job::Expand(archives) => archives
            .get(index)
            .map(|archive| format!("Expanding “{}”", display_name(archive)))
            .unwrap_or_default()
            .into(),
        Job::Create(items) => match items.as_slice() {
            [one] => format!("Archiving “{}”", display_name(one)).into(),
            _ => format!("Archiving {} items", items.len()).into(),
        },
    }
}

/// The background half of [`run`]: the archive work, then Settings' After
/// expanding/After archiving choice. Returns the alert messages and the
/// results to reveal.
fn work(
    job: &Job,
    settings: &settings::Settings,
    cancel: &AtomicBool,
    sender: &async_channel::Sender<Update>,
) -> (Vec<String>, Vec<PathBuf>) {
    let mut failures = Vec::new();
    let mut revealed = Vec::new();
    let mut report = |progress: rmac_archive::Progress| {
        let _ = sender.try_send(Update::Progress(progress));
    };
    match job {
        Job::Expand(archives) => {
            for (index, archive) in archives.iter().enumerate() {
                let _ = sender.try_send(Update::Started(index));
                match settings::expand(archive, settings, cancel, &mut report) {
                    Ok(placed) => {
                        if let Err(error) = settings.after_expanding.apply(archive) {
                            eprintln!("rmac-archive-utility: {error}");
                        }
                        if settings.reveal_expanded {
                            revealed.push(placed);
                        }
                    }
                    Err(rmac_archive::Error::Cancelled) => break,
                    Err(error) => {
                        failures.extend(rmac_archive::expand_error_message(archive, &error))
                    }
                }
            }
        }
        Job::Create(items) => {
            let _ = sender.try_send(Update::Started(0));
            let Some(first) = items.first() else {
                return (failures, revealed);
            };
            let destination = settings.archive_destination(first);
            match rmac_archive::compress_into(items, &destination, cancel, &mut report) {
                Ok(archive) => {
                    for item in items {
                        if let Err(error) = settings.after_archiving.apply(item) {
                            eprintln!("rmac-archive-utility: {error}");
                        }
                    }
                    if settings.reveal_archive {
                        revealed.push(archive);
                    }
                }
                Err(error) => failures.extend(rmac_archive::compress_error_message(items, &error)),
            }
        }
    }
    (failures, revealed)
}

/// Runs `job` on the background executor, showing the progress window only
/// once it has taken a while, then reveals the results Settings asks for
/// and shows the Mac's alert for each failure.
fn run(job: Job, cx: &mut App) {
    let settings = cx.global::<ArchiveSettings>().0.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let (sender, receiver) = async_channel::unbounded::<Update>();
    begin(cx);
    let task = {
        let job = job.clone();
        let cancel = cancel.clone();
        cx.background_executor()
            .spawn(async move { work(&job, &settings, &cancel, &sender) })
    };

    let window: Rc<RefCell<Option<ProgressWindow>>> = Rc::new(RefCell::new(None));
    let finished = Rc::new(Cell::new(false));
    let current = Rc::new(Cell::new(0usize));

    // Show the progress window only once the job has run for a while.
    cx.spawn({
        let window = window.clone();
        let finished = finished.clone();
        let current = current.clone();
        let job = job.clone();
        let cancel = cancel.clone();
        async move |cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(PROGRESS_DELAY).await;
            cx.update(|cx| {
                if finished.get() {
                    return;
                }
                let label = job_label(&job, current.get());
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
                        if let Some(view) = shown {
                            let label = job_label(&job, index);
                            view.update(cx, |view, cx| {
                                view.label = label;
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
        let (failures, revealed) = task.await;
        // Reveal expanded item(s)/archive in Files.
        for item in revealed {
            if let Err(error) = rmac_portal::show_item(&item).await {
                eprintln!("rmac-archive-utility: could not reveal the result: {error}");
            }
        }
        cx.update(|cx| {
            finished.set(true);
            if let Some((handle, _)) = window.borrow_mut().take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            for message in failures {
                open_alert(message.into(), cx);
            }
            end(cx);
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
