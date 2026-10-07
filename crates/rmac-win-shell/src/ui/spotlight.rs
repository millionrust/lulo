//! Spotlight: Alt+Space (or the bar's magnifier) opens a search field in
//! the upper middle of the screen. It finds apps (the Lulo apps and every
//! app in Windows' Apps folder) and files in the user's own folders, and
//! Return opens the selected one. Esc, or a click outside, closes it and
//! gives the keyboard back to the app in front.

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

const WIDTH: f32 = 680.0;
const FIELD: f32 = 56.0;
const ROW: f32 = 34.0;
const SECTION: f32 = 24.0;
const RESULTS: usize = 9;
/// The window holds the field and the longest result list; the clear part
/// below the panel closes Spotlight when clicked.
const HEIGHT: f32 = FIELD + 12.0 + RESULTS as f32 * ROW + 3.0 * SECTION + 16.0;

pub(crate) struct SpotlightView {
    shell: Entity<ShellState>,
    query: Entity<InputState>,
    results: Vec<Entry>,
    selected: usize,
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
        let observe = cx.observe(&shell, |_, _, cx| cx.notify());
        cx.set_global(SpotlightEntity(cx.entity()));
        Self {
            shell,
            query,
            results: Vec::new(),
            selected: 0,
            _subscriptions: vec![changed, activation, observe],
        }
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
        self.results = results;
        self.selected = 0;
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
        Target::Shell(target) => launch::open(launch::Request::Shell(target.clone())),
    }
}

fn rect(cx: &App) -> RECT {
    let (monitor, _) = surface::primary_monitor();
    let scale = runtime(cx).scale;
    let width = (WIDTH * scale).round() as i32;
    let height = (HEIGHT * scale).round() as i32;
    let left = monitor.left + ((monitor.right - monitor.left) - width) / 2;
    let top = monitor.top + ((monitor.bottom - monitor.top) as f32 * 0.2) as i32;
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
    let bounds = super::logical(rect(cx), runtime(cx).scale);
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
    let rect = rect(cx);
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
    let front = shell.update(cx, |state, cx| {
        state.spotlight_open = false;
        cx.notify();
        state.front.as_ref().map(|front| front.hwnd)
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
                    .w(px(WIDTH - 16.0))
                    .mt(px(8.0))
                    .flex()
                    .flex_col()
                    .rounded(px(mac::radius_large_surface()))
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
