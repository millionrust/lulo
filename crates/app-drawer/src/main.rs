//! rmac App Drawer — a Launchpad / App-Library-style app grid.
//!
//! Scans the standard application folders, extracts each app's real icon
//! (`.icns` → cached PNG via `sips`, off the main thread), and renders a
//! searchable grid. Clicking an app launches it.
//!
//! Beyond the basic grid this adds App-Library-style ergonomics:
//!   * keyboard navigation — arrow keys move a selection cursor over the grid
//!     (or list), Return launches the selected app, Escape clears the search;
//!   * a grid/list view toggle in the toolbar;
//!   * a category derived per app (from its folder + a name keyword map) and a
//!     category filter bar.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use gpui::{
    actions, div, img, prelude::FluentBuilder as _, px, svg, AppContext as _, Context, Div, Entity,
    FocusHandle, InteractiveElement as _, IntoElement, KeyBinding, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Render, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled, Window,
};
use gpui_component::{
    input::{Input, InputState},
    StyledExt as _,
};
use rmac_ui::mac;

const TILE_W: f32 = 116.0;
const ICON: f32 = 60.0;
const ROW_ICON: f32 = 32.0;
const ACCENT: u32 = 0x0a84ff;

actions!(
    app_drawer,
    [MoveLeft, MoveRight, MoveUp, MoveDown, Launch, ClearSearch, OpenApp, RevealInFinder]
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grid,
    List,
}

/// App-Library-style buckets. Derived per app from its install folder and a
/// keyword map over its name (see [`categorize`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Productivity,
    Internet,
    Media,
    Developer,
    Utilities,
    Games,
    System,
    Other,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Category::Productivity => "Productivity",
            Category::Internet => "Internet",
            Category::Media => "Media",
            Category::Developer => "Developer",
            Category::Utilities => "Utilities",
            Category::Games => "Games",
            Category::System => "System",
            Category::Other => "Other",
        }
    }

    /// Display order for the filter bar.
    const ORDER: [Category; 8] = [
        Category::Productivity,
        Category::Internet,
        Category::Media,
        Category::Developer,
        Category::Utilities,
        Category::Games,
        Category::System,
        Category::Other,
    ];
}

#[derive(Clone)]
struct App {
    name: SharedString,
    path: PathBuf,
    icon: Option<PathBuf>,
    category: Category,
}

struct AppDrawer {
    apps: Vec<App>,
    query: Entity<InputState>,
    focus: FocusHandle,
    view: ViewMode,
    /// `None` == "All".
    filter: Option<Category>,
    /// Cursor into the currently-visible (filtered) list.
    selected: usize,
    /// Where the right-click context menu is open (window-relative), if any.
    menu_at: Option<Point<Pixels>>,
    /// Columns in the grid as last laid out — used for up/down navigation.
    cols: usize,
}

impl AppDrawer {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let apps = scan_apps();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));

        // Typing in the search field re-anchors the cursor to the first match.
        cx.observe(&query, |this: &mut AppDrawer, _, cx| {
            this.selected = 0;
            // If the active category filter no longer has any matches under the
            // new search, drop back to "All" so we never show an empty view.
            if let Some(c) = this.filter {
                if !this.present_categories(cx).contains(&c) {
                    this.filter = None;
                }
            }
            cx.notify();
        })
        .detach();

        let focus = cx.focus_handle();
        focus.focus(window);

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

        Self {
            apps,
            query,
            focus,
            view: ViewMode::Grid,
            filter: None,
            selected: 0,
            menu_at: None,
            cols: 6,
        }
    }

    /// Indices into `self.apps` that pass the current category + search filter.
    fn visible_indices(&self, cx: &gpui::App) -> Vec<usize> {
        let q = self.query.read(cx).value().to_lowercase();
        self.apps
            .iter()
            .enumerate()
            .filter(|(_, a)| self.filter.map_or(true, |c| a.category == c))
            .filter(|(_, a)| q.is_empty() || a.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    /// The set of categories actually present after the *search* filter — used
    /// to build the category bar so we never show an empty bucket.
    fn present_categories(&self, cx: &gpui::App) -> Vec<Category> {
        let q = self.query.read(cx).value().to_lowercase();
        Category::ORDER
            .into_iter()
            .filter(|c| {
                self.apps.iter().any(|a| {
                    a.category == *c && (q.is_empty() || a.name.to_lowercase().contains(&q))
                })
            })
            .collect()
    }

    fn move_by(&mut self, dx: isize, dy: isize, cx: &mut Context<Self>) {
        let count = self.visible_indices(cx).len();
        if count == 0 {
            return;
        }
        let cols = if self.view == ViewMode::List {
            1
        } else {
            self.cols.max(1)
        } as isize;
        let cur = self.selected.min(count - 1) as isize;
        let step = dx + dy * cols;
        let next = (cur + step).clamp(0, count as isize - 1);
        self.selected = next as usize;
        cx.notify();
    }

    fn launch_selected(&mut self, cx: &mut Context<Self>) {
        let vis = self.visible_indices(cx);
        if let Some(&idx) = vis.get(self.selected.min(vis.len().saturating_sub(1))) {
            let path = self.apps[idx].path.clone();
            cx.open_with_system(&path);
        }
    }

    fn selected_app_path(&self, cx: &gpui::App) -> Option<PathBuf> {
        let vis = self.visible_indices(cx);
        let &idx = vis.get(self.selected.min(vis.len().saturating_sub(1)))?;
        self.apps.get(idx).map(|a| a.path.clone())
    }

    /// Reveal the selected app in the real Finder. GPUI can't initiate a native
    /// drag out to the Dock/desktop, so this is the honest bridge: it takes you
    /// to the app in Finder, where it can be dragged onto the Dock.
    fn reveal_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_app_path(cx) {
            let _ = Command::new("open").arg("-R").arg(&path).spawn();
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.selected_app_path(cx) {
            cx.open_with_system(&path);
        }
    }

    /// The right-click menu shared by grid tiles and list rows.
    fn app_menu(pos: Point<Pixels>) -> rmac_ui::ContextMenu {
        rmac_ui::ContextMenu::new(pos)
            .item("Open", Box::new(OpenApp))
            .item("Reveal in Finder", Box::new(RevealInFinder))
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |st, cx| st.set_value("", window, cx));
        self.selected = 0;
        self.focus.focus(window);
        cx.notify();
    }

    fn set_filter(&mut self, filter: Option<Category>, cx: &mut Context<Self>) {
        self.filter = filter;
        self.selected = 0;
        cx.notify();
    }

    fn icon_element(&self, app: &App, size: f32) -> gpui::AnyElement {
        match &app.icon {
            Some(p) => img(p.clone()).w(px(size)).h(px(size)).into_any_element(),
            None => {
                // Placeholder squircle with the app initial until the icon loads.
                let initial = app
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                div()
                    .w(px(size))
                    .h(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(size * 0.23))
                    .bg(gpui::rgb(0xc9c9d0))
                    .text_color(gpui::white())
                    .text_size(px(size * 0.43))
                    .child(initial)
                    .into_any_element()
            }
        }
    }

    /// `pos` is the position in the currently-visible list (what `selected`
    /// tracks); `path` is used for the launch path lookup.
    fn tile(&self, app: &App, pos: usize, selected: bool, cx: &Context<Self>) -> impl IntoElement {
        let path = app.path.clone();
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
            .when(selected, |d: Stateful<Div>| {
                d.bg(gpui::rgba((ACCENT << 8) | 0x22))
                    .border_1()
                    .border_color(gpui::rgba((ACCENT << 8) | 0x66))
            })
            .when(!selected, |d: Stateful<Div>| {
                d.border_1().border_color(gpui::transparent_black())
            })
            .hover(|h| h.bg(gpui::rgba(0x00000010)))
            .child(self.icon_element(app, ICON))
            .child(
                div()
                    .max_w(px(TILE_W - 8.0))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .text_center()
                    .truncate()
                    .child(app.name.clone()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = pos;
                cx.open_with_system(&path);
            }))
            // Right-click selects this tile so the menu acts on it.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.selected = pos;
                    this.menu_at = Some(ev.position);
                    cx.notify();
                }),
            )
    }

    /// `pos` is the position in the currently-visible list (what `selected`
    /// tracks); `path` is used for the launch path lookup.
    fn row(&self, app: &App, pos: usize, selected: bool, cx: &Context<Self>) -> impl IntoElement {
        let path = app.path.clone();
        div()
            .id(SharedString::from(format!("row-{}", app.path.display())))
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .px_3()
            .py_1p5()
            .rounded(px(8.0))
            .when(selected, |d: Stateful<Div>| {
                d.bg(gpui::rgba((ACCENT << 8) | 0x22))
            })
            .when(!selected, |d: Stateful<Div>| {
                d.hover(|h| h.bg(gpui::rgba(0x00000008)))
            })
            .child(self.icon_element(app, ROW_ICON))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(mac::text())
                    .truncate()
                    .child(app.name.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(app.category.label()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = pos;
                cx.open_with_system(&path);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.selected = pos;
                    this.menu_at = Some(ev.position);
                    cx.notify();
                }),
            )
    }

    fn view_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let seg = |id: &'static str, glyph: &'static str, mode: ViewMode, active: bool| {
            div()
                .id(id)
                .w(px(34.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .when(active, |d: Stateful<Div>| d.bg(gpui::white()))
                .child(
                    svg()
                        .path(glyph)
                        .w(px(15.0))
                        .h(px(15.0))
                        .text_color(if active { mac::text() } else { mac::text_secondary() }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.view = mode;
                    cx.notify();
                }))
        };
        div()
            .flex()
            .items_center()
            .gap_0p5()
            .p_0p5()
            .rounded(px(7.0))
            .bg(gpui::rgb(0xe2e2e4))
            .child(seg(
                "v-grid",
                "icons/layout-dashboard.svg",
                ViewMode::Grid,
                self.view == ViewMode::Grid,
            ))
            .child(seg(
                "v-list",
                "icons/menu.svg",
                ViewMode::List,
                self.view == ViewMode::List,
            ))
    }

    fn category_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let present = self.present_categories(cx);
        let pill = |id: SharedString, label: SharedString, active: bool, target: Option<Category>| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded(px(13.0))
                .text_size(px(12.0))
                .when(active, |d: Stateful<Div>| {
                    d.bg(gpui::rgb(ACCENT)).text_color(gpui::white())
                })
                .when(!active, |d: Stateful<Div>| {
                    d.bg(gpui::rgb(0xe9e9eb))
                        .text_color(mac::text())
                        .hover(|h| h.bg(gpui::rgb(0xdedee1)))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_filter(target, cx)))
        };

        let mut bar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .justify_center()
            .px_8()
            .pb_2()
            .child(pill(
                "cat-all".into(),
                "All".into(),
                self.filter.is_none(),
                None,
            ));
        for c in present {
            bar = bar.child(pill(
                SharedString::from(format!("cat-{}", c.label())),
                c.label().into(),
                self.filter == Some(c),
                Some(c),
            ));
        }
        bar
    }
}

impl Render for AppDrawer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Recompute the live grid column count from the viewport so up/down
        // navigation matches what the user sees.
        let vw = f32::from(window.viewport_size().width);
        let usable = (vw - 64.0).max(TILE_W);
        self.cols = ((usable / (TILE_W + 8.0)).floor() as usize).max(1);

        let vis = self.visible_indices(cx);
        let sel = self.selected.min(vis.len().saturating_sub(1));

        let body: gpui::AnyElement = if self.view == ViewMode::Grid {
            let tiles = vis
                .iter()
                .enumerate()
                .map(|(pos, &idx)| self.tile(&self.apps[idx], pos, pos == sel, cx))
                .collect::<Vec<_>>();
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .justify_center()
                .children(tiles)
                .into_any_element()
        } else {
            let rows = vis
                .iter()
                .enumerate()
                .map(|(pos, &idx)| self.row(&self.apps[idx], pos, pos == sel, cx))
                .collect::<Vec<_>>();
            div()
                .v_flex()
                .gap_0p5()
                .w_full()
                .max_w(px(640.0))
                .mx_auto()
                .children(rows)
                .into_any_element()
        };

        div()
            .track_focus(&self.focus)
            .key_context("AppDrawer")
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.move_by(-1, 0, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.move_by(1, 0, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_by(0, -1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_by(0, 1, cx)))
            .on_action(cx.listener(|this, _: &Launch, _, cx| this.launch_selected(cx)))
            .on_action(cx.listener(|this, _: &OpenApp, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| this.reveal_selected(cx)))
            .on_action(cx.listener(|this, _: &ClearSearch, window, cx| {
                this.clear_search(window, cx)
            }))
            .on_action(cx.listener(|this, _: &rmac_ui::DismissMenu, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()))
            .size_full()
            .v_flex()
            .bg(gpui::rgb(0xf5f5f7))
            .text_color(mac::text())
            .child(rmac_ui::title_bar("Applications"))
            .child(
                // toolbar: centered search with the view toggle pinned right.
                div()
                    .flex()
                    .items_center()
                    .px_8()
                    .py_4()
                    .child(div().w(px(34.0)))
                    .child(
                        div().flex_1().flex().justify_center().child(
                            div()
                                .w(px(280.0))
                                .child(Input::new(&self.query).cleanable(true)),
                        ),
                    )
                    .child(self.view_toggle(cx)),
            )
            .child(self.category_bar(cx))
            .child(
                div()
                    .id("grid-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_8()
                    .pb_8()
                    .child(body),
            )
            .when_some(self.menu_at, |el: Div, pos| {
                el.child(Self::app_menu(pos).render())
            })
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
    let mut pairs: Vec<(String, PathBuf)> = Vec::new();
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
                pairs.push((name, path));
            }
        }
    }
    // Categories read each app's Info.plist (a subprocess), so resolve them in
    // parallel to keep startup fast.
    let cats = parallel_categorize(&pairs);
    apps.extend(pairs.into_iter().zip(cats).map(|((name, path), category)| App {
        name: name.into(),
        path,
        icon: None,
        category,
    }));
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Resolve every app's category concurrently (each read is an independent
/// subprocess), preserving input order.
fn parallel_categorize(pairs: &[(String, PathBuf)]) -> Vec<Category> {
    let n = pairs.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = 8.min(n);
    let chunk = n.div_ceil(workers);
    let mut result = vec![Category::Other; n];
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (ci, slice) in pairs.chunks(chunk).enumerate() {
            handles.push((ci, s.spawn(move || {
                slice.iter().map(|(name, path)| categorize(name, path)).collect::<Vec<_>>()
            })));
        }
        for (ci, h) in handles {
            if let Ok(part) = h.join() {
                let start = ci * chunk;
                for (i, c) in part.into_iter().enumerate() {
                    result[start + i] = c;
                }
            }
        }
    });
    result
}

/// The real `LSApplicationCategoryType` from an app's Info.plist, mapped to a
/// bucket — or `None` if the app declares no category.
fn real_category(path: &Path) -> Option<Category> {
    let info = path.join("Contents/Info");
    let out = Command::new("defaults")
        .arg("read")
        .arg(&info)
        .arg("LSApplicationCategoryType")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    // Apple values look like "public.app-category.developer-tools".
    Some(if s.contains("developer") {
        Category::Developer
    } else if s.contains("game") {
        Category::Games
    } else if s.contains("music")
        || s.contains("video")
        || s.contains("photo")
        || s.contains("entertainment")
        || s.contains("graphics")
    {
        Category::Media
    } else if s.contains("social") || s.contains("news") {
        Category::Internet
    } else if s.contains("utilit") {
        Category::Utilities
    } else if s.contains("productivity")
        || s.contains("business")
        || s.contains("finance")
        || s.contains("reference")
        || s.contains("education")
        || s.contains("weather")
    {
        Category::Productivity
    } else {
        Category::Other
    })
}

/// Derive an App-Library-style category. The app's real declared
/// `LSApplicationCategoryType` wins; only when a bundle declares none do we fall
/// back to install-folder rules and a best-effort name match.
fn categorize(name: &str, path: &Path) -> Category {
    // Prefer the app's real declared category; fall back to the heuristic only
    // when the bundle declares none.
    if let Some(c) = real_category(path) {
        return c;
    }
    let p = path.to_string_lossy();
    if p.contains("/Utilities/") {
        return Category::Utilities;
    }
    let n = name.to_lowercase();

    const INTERNET: &[&str] = &[
        "safari", "mail", "messages", "facetime", "chrome", "firefox", "edge", "news",
        "contacts", "freeform", "maps",
    ];
    const MEDIA: &[&str] = &[
        "music", "tv", "photos", "podcasts", "quicktime", "books", "voice memos",
        "image capture", "photo booth", "garageband", "imovie",
    ];
    const PRODUCTIVITY: &[&str] = &[
        "calendar", "notes", "reminders", "numbers", "pages", "keynote", "stocks",
        "weather", "calculator", "dictionary", "home", "clock", "shortcuts", "preview",
        "stickies", "textedit", "font book",
    ];
    const DEVELOPER: &[&str] = &[
        "xcode", "terminal", "script editor", "automator", "console", "instruments",
        "simulator", "visual studio", "code",
    ];
    const GAMES: &[&str] = &["chess", "game center"];

    let any = |list: &[&str]| list.iter().any(|k| n.contains(k));

    if any(GAMES) {
        Category::Games
    } else if any(DEVELOPER) {
        Category::Developer
    } else if any(INTERNET) {
        Category::Internet
    } else if any(MEDIA) {
        Category::Media
    } else if any(PRODUCTIVITY) {
        Category::Productivity
    } else if name == "System Settings"
        || name == "App Store"
        || name == "Find My"
        || name == "Passwords"
        || name == "Tips"
    {
        Category::System
    } else if p.starts_with("/System/Applications") {
        Category::System
    } else {
        Category::Other
    }
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
        let drawer = AppDrawer::new(window, cx);
        cx.bind_keys([
            KeyBinding::new("left", MoveLeft, Some("AppDrawer")),
            KeyBinding::new("right", MoveRight, Some("AppDrawer")),
            KeyBinding::new("up", MoveUp, Some("AppDrawer")),
            KeyBinding::new("down", MoveDown, Some("AppDrawer")),
            KeyBinding::new("enter", Launch, Some("AppDrawer")),
            KeyBinding::new("escape", ClearSearch, Some("AppDrawer")),
        ]);
        drawer
    });
}
