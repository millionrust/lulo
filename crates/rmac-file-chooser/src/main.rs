//! `rmac-file-chooser`: the rmac Open/Save panel process.
//!
//! On Linux it owns `org.freedesktop.impl.portal.desktop.rmac.filechooser`
//! (D-Bus activated through `rmac-file-chooser.service`) and opens one panel
//! window per portal request. `--preview open|save|save-files|folder` opens a
//! panel without a bus, printing the chosen URIs, for visual comparison with
//! design-lab/file-chooser.html.

mod panel;
mod panel_view;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    px, size, App, AppContext as _, AssetSource, Bounds, Result, SharedString, WindowBounds,
    WindowKind, WindowOptions,
};
use gpui_component::Root;
use rmac_file_chooser::filter::MimeDatabase;
use rmac_file_chooser::outcome::{results, Outcome};
use rmac_file_chooser::request::{Mode, RawOptions, Request};
use rmac_file_chooser::service::PanelRequest;

use crate::panel::Panel;

pub const APP_ID_OPEN: &str = "org.rmac.FileChooser";
/// The Save panel has its own id so niri can clip it at the sheet radius.
pub const APP_ID_SAVE: &str = "org.rmac.FileChooser.Save";

/// Files' icon artwork, so both apps draw identical glyphs.
#[derive(rust_embed::RustEmbed)]
#[folder = "../finder/assets"]
#[include = "icons/**/*.svg"]
struct FilesAssets;

struct CombinedAssets;

impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = FilesAssets::get(path) {
            return Ok(Some(asset.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = FilesAssets::iter()
            .filter(|asset| asset.starts_with(path))
            .map(|asset| SharedString::from(asset.to_string()))
            .collect::<Vec<_>>();
        if let Ok(mut component_assets) = gpui_component_assets::Assets.list(path) {
            assets.append(&mut component_assets);
        }
        Ok(assets)
    }
}

fn open_panel(panel: PanelRequest, database: Arc<MimeDatabase>, cx: &mut App) {
    let PanelRequest {
        request,
        reply,
        closed,
        ..
    } = panel;
    let app_id = if request.mode == Mode::Save {
        APP_ID_SAVE
    } else {
        APP_ID_OPEN
    };
    let title = if request.title.is_empty() {
        match request.mode {
            Mode::Open => "Open".to_owned(),
            _ => "Save".to_owned(),
        }
    } else {
        request.title.clone()
    };
    // The first frame is laid out at the final size; see Panel::window_size.
    let (width, height) = match request.mode {
        Mode::Save => (
            rmac_file_chooser::metrics::COMPACT_WIDTH,
            rmac_file_chooser::metrics::compact_height(
                2 + usize::from(request.filters.len() > 1) + request.choices.len(),
            ),
        ),
        _ => (
            rmac_file_chooser::metrics::PANEL_WIDTH,
            rmac_file_chooser::metrics::PANEL_HEIGHT,
        ),
    };
    let mut options: WindowOptions =
        rmac_ui::window_options_for_app_with_title(app_id, title, width, height, cx);
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    options.window_bounds = Some(WindowBounds::Windowed(bounds));
    options.window_min_size = Some(size(px(width.min(640.0)), px(height.min(360.0))));
    // A modal xdg_dialog_v1 window; see ADR 0012 for parent handling.
    options.kind = WindowKind::Dialog;
    options.is_minimizable = false;
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| Panel::new(request, reply, closed, database, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if opened.is_err() {
        eprintln!("rmac-file-chooser: the panel window could not be opened");
    }
    cx.activate(true);
}

fn preview_request(kind: &str) -> Option<Request> {
    let (mode, options) = match kind {
        "open" => (
            Mode::Open,
            RawOptions {
                multiple: Some(true),
                ..RawOptions::default()
            },
        ),
        "folder" => (
            Mode::Open,
            RawOptions {
                directory: Some(true),
                ..RawOptions::default()
            },
        ),
        "save" => (
            Mode::Save,
            RawOptions {
                current_name: Some("Untitled.txt".into()),
                filters: Some(vec![
                    ("Plain Text".into(), vec![(1, "text/plain".into())]),
                    ("All Files".into(), vec![(0, "*".into())]),
                ]),
                ..RawOptions::default()
            },
        ),
        "save-files" => (
            Mode::SaveFiles,
            RawOptions {
                files: Some(vec![b"Untitled.txt".to_vec()]),
                ..RawOptions::default()
            },
        ),
        _ => return None,
    };
    Request::from_wire(mode, String::new(), "", String::new(), options).ok()
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let preview = match (arguments.next(), arguments.next()) {
        (None, _) => None,
        (Some(flag), Some(kind)) if flag == "--preview" => match preview_request(&kind) {
            Some(request) => Some(request),
            None => {
                eprintln!("usage: rmac-file-chooser [--preview open|folder|save|save-files]");
                std::process::exit(2);
            }
        },
        _ => {
            eprintln!("usage: rmac-file-chooser [--preview open|folder|save|save-files]");
            std::process::exit(2);
        }
    };

    rmac_ui::application()
        .with_assets(CombinedAssets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            panel::bind_keys(cx);
            let database = Arc::new(MimeDatabase::load());
            match preview {
                Some(request) => {
                    let (reply, outcome) = async_channel::bounded(1);
                    // No frontend can close a preview; the sender lives with
                    // the task below so the panel never sees a Close().
                    let (keep_open, closed) = async_channel::bounded::<()>(1);
                    open_panel(
                        PanelRequest {
                            id: 0,
                            request,
                            reply,
                            closed,
                        },
                        database,
                        cx,
                    );
                    cx.spawn(async move |cx| {
                        let _keep_open = keep_open;
                        let result = outcome.recv().await.unwrap_or(Outcome::Cancelled);
                        match result {
                            Outcome::Chosen(selection) => {
                                for uri in results(&selection).map(|r| r.uris).unwrap_or_default() {
                                    println!("{uri}");
                                }
                            }
                            Outcome::Cancelled => println!("cancelled"),
                        }
                        cx.update(|cx| cx.quit());
                    })
                    .detach();
                }
                None => serve(database, cx),
            }
        });
}

#[cfg(target_os = "linux")]
fn serve(database: Arc<MimeDatabase>, cx: &mut App) {
    let (broker, panels) = rmac_file_chooser::service::Broker::channel();
    // zbus runs on its own thread; requests cross into GPUI over a channel.
    std::thread::Builder::new()
        .name("file-chooser-portal".into())
        .spawn(move || {
            async_io::block_on(async move {
                match rmac_file_chooser::dbus::serve(broker).await {
                    Ok(_connection) => std::future::pending::<()>().await,
                    Err(error) => {
                        eprintln!("rmac-file-chooser: portal backend unavailable: {error}");
                        std::process::exit(1);
                    }
                }
            })
        })
        .expect("failed to start the portal thread");
    cx.spawn(async move |cx| {
        while let Ok(panel) = panels.recv().await {
            let database = database.clone();
            cx.update(|cx| open_panel(panel, database, cx));
        }
    })
    .detach();
}

#[cfg(not(target_os = "linux"))]
fn serve(_database: Arc<MimeDatabase>, cx: &mut App) {
    eprintln!("rmac-file-chooser: the portal backend is Linux-only; use --preview");
    cx.quit();
}
