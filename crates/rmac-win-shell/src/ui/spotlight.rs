//! Spotlight: Alt+Space (or the bar's magnifier) opens a search field in
//! the upper middle of the screen. It finds apps (the Lulo apps and every
//! app in Windows' Apps folder) and files in the user's own folders, and
//! Return opens the selected one (a folder in Lulo's Files). Esc, or a
//! click outside, closes it and gives the keyboard back to the app in front.
//!
//! As on the Mac, it opens as the search bar alone and grows downwards
//! when there are results (WIN-OS-44): its window is the panel plus a clear
//! margin for the shadow, resized to what it shows.

use gpui::{
    div, img, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity,
    Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window,
};
use rmac_ui::{mac, IconSource, InputEvent, InputState, TextField};
use windows::Win32::Foundation::RECT;

use super::{load_catalog, runtime, shell, ShellState};
use crate::model::apps;
use crate::model::search::{self, Entry, Kind, Target};
use crate::win::{launch, surface, trace, windows_list};

/// The Mac's Spotlight panel: 640 × 56 pt as the bar alone.
const WIDTH: f32 = 640.0;
const FIELD: f32 = 56.0;
const ROW: f32 = 34.0;
const SECTION: f32 = 24.0;
const RESULTS: usize = 9;
/// Clear room round the panel for its shadow.
const MARGIN: f32 = 24.0;
/// The panel's top edge, as a share of the screen's height (the Mac's
/// 190 pt on a 956 pt screen): in the upper third.
const TOP: f32 = 0.2;

/// The panel's height for `results`: the field alone, or the field and the
/// list (its section headings, rows and padding).
fn panel_height(results: &[Entry]) -> f32 {
    if results.is_empty() {
        return FIELD;
    }
    let mut sections = 0;
    let mut last: Option<Kind> = None;
    for entry in results {
        if last != Some(entry.kind) {
            sections += 1;
            last = Some(entry.kind);
        }
    }
    FIELD + 1.0 + 4.0 + results.len() as f32 * ROW + sections as f32 * SECTION + 8.0
}

pub(crate) struct SpotlightView {
    shell: Entity<ShellState>,
    query: Entity<InputState>,
    results: Vec<Entry>,
    selected: usize,
    /// The catalogue's generation the results were found in.
    catalog_seen: u64,
    _subscriptions: Vec<Subscription>,
}

impl SpotlightView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shell = shell(cx);
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Spotlight Search"));
        let changed = cx.subscribe(&query, |this, query, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = query.read(cx).value().to_string();
                this.search(&text, cx);
            }
        });
        // Clicking another window closes Spotlight, as on the Mac.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() && this.shell.read(cx).spotlight_open {
                cx.defer(|cx| hide(false, cx));
            }
        });
        // The catalogue loads the first time Spotlight opens: what was typed
        // before it arrived is searched again when it does.
        let observe = cx.observe(&shell, |this, shell, cx| {
            let generation = shell.read(cx).catalog_generation;
            if generation != this.catalog_seen {
                this.catalog_seen = generation;
                let text = this.query.read(cx).value().to_string();
                if !text.is_empty() {
                    this.search(&text, cx);
                }
            }
            cx.notify();
        });
        let entity = cx.entity();
        cx.set_global(SpotlightEntity(entity));
        let catalog_seen = shell.read(cx).catalog_generation;
        Self {
            shell,
            query,
            results: Vec::new(),
            selected: 0,
            catalog_seen,
            _subscriptions: vec![changed, activation, observe],
        }
    }

    /// Grow or shrink the window to the panel's height for what it shows.
    fn fit_window(&self, cx: &mut Context<Self>) {
        let Some(spotlight) = runtime(cx).spotlight else {
            return;
        };
        let rect = rect(cx, panel_height(&self.results));
        super::later(cx, move || {
            surface::set_bounds(windows_list::handle(spotlight.hwnd), rect)
        });
        trace(|| format!("spotlight panel {:.0} high", panel_height(&self.results)));
    }

    fn search(&mut self, text: &str, cx: &mut Context<Self>) {
        let results = {
            let state = self.shell.read(cx);
            search::search(text, state.apps.iter().chain(state.files.iter()), RESULTS)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        };
        let wanted = results
            .iter()
            .filter_map(|entry| match &entry.target {
                Target::Shell(target) => Some(target.clone()),
                Target::Lulo(_) => None,
            })
            .collect::<Vec<_>>();
        self.shell.update(cx, |state, _| {
            for source in &wanted {
                state.want_icon(source);
            }
        });
        trace(|| format!("spotlight {text:?}: {} results", results.len()));
        let resized = panel_height(&results) != panel_height(&self.results);
        self.results = results;
        self.selected = 0;
        if resized && self.shell.read(cx).spotlight_open {
            self.fit_window(cx);
        }
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |query, cx| query.set_value("", window, cx));
        self.results.clear();
        self.selected = 0;
        self.query.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
}

fn open_entry(entry: &Entry, cx: &mut App) {
    trace(|| format!("spotlight open {}", entry.name));
    hide(false, cx);
    match &entry.target {
        Target::Lulo(exe) => launch::open(launch::Request::Lulo((*exe).to_owned())),
        // Folders open in Lulo's Files, as the Finder opens them.
        Target::Shell(target) if entry.kind == Kind::Folder => {
            launch::open(launch::folder_request(target))
        }
        Target::Shell(target) => launch::open(launch::Request::Shell(target.clone())),
    }
}

/// The window for a panel `panel` points high: centred, its panel's top at
/// the Mac's height in the upper third, with the shadow's margin round it.
fn rect(cx: &App, panel: f32) -> RECT {
    let (monitor, _) = surface::primary_monitor();
    let scale = runtime(cx).scale;
    let width = ((WIDTH + 2.0 * MARGIN) * scale).round() as i32;
    let height = ((panel + 2.0 * MARGIN) * scale).round() as i32;
    let left = monitor.left + ((monitor.right - monitor.left) - width) / 2;
    let top = monitor.top + ((monitor.bottom - monitor.top) as f32 * TOP) as i32
        - (MARGIN * scale).round() as i32;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// Make Spotlight's window, hidden, so the hotkey shows it at once.
pub(crate) fn prepare(cx: &mut App) {
    if runtime(cx).spotlight.is_some() {
        return;
    }
    let bounds = super::logical(rect(cx, FIELD), runtime(cx).scale);
    if let Some(spotlight) = super::open_surface(bounds, true, cx, SpotlightView::new) {
        cx.global_mut::<super::Runtime>().spotlight = Some(spotlight);
    }
}

pub(crate) fn toggle(cx: &mut App) {
    if shell(cx).read(cx).spotlight_open {
        hide(true, cx);
    } else {
        show(cx);
    }
}

fn show(cx: &mut App) {
    super::keep_panel(super::Panel::Spotlight, cx);
    prepare(cx);
    let Some(spotlight) = runtime(cx).spotlight else {
        return;
    };
    super::menu::close(false, cx);
    // Apps or files that changed since the last look are read again now.
    load_catalog(cx);
    shell(cx).update(cx, |state, cx| {
        state.spotlight_open = true;
        cx.notify();
    });
    with_view(cx, |view, window, cx| view.reset(window, cx));
    // The search bar alone until there are results.
    let rect = rect(cx, FIELD);
    super::later(cx, move || {
        surface::show_focused_at(windows_list::handle(spotlight.hwnd), rect)
    });
    trace(|| format!("spotlight shown at {:.0} ms", crate::win::process_millis()));
}

/// Close Spotlight; with `give_back`, the app in front gets the keyboard
/// again (an app Spotlight opens takes it instead).
pub(crate) fn hide(give_back: bool, cx: &mut App) {
    let shell = shell(cx);
    if !shell.read(cx).spotlight_open {
        return;
    }
    let spotlight = runtime(cx).spotlight.map(|spotlight| spotlight.hwnd);
    // The app in front gets the keyboard back; with the desktop in front
    // (Lulo mode), Lulo's desktop does.
    let desktop = runtime(cx).desktop.map(|desktop| desktop.hwnd);
    let front = shell.update(cx, |state, cx| {
        state.spotlight_open = false;
        cx.notify();
        state.front.as_ref().map(|front| front.hwnd).or(desktop)
    });
    super::later(cx, move || {
        if let Some(spotlight) = spotlight {
            surface::hide(windows_list::handle(spotlight));
        }
        if give_back {
            if let Some(hwnd) = front {
                windows_list::activate(hwnd);
            }
        }
    });
    trace(|| "spotlight hidden".into());
    super::release_later(super::Panel::Spotlight, cx);
}

/// Let go of Spotlight's window, its textures and the file list a while
/// after it closed; the next open makes them again.
pub(crate) fn release(cx: &mut App) {
    let shell = shell(cx);
    if shell.read(cx).spotlight_open {
        return;
    }
    let Some(spotlight) = cx.global_mut::<super::Runtime>().spotlight.take() else {
        return;
    };
    if cx.has_global::<SpotlightEntity>() {
        cx.remove_global::<SpotlightEntity>();
    }
    let _ = spotlight
        .handle
        .update(cx, |_, window, _| window.remove_window());
    shell.update(cx, |state, _| state.release_catalog());
    trace(|| "spotlight released".into());
}

struct SpotlightEntity(Entity<SpotlightView>);

impl gpui::Global for SpotlightEntity {}

fn with_view(
    cx: &mut App,
    f: impl FnOnce(&mut SpotlightView, &mut Window, &mut Context<SpotlightView>),
) {
    let (Some(spotlight), Some(entity)) = (
        runtime(cx).spotlight,
        cx.try_global::<SpotlightEntity>()
            .map(|view| view.0.clone()),
    ) else {
        return;
    };
    let _ = spotlight.handle.update(cx, |_, window, cx| {
        entity.update(cx, |view, cx| f(view, window, cx));
    });
}

impl Render for SpotlightView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        let state = self.shell.read(cx);
        let mut rows = Vec::new();
        let mut last_kind: Option<Kind> = None;
        for (index, entry) in self.results.iter().enumerate() {
            if last_kind != Some(entry.kind) {
                last_kind = Some(entry.kind);
                rows.push(
                    div()
                        .h(px(SECTION))
                        .px(px(14.0))
                        .flex()
                        .items_end()
                        .pb(px(3.0))
                        .text_size(px(11.0))
                        .font_weight(mac::SEMIBOLD)
                        .text_color(mac::text_secondary())
                        .child(entry.kind.section())
                        .into_any_element(),
                );
            }
            let icon = match &entry.target {
                Target::Lulo(exe) => match apps::lulo_app_for_exe(exe) {
                    Some(app) => rmac_ui::svg_icon(IconSource::from(app.icon), 22.0, scale, cx)
                        .size(px(22.0))
                        .into_any_element(),
                    None => div().size(px(22.0)).into_any_element(),
                },
                Target::Shell(target) => match state.cached_icon(target) {
                    Some(image) => img(image).size(px(22.0)).into_any_element(),
                    None => div()
                        .size(px(22.0))
                        .rounded(px(mac::radius_control()))
                        .bg(mac::control_fill())
                        .into_any_element(),
                },
            };
            let selected = index == self.selected;
            let clicked = entry.clone();
            rows.push(
                div()
                    .id(("lulo-spotlight-row", index))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(entry.name.clone()))
                    .aria_selected(selected)
                    .h(px(ROW))
                    .mx(px(6.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(mac::radius_menu_item()))
                    .when(selected, |row| {
                        row.bg(mac::accent()).text_color(mac::on_accent())
                    })
                    .child(icon)
                    .child(div().flex_1().truncate().child(entry.name.clone()))
                    .when(!entry.location.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(px(11.0))
                                .when(!selected, |text| text.text_color(mac::text_secondary()))
                                .truncate()
                                .max_w(px(260.0))
                                .child(entry.location.clone()),
                        )
                    })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        let entry = clicked.clone();
                        cx.defer(move |cx| open_entry(&entry, cx));
                    })
                    .into_any_element(),
            );
        }
        let panel_fill = mac::menu_surface();
        let has_results = !rows.is_empty();
        div()
            .id("lulo-spotlight")
            .size_full()
            .p(px(MARGIN))
            .flex()
            .flex_col()
            .items_center()
            .text_color(mac::text())
            .text_size(px(13.0))
            // A click on the clear part closes Spotlight.
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.defer(|cx| hide(true, cx));
            })
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => cx.defer(|cx| hide(true, cx)),
                    "down" => {
                        if this.selected + 1 < this.results.len() {
                            this.selected += 1;
                        }
                        cx.notify();
                    }
                    "up" => {
                        this.selected = this.selected.saturating_sub(1);
                        cx.notify();
                    }
                    "enter" => {
                        let entry = this.results.get(this.selected).cloned();
                        if let Some(entry) = entry {
                            cx.defer(move |cx| open_entry(&entry, cx));
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id("lulo-spotlight-panel")
                    .role(Role::Dialog)
                    .aria_label("Spotlight")
                    .w(px(WIDTH))
                    .flex()
                    .flex_col()
                    // A pill as the bar alone, a rounded panel with results.
                    .rounded(px(if has_results {
                        mac::radius_large_surface()
                    } else {
                        FIELD / 2.0
                    }))
                    .bg(panel_fill)
                    .border_1()
                    .border_color(mac::separator())
                    .shadow(mac::menu_shadow())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(FIELD))
                            .px(px(18.0))
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                gpui::svg()
                                    .path("symbols/search.svg")
                                    .size(px(20.0))
                                    .text_color(mac::text_secondary()),
                            )
                            .child(
                                TextField::new(&self.query)
                                    .appearance(false)
                                    .flex_1()
                                    .text_size(px(22.0)),
                            ),
                    )
                    .when(has_results, |panel| {
                        panel.child(
                            div()
                                .border_t_1()
                                .border_color(mac::separator())
                                .pt(px(4.0))
                                .pb(px(8.0))
                                .flex()
                                .flex_col()
                                .children(rows),
                        )
                    }),
            )
    }
}
