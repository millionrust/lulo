//! rmac App Drawer — a Launchpad / App-Library-style app grid.
//!
//! Scans the standard application folders, extracts each app's real icon
//! (`.icns` → cached PNG via `sips`, off the main thread), and renders a
//! searchable grid. Clicking an app launches it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use gpui::{
    div, img, px, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt as _,
};
use rmac_ui::mac;

const TILE_W: f32 = 116.0;
const ICON: f32 = 60.0;

#[derive(Clone)]
struct App {
    name: SharedString,
    path: PathBuf,
    icon: Option<PathBuf>,
}

struct AppDrawer {
    apps: Vec<App>,
    query: Entity<InputState>,
}

impl AppDrawer {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let apps = scan_apps();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        // Extract icons off the main thread, then fill them in.
        let snapshot: Vec<(usize, String, PathBuf)> = apps
            .iter()
            .enumerate()
            .map(|(i, a)| (i, a.name.to_string(), a.path.clone()))
            .collect();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let icons = cx
                .background_executor()
                .spawn(async move {
                    let cache = cache_dir();
                    snapshot
                        .into_iter()
                        .map(|(i, name, path)| (i, extract_icon(&name, &path, &cache)))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this: &mut AppDrawer, cx| {
                for (i, icon) in icons {
                    if let Some(a) = this.apps.get_mut(i) {
                        a.icon = icon;
                    }
                }
                cx.notify();
            });
        })
        .detach();

        // Live search re-filter.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();

        Self { apps, query }
    }

    fn tile(&self, app: &App, cx: &Context<Self>) -> impl IntoElement {
        let path = app.path.clone();
        let icon: gpui::AnyElement = match &app.icon {
            Some(p) => img(p.clone()).w(px(ICON)).h(px(ICON)).into_any_element(),
            None => {
                // Placeholder squircle with the app initial until the icon loads.
                let initial = app
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                div()
                    .w(px(ICON))
                    .h(px(ICON))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(14.0))
                    .bg(gpui::rgb(0xc9c9d0))
                    .text_color(gpui::white())
                    .text_size(px(26.0))
                    .child(initial)
                    .into_any_element()
            }
        };

        div()
            .id(SharedString::from(format!("app-{}", app.path.display())))
            .w(px(TILE_W))
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .px_1()
            .py_2()
            .rounded(px(10.0))
            .hover(|h| h.bg(gpui::rgba(0x00000010)))
            .child(icon)
            .child(
                div()
                    .max_w(px(TILE_W - 8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .text_center()
                    .truncate()
                    .child(app.name.clone()),
            )
            .on_click(cx.listener(move |_this, _, _, cx| {
                cx.open_with_system(&path);
            }))
    }
}

impl Render for AppDrawer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let q = self.query.read(cx).value().to_lowercase();
        let visible: Vec<App> = self
            .apps
            .iter()
            .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q))
            .cloned()
            .collect();
        let tiles = visible.iter().map(|a| self.tile(a, cx)).collect::<Vec<_>>();

        div()
            .size_full()
            .v_flex()
            .bg(gpui::rgb(0xf5f5f7))
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Applications"))
            .child(
                // centered search field
                div().flex().justify_center().py_4().child(
                    div()
                        .w(px(280.0))
                        .child(Input::new(&self.query).cleanable(true)),
                ),
            )
            .child(
                div()
                    .id("grid-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_8()
                    .pb_8()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .justify_center()
                            .children(tiles),
                    ),
            )
    }
}

// ---- app discovery & icon extraction ----

fn scan_apps() -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let dirs = [
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    for dir in dirs {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some("app") {
                    continue;
                }
                let name = path
                    .file_stem()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.is_empty() || !seen.insert(name.clone()) {
                    continue;
                }
                apps.push(App {
                    name: name.into(),
                    path,
                    icon: None,
                });
            }
        }
    }
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Library/Caches/rmac-app-drawer");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Find an app's `.icns`, convert to a cached 128px PNG (cached across launches).
fn extract_icon(name: &str, app: &Path, cache: &Path) -> Option<PathBuf> {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let out = cache.join(format!("{safe}.png"));
    if out.exists() {
        return Some(out);
    }
    let icns = icns_path(app)?;
    let ok = Command::new("sips")
        .args([
            "-s",
            "format",
            "png",
            "-Z",
            "128",
            icns.to_str()?,
            "--out",
            out.to_str()?,
        ])
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok && out.exists() {
        Some(out)
    } else {
        None
    }
}

fn icns_path(app: &Path) -> Option<PathBuf> {
    let resources = app.join("Contents/Resources");
    let plist = app.join("Contents/Info.plist");

    // Preferred: the icon named by CFBundleIconFile.
    if let Ok(out) = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIconFile", plist.to_string_lossy().as_ref()])
        .output()
    {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() {
            let mut p = resources.join(&name);
            if p.extension().is_none() {
                p.set_extension("icns");
            }
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Fallback: the largest `.icns` in Resources.
    std::fs::read_dir(&resources)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("icns"))
        .max_by_key(|p| p.metadata().map(|m| m.len()).unwrap_or(0))
}

fn main() {
    rmac_ui::boot("Applications", 1080.0, 720.0, |window, cx| {
        AppDrawer::new(window, cx)
    });
}
