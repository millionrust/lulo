//! The Force Quit Applications window (⌥⌘⎋), drawn at the measurements in
//! design-lab/force-quit.html. The resident switcher service owns it: it
//! already follows every window and holds the app names and icons, so the
//! window opens within a frame and its list follows apps as they start and
//! quit, with no polling.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui::{
    div, img, point, prelude::*, px, rgba, AnyElement, App, AsyncApp, Bounds, Context, FocusHandle,
    FontWeight, KeyDownEvent, Role, ScrollHandle, Size, TitlebarOptions, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowDecorations, WindowHandle,
    WindowOptions,
};
use rmac_shell_ui::tokens;

use crate::force_quit::{self, layout, Entry, List};
use crate::linux_wayland::Service;

// Measured on macOS 26.2 dark mode (design-lab/force-quit.html).
const DARK_BACKGROUND: u32 = 0x2023_2DFF;
const DARK_TEXT: u32 = 0xDDDD_DFFF;
const DARK_TITLE: u32 = 0x9A9C_A0FF;
const DARK_LIST_BORDER: u32 = 0x3535_35FF;
const DARK_SELECTION: u32 = 0x2558_CAFF;
const DARK_BUTTON: u32 = 0x3478_F6FF;
const WHITE: u32 = 0xFFFF_FFFF;
const FONT: &str = "Inter";

struct Colors {
    background: u32,
    text: u32,
    title: u32,
    border: u32,
    selection: u32,
    button: u32,
    not_responding: u32,
}

impl Colors {
    fn current() -> Self {
        if tokens::is_dark() {
            Self {
                background: DARK_BACKGROUND,
                text: DARK_TEXT,
                title: DARK_TITLE,
                border: DARK_LIST_BORDER,
                selection: DARK_SELECTION,
                button: DARK_BUTTON,
                not_responding: tokens::system_red(),
            }
        } else {
            Self {
                background: tokens::surface_window(),
                text: tokens::primary_text(),
                title: tokens::secondary_text(),
                border: tokens::separator(),
                selection: tokens::accent(),
                button: tokens::accent(),
                not_responding: tokens::system_red(),
            }
        }
    }
}

/// The sheet over the list: the Mac's confirmation, or a failure the user
/// has to see.
#[derive(Clone, Debug, PartialEq)]
enum Sheet {
    Confirm(Entry),
    Failed { name: String, reason: String },
}

pub(crate) struct ForceQuitView {
    service: WeakEntity<Service>,
    list: List,
    icons: BTreeMap<String, PathBuf>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    sheet: Option<Sheet>,
    active: bool,
    closing: bool,
}

impl ForceQuitView {
    fn new(
        service: WeakEntity<Service>,
        list: List,
        icons: BTreeMap<String, PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            this.active = window.is_window_active();
            cx.notify();
        })
        .detach();
        let scroll = ScrollHandle::new();
        if let Some(index) = list.selected {
            scroll.scroll_to_item(index);
        }
        Self {
            service,
            list,
            icons,
            focus,
            scroll,
            sheet: None,
            active: true,
            closing: false,
        }
    }

    /// Apps started or quit: take the new list, keeping the selection on
    /// the same app. A confirmation for an app that has gone away closes.
    pub(crate) fn refresh(
        &mut self,
        entries: Vec<Entry>,
        icons: BTreeMap<String, PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let gone = matches!(
            &self.sheet,
            Some(Sheet::Confirm(entry))
                if !entries.iter().any(|candidate| candidate.app_id == entry.app_id)
        );
        if gone {
            self.sheet = None;
            cx.notify();
        }
        let before = self.list.clone();
        self.list.refresh(entries);
        self.icons = icons;
        if self.list != before {
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        let _ = self
            .service
            .update(cx, |service, cx| service.force_quit_closed(cx));
        window.remove_window();
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.list.step(forward);
        if let Some(index) = self.list.selected {
            self.scroll.scroll_to_item(index);
        }
        cx.notify();
    }

    /// The Force Quit button: ask first, as the Mac does.
    fn request(&mut self, cx: &mut Context<Self>) {
        if self.sheet.is_some() {
            return;
        }
        if let Some(entry) = self
            .list
            .selected_entry()
            .filter(|entry| entry.can_force_quit())
        {
            self.sheet = Some(Sheet::Confirm(entry.clone()));
            cx.notify();
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(Sheet::Confirm(entry)) = self.sheet.take() else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = rmac_app_launch::terminate_application(
                entry.pids.clone(),
                rmac_app_launch::TerminationKind::ForceQuit,
            )
            .await;
            match result {
                // The windows disappear from the compositor and the list
                // follows them; nothing else to do.
                Ok(()) | Err(rmac_app_launch::TerminationError::Missing) => {}
                Err(error) => {
                    eprintln!("could not force quit {}: {error}", entry.app_id);
                    let _ = this.update(cx, |view, cx| {
                        view.sheet = Some(Sheet::Failed {
                            name: entry.name.clone(),
                            reason: error.to_string(),
                        });
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn dismiss_sheet(&mut self, cx: &mut Context<Self>) {
        if self.sheet.take().is_some() {
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        let command = modifiers.platform;
        // 0: no sheet, 1: the confirmation, 2: a failure.
        let sheet = match &self.sheet {
            None => 0,
            Some(Sheet::Confirm(_)) => 1,
            Some(Sheet::Failed { .. }) => 2,
        };
        match (sheet, key) {
            (1, "enter") => self.confirm(cx),
            (1 | 2, "enter" | "escape") => self.dismiss_sheet(cx),
            (1 | 2, ".") if command => self.dismiss_sheet(cx),
            (1 | 2, _) => {}
            (_, "up") => self.step(false, cx),
            (_, "down") => self.step(true, cx),
            (_, "home") => {
                self.list.select(0);
                self.step(false, cx);
            }
            (_, "end") => {
                self.list.select(self.list.entries.len().saturating_sub(1));
                self.step(true, cx);
            }
            (_, "enter") => self.request(cx),
            (_, "escape") => self.close(window, cx),
            (_, "w" | ".") if command => self.close(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn row(
        &self,
        index: usize,
        entry: &Entry,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.list.selected == Some(index);
        let fill = if self.active {
            colors.selection
        } else {
            tokens::light_selection()
        };
        let text = if selected && self.active {
            WHITE
        } else {
            colors.text
        };
        let label = if entry.not_responding {
            format!("{} {}", entry.name, force_quit::NOT_RESPONDING)
        } else {
            entry.name.clone()
        };
        div()
            .id(("force-quit-app", index))
            .role(Role::ListBoxOption)
            .aria_label(label)
            .aria_selected(selected)
            .relative()
            .flex_none()
            .w_full()
            .h(px(layout::ROW_HEIGHT))
            .when(selected, |row| row.bg(rgba(fill)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.list.select(index);
                cx.notify();
            }))
            .child(
                div()
                    .absolute()
                    .left(px(layout::ICON_LEFT))
                    .top(px(layout::ICON_TOP))
                    .size(px(layout::ICON))
                    .child(icon(self.icons.get(&entry.app_id), &entry.name)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(layout::NAME_LEFT))
                    .right(px(4.0))
                    .top_0()
                    .h(px(layout::ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.0))
                    .text_color(rgba(text))
                    .child(entry.name.clone())
                    .when(entry.not_responding, |line| {
                        line.child(
                            div()
                                .text_color(rgba(if selected && self.active {
                                    WHITE
                                } else {
                                    colors.not_responding
                                }))
                                .child(force_quit::NOT_RESPONDING),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_sheet(&self, sheet: &Sheet, colors: &Colors, cx: &mut Context<Self>) -> AnyElement {
        let metrics = tokens::current().metrics;
        let width = metrics.alert_width;
        let (title, message, confirm) = match sheet {
            Sheet::Confirm(entry) => (
                force_quit::confirmation_title(&entry.name),
                force_quit::CONFIRMATION_MESSAGE.to_owned(),
                true,
            ),
            Sheet::Failed { name, reason } => (
                format!("Lulo OS couldn\u{2019}t force \u{201c}{name}\u{201d} to quit."),
                format!(
                    "{}. Try again, or open System Monitor to end its processes.",
                    capitalized(reason)
                ),
                false,
            ),
        };
        let icon_path = match sheet {
            Sheet::Confirm(entry) => self.icons.get(&entry.app_id).cloned(),
            Sheet::Failed { .. } => None,
        };
        let button = |id: &'static str, label: &'static str, default: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .flex_1()
                .h(px(metrics.alert_button_height))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_size(px(13.0))
                .text_color(rgba(if default { WHITE } else { colors.text }))
                .bg(rgba(if default {
                    if confirm {
                        tokens::system_red()
                    } else {
                        colors.button
                    }
                } else {
                    tokens::fill_control()
                }))
                .child(label)
        };
        let buttons = if confirm {
            div()
                .w_full()
                .flex()
                .gap(px(metrics.alert_button_gap))
                .child(
                    button("force-quit-cancel", "Cancel", false)
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_sheet(cx))),
                )
                .child(
                    button("force-quit-confirm", "Force Quit", true)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                )
        } else {
            div().w_full().flex().child(
                button("force-quit-ok", "OK", true)
                    .on_click(cx.listener(|this, _, _, cx| this.dismiss_sheet(cx))),
            )
        };
        div()
            .id("force-quit-sheet-layer")
            .absolute()
            .inset_0()
            .occlude()
            .child(
                div()
                    .id("force-quit-sheet")
                    .role(Role::Alert)
                    .aria_label(title.clone())
                    .absolute()
                    .top(px(metrics.titlebar_height))
                    .left(px(((layout::WIDTH - width) / 2.0).max(0.0)))
                    .w(px(width))
                    .p(px(metrics.alert_padding))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(tokens::card_radius()))
                    .bg(rgba(tokens::regular_dark_tint()))
                    .border_1()
                    .border_color(rgba(tokens::light_border()))
                    .shadow_lg()
                    .text_color(rgba(colors.text))
                    .when_some(icon_path, |sheet, path| {
                        sheet.child(img(path).size(px(metrics.alert_icon)))
                    })
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_center()
                            .child(title),
                    )
                    .child(div().text_size(px(11.0)).text_center().child(message))
                    .child(buttons),
            )
            .into_any_element()
    }
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// The app's artwork, or a plain plate with the name's initial.
fn icon(path: Option<&PathBuf>, name: &str) -> AnyElement {
    if let Some(path) = path {
        return img(path.clone()).size(px(layout::ICON)).into_any_element();
    }
    div()
        .size(px(layout::ICON))
        .rounded(px(4.0))
        .bg(rgba(0x8E8E_93FF))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgba(WHITE))
        .child(name.chars().next().map(String::from).unwrap_or_default())
        .into_any_element()
}

/// The close / minimise / zoom cluster. Force Quit is a fixed-size dialogue:
/// only close works, so minimise and zoom stay grey.
fn traffic_lights(active: bool, cx: &mut Context<ForceQuitView>) -> AnyElement {
    let design = tokens::current();
    let metrics = design.metrics;
    let colors = design.colors;
    let origin = metrics.traffic_center_titlebar - metrics.traffic_hit / 2.0;
    let light = |index: usize, fill: gpui::Hsla, border: gpui::Hsla| {
        div()
            .absolute()
            .left(px(origin + index as f32 * metrics.traffic_spacing))
            .top(px(origin))
            .size(px(metrics.traffic_hit))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(metrics.traffic_diameter))
                    .rounded_full()
                    .border_1()
                    .bg(fill)
                    .border_color(border),
            )
    };
    let off = (
        tokens::hsla(colors.traffic_inactive),
        tokens::hsla(colors.traffic_inactive_border),
    );
    let (close_fill, close_border) = if active {
        (
            tokens::hsla(colors.traffic_close),
            tokens::hsla(colors.traffic_close_border),
        )
    } else {
        off
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .child(
            light(0, close_fill, close_border)
                .id("force-quit-close")
                .role(Role::Button)
                .aria_label("Close")
                .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
        )
        .child(light(1, off.0, off.1))
        .child(light(2, off.0, off.1))
        .into_any_element()
}

impl Render for ForceQuitView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::current();
        let metrics = tokens::current().metrics;
        let rows = self
            .list
            .entries
            .clone()
            .iter()
            .enumerate()
            .map(|(index, entry)| self.row(index, entry, &colors, cx))
            .collect::<Vec<_>>();
        let can_force_quit = self
            .list
            .selected_entry()
            .is_some_and(Entry::can_force_quit);
        let sheet = self
            .sheet
            .clone()
            .map(|sheet| self.render_sheet(&sheet, &colors, cx));
        div()
            .id("force-quit")
            .track_focus(&self.focus)
            .role(Role::Dialog)
            .aria_label(force_quit::TITLE)
            .relative()
            .size_full()
            .bg(rgba(colors.background))
            .font_family(FONT)
            .text_color(rgba(colors.text))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .id("force-quit-title-bar")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(metrics.titlebar_height))
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(traffic_lights(self.active, cx))
            .child(
                div()
                    .absolute()
                    .left(px(layout::TITLE_LEFT))
                    .top(px(layout::TITLE_TOP))
                    .h(px(layout::TITLE_HEIGHT))
                    .text_size(px(metrics.title_titlebar_size))
                    .line_height(px(layout::TITLE_HEIGHT))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgba(colors.title))
                    .child(force_quit::TITLE),
            )
            .child(
                div()
                    .absolute()
                    .left(px(layout::TEXT_LEFT))
                    .top(px(layout::INSTRUCTION_TOP))
                    .w(px(layout::INSTRUCTION_WIDTH))
                    .text_size(px(13.0))
                    .line_height(px(layout::INSTRUCTION_LINE))
                    .child(force_quit::INSTRUCTION),
            )
            .child(
                div()
                    .id("force-quit-list")
                    .role(Role::ListBox)
                    .aria_label("Applications")
                    .absolute()
                    .left(px(layout::LIST_LEFT))
                    .top(px(layout::LIST_TOP))
                    .w(px(layout::LIST_WIDTH))
                    .h(px(layout::LIST_HEIGHT))
                    .border_1()
                    .border_color(rgba(colors.border))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
            .child(
                div()
                    .absolute()
                    .left(px(layout::TEXT_LEFT))
                    .top(px(layout::FOOTER_TOP))
                    .w(px(layout::FOOTER_WIDTH))
                    .text_size(px(11.0))
                    .line_height(px(layout::FOOTER_LINE))
                    .child(force_quit::FOOTER),
            )
            .child(
                div()
                    .id("force-quit-button")
                    .role(Role::Button)
                    .aria_label("Force Quit")
                    .absolute()
                    .left(px(layout::BUTTON_LEFT))
                    .top(px(layout::BUTTON_TOP))
                    .w(px(layout::BUTTON_WIDTH))
                    .h(px(layout::BUTTON_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_size(px(13.0))
                    .text_color(rgba(if can_force_quit {
                        WHITE
                    } else {
                        tokens::disabled_text()
                    }))
                    .bg(rgba(if can_force_quit && self.active {
                        colors.button
                    } else {
                        tokens::fill_control()
                    }))
                    .when(can_force_quit, |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.request(cx)))
                    })
                    .child("Force Quit"),
            )
            .children(sheet)
    }
}

/// Open the window, or bring the open one forward.
pub(crate) fn open(
    service: &gpui::Entity<Service>,
    list: List,
    icons: BTreeMap<String, PathBuf>,
    cx: &mut App,
) -> Option<WindowHandle<ForceQuitView>> {
    let weak = service.downgrade();
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(force_quit::TITLE.into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        focus: true,
        show: true,
        // niri centres a new floating window; only the size matters here.
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: Size::new(px(layout::WIDTH), px(layout::HEIGHT)),
        })),
        app_id: Some(force_quit::APP_ID.to_owned()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match cx.open_window(options, move |window, cx| {
        cx.new(|cx| ForceQuitView::new(weak, list, icons, window, cx))
    }) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("could not open Force Quit Applications: {error}");
            None
        }
    }
}
