//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · Apple Account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). On macOS the
//! panes are read-only mockups; real backends (gsettings/dconf/compositor) land
//! on the Linux target.

use std::borrow::Cow;
use std::process::Command;

use gpui::{
    div, prelude::FluentBuilder as _, px, svg, AssetSource, Context, Div, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement, Render, Result, SharedString,
    StatefulInteractiveElement as _, Stateful, Styled, Svg, Window,
};
use gpui_component::StyledExt as _;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct AppAssets;

struct CombinedAssets;
impl AssetSource for CombinedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(f) = AppAssets::get(path) {
            return Ok(Some(f.data));
        }
        gpui_component_assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut v: Vec<SharedString> = AppAssets::iter()
            .filter(|p| p.starts_with(path))
            .map(|p| SharedString::from(p.to_string()))
            .collect();
        if let Ok(mut o) = gpui_component_assets::Assets.list(path) {
            v.append(&mut o);
        }
        Ok(v)
    }
}

fn hsl(h: u32) -> Hsla {
    gpui::rgb(h).into()
}
fn sidebar_bg() -> Hsla { hsl(0xe7e7ea) }
fn pane_bg() -> Hsla { hsl(0xf2f2f4) }
fn card_bg() -> Hsla { hsl(0xffffff) }
fn accent() -> Hsla { hsl(0x0a84ff) }
fn label() -> Hsla { hsl(0x1d1d1f) }
fn secondary() -> Hsla { hsl(0x86868b) }
fn sep() -> Hsla { hsl(0xe5e5e5) }
fn white() -> Hsla { gpui::white() }

fn glyph(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg().path(path).w(px(size)).h(px(size)).text_color(color).flex_none()
}

/// A colored rounded-square icon tile (SF-symbol-on-color, like Settings).
fn tile(path: &'static str, bg: Hsla, size: f32) -> impl IntoElement {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.28))
        .bg(bg)
        .child(glyph(path, size * 0.62, white()))
}

#[derive(Clone)]
struct Row {
    icon: &'static str,
    color: Hsla,
    label: SharedString,
}

#[derive(Clone)]
struct Category {
    name: SharedString,
    icon: &'static str,
    color: Hsla,
    desc: SharedString,
    cards: Vec<Vec<Row>>,
}

struct Settings {
    account: SharedString,
    sections: Vec<Vec<Category>>,
    selected: (usize, usize),
    dragging: bool,
}

impl Settings {
    fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            account: account_name().into(),
            sections: categories(),
            selected: (1, 0), // General
            dragging: false,
        }
    }

    fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("topbar")
            .h(px(52.0))
            .flex_none()
            .w_full()
            .flex()
            .on_mouse_down(MouseButton::Left, cx.listener(|t, _, _, _| t.dragging = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|t, _, _, _| t.dragging = false))
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
                    window.start_window_move();
                }
            }))
            .child(div().w(px(SIDEBAR_W)).h_full().bg(sidebar_bg()))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(pane_bg())
                    .flex()
                    .items_center()
                    .pl_5()
                    .gap_2()
                    .child(glyph("icons/chevron-left.svg", 17.0, secondary()))
                    .child(glyph("icons/chevron-right.svg", 17.0, hsl(0xc4c4c8))),
            )
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let search = div()
            .mx_2()
            .mt_1()
            .mb_2()
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .rounded(px(7.0))
            .bg(hsl(0xdcdce0))
            .child(glyph("icons/search.svg", 13.0, secondary()))
            .child(div().text_size(px(13.0)).text_color(secondary()).child("Search"));

        let account = div()
            .flex()
            .items_center()
            .gap_2p5()
            .mx_2()
            .mb_2()
            .px_2()
            .py_1p5()
            .rounded(px(8.0))
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(hsl(0xc7c7cc))
                    .child(glyph("icons/user.svg", 22.0, white())),
            )
            .child(
                div()
                    .v_flex()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(self.account.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(secondary())
                            .child("Apple Account"),
                    ),
            );

        let mut col = div()
            .id("sidebar-scroll")
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_1()
            .bg(sidebar_bg())
            .border_r_1()
            .border_color(sep())
            .overflow_y_scroll()
            .child(search)
            .child(account);

        for (si, section) in self.sections.iter().enumerate() {
            if si > 0 {
                col = col.child(div().h(px(14.0)));
            }
            for (ci, cat) in section.iter().enumerate() {
                let selected = self.selected == (si, ci);
                col = col.child(
                    div()
                        .id(SharedString::from(format!("cat-{si}-{ci}")))
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .h(px(30.0))
                        .mx_2()
                        .px_2()
                        .rounded(px(6.0))
                        .when(selected, |el: Stateful<Div>| el.bg(accent()))
                        .when(!selected, |el: Stateful<Div>| el.hover(|h| h.bg(hsl(0x00000008))))
                        .child(tile(cat.icon, cat.color, 20.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(if selected { white() } else { label() })
                                .child(cat.name.clone()),
                        )
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.selected = (si, ci);
                            cx.notify();
                        })),
                );
            }
        }
        col
    }

    fn render_detail(&self, _cx: &Context<Self>) -> impl IntoElement {
        let cat = self.current();

        let hero = div()
            .v_flex()
            .items_center()
            .gap_2()
            .pt_6()
            .pb_5()
            .child(tile(cat.icon, cat.color, 64.0))
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(cat.name.clone()),
            )
            .child(
                div()
                    .max_w(px(440.0))
                    .text_center()
                    .text_size(px(13.0))
                    .text_color(secondary())
                    .child(cat.desc.clone()),
            );

        let cards = cat.cards.iter().map(|rows| {
            let n = rows.len();
            let row_els = rows.iter().enumerate().map(|(i, r)| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(44.0))
                    .px_3()
                    .when(i + 1 < n, |el: Div| el.border_b_1().border_color(sep()))
                    .child(tile(r.icon, r.color, 22.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(label())
                            .child(r.label.clone()),
                    )
                    .child(glyph("icons/chevron-right.svg", 14.0, hsl(0xc4c4c8)))
            });
            div()
                .v_flex()
                .mb_3()
                .rounded(px(10.0))
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .children(row_els)
        });

        div()
            .id("detail-scroll")
            .flex_1()
            .h_full()
            .bg(pane_bg())
            .overflow_y_scroll()
            .child(
                div()
                    .max_w(px(560.0))
                    .mx_auto()
                    .px_5()
                    .pb_8()
                    .child(hero)
                    .children(cards),
            )
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .bg(pane_bg())
            .text_color(label())
            .child(self.render_topbar(cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .child(self.render_sidebar(cx))
                    .child(self.render_detail(cx)),
            )
    }
}

const SIDEBAR_W: f32 = 248.0;

fn account_name() -> String {
    Command::new("id")
        .arg("-F")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "User".into()))
}

fn categories() -> Vec<Vec<Category>> {
    let blue = hsl(0x0a84ff);
    let gray = hsl(0x8e8e93);
    let green = hsl(0x34c759);
    let red = hsl(0xff3b30);
    let pink = hsl(0xff2d55);
    let indigo = hsl(0x5e5ce6);
    let purple = hsl(0xaf52de);
    let teal = hsl(0x30b0c7);

    let row = |icon: &'static str, color: Hsla, label: &str| Row {
        icon,
        color,
        label: label.to_string().into(),
    };
    let cat = |name: &str, icon: &'static str, color: Hsla, desc: &str, cards: Vec<Vec<Row>>| {
        Category {
            name: name.to_string().into(),
            icon,
            color,
            desc: desc.to_string().into(),
            cards,
        }
    };

    vec![
        vec![
            cat("Wi-Fi", "icons/wifi.svg", blue, "Connect to Wi-Fi networks and manage known networks.", vec![]),
            cat("Bluetooth", "icons/bluetooth.svg", blue, "Pair and manage Bluetooth devices.", vec![]),
            cat("Network", "icons/globe.svg", blue, "Configure network services and connections.", vec![]),
            cat("VPN", "icons/key.svg", blue, "Set up and manage VPN configurations.", vec![]),
            cat("Battery", "icons/battery-charging.svg", green, "Monitor battery usage and energy settings.", vec![]),
        ],
        vec![
            cat(
                "General",
                "icons/settings.svg",
                gray,
                "Manage your overall setup and preferences for Mac, such as software updates, device language, AirDrop, and more.",
                vec![
                    vec![
                        row("icons/info.svg", gray, "About"),
                        row("icons/refresh-cw.svg", gray, "Software Update"),
                        row("icons/database.svg", gray, "Storage"),
                    ],
                    vec![row("icons/heart-handshake.svg", red, "AppleCare & Warranty")],
                    vec![row("icons/folder-symlink.svg", blue, "AirDrop & Continuity")],
                    vec![
                        row("icons/key.svg", gray, "AutoFill & Passwords"),
                        row("icons/clock.svg", gray, "Date & Time"),
                        row("icons/languages.svg", blue, "Language & Region"),
                        row("icons/power.svg", gray, "Login Items & Extensions"),
                        row("icons/folder-symlink.svg", blue, "Sharing"),
                        row("icons/hard-drive.svg", gray, "Startup Disk"),
                        row("icons/history.svg", green, "Time Machine"),
                    ],
                ],
            ),
            cat("Accessibility", "icons/accessibility.svg", blue, "Customize your Mac for the way you work.", vec![]),
            cat("Appearance", "icons/palette.svg", hsl(0x1d1d1f), "Change how windows, buttons, and menus look.", vec![]),
            cat("Apple Intelligence & Siri", "icons/sparkles.svg", purple, "Set up Apple Intelligence and Siri.", vec![]),
            cat("Desktop & Dock", "icons/app-window.svg", gray, "Adjust the Dock, Stage Manager, and windows.", vec![]),
            cat("Displays", "icons/monitor.svg", blue, "Arrange displays and adjust resolution.", vec![]),
            cat("Spotlight", "icons/search.svg", gray, "Choose which categories Spotlight searches.", vec![]),
            cat("Wallpaper", "icons/image.svg", teal, "Choose a wallpaper for your desktop.", vec![]),
        ],
        vec![
            cat("Notifications", "icons/bell.svg", red, "Choose how you receive notifications.", vec![]),
            cat("Sound", "icons/volume-2.svg", pink, "Adjust sound effects and output.", vec![]),
            cat("Focus", "icons/moon.svg", indigo, "Stay focused by silencing notifications.", vec![]),
            cat("Screen Time", "icons/timer.svg", indigo, "Monitor usage and set limits.", vec![]),
        ],
        vec![
            cat("Lock Screen", "icons/lock.svg", gray, "Adjust your lock screen and login.", vec![]),
            cat("Privacy & Security", "icons/shield.svg", blue, "Control what your Mac and apps can access.", vec![]),
        ],
    ]
}

fn main() {
    rmac_ui::boot_unified_with_assets(CombinedAssets, 1000.0, 720.0, |window, cx| {
        Settings::new(window, cx)
    });
}
