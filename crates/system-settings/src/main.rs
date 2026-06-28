//! rmac System Settings — matched to macOS System Settings (Ventura+).
//!
//! Sidebar (search · Apple Account card · colored category tiles) + detail pane
//! (hero icon/title/description + grouped rounded cards of rows). Several panes
//! are interactive: General/Appearance/Sound/Wi-Fi/Bluetooth carry real controls
//! (Switch toggles, Sliders, segmented pickers) whose state is held in the view.
//! Row chevrons push detail subpages with a back stack (toolbar back button +
//! ⌘[). Where it is safe and read-only, panes reflect real macOS state
//! (appearance, computer name, macOS version, chip, memory).

use std::borrow::Cow;
use std::process::Command;

use gpui::{
    actions, div, prelude::FluentBuilder as _, px, svg, AnyElement, AppContext as _, AssetSource,
    Context, Div, ElementId, Entity, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, MouseButton, ParentElement, Render, Result, SharedString,
    StatefulInteractiveElement as _, Stateful, Styled, Svg, Window,
};
use gpui_component::slider::{Slider, SliderState};
use gpui_component::switch::Switch;
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

actions!(system_settings, [GoBack]);

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

// ---- interactive state enums --------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Appearance {
    Light,
    Dark,
    Auto,
}

/// A navigation subpage pushed onto the back stack from a row chevron.
#[derive(Clone)]
enum SubPage {
    About,
    SoftwareUpdate,
    Storage,
    /// A generic placeholder detail page identified by its row label.
    Placeholder { icon: &'static str, color: Hsla, title: SharedString },
}

/// Real, read-only macOS facts gathered once at launch.
struct SysInfo {
    computer_name: String,
    os: String,
    chip: String,
    memory: String,
    model: String,
}

// ---- a single Wi-Fi network (mock backend) ------------------------------

#[derive(Clone)]
struct WifiNetwork {
    name: SharedString,
    secure: bool,
    strength: u8, // 1..=3
}

struct Settings {
    account: SharedString,
    sysinfo: SysInfo,
    sections: Vec<Vec<Category>>,
    selected: (usize, usize),
    nav: Vec<SubPage>,
    search: Entity<gpui_component::input::InputState>,
    focus: FocusHandle,
    focused_once: bool,
    dragging: bool,

    // Wi-Fi
    wifi_on: bool,
    ask_to_join: bool,
    joined: Option<usize>,
    networks: Vec<WifiNetwork>,

    // Bluetooth
    bluetooth_on: bool,
    bt_discoverable: bool,

    // Appearance
    appearance: Appearance,
    accent_idx: usize,
    show_color_in_menu: bool,
    large_sidebar: bool,

    // Sound
    output_volume: Entity<SliderState>,
    alert_volume: Entity<SliderState>,
    balance: Entity<SliderState>,
    mute: bool,
    play_on_startup: bool,
    play_ui_sounds: bool,
    alert_idx: usize,

    // General
    handoff: bool,
    airdrop_idx: usize,
    airplay_receiver: bool,
}

const ACCENTS: &[(&str, u32)] = &[
    ("Blue", 0x0a84ff),
    ("Purple", 0xaf52de),
    ("Pink", 0xff2d55),
    ("Red", 0xff3b30),
    ("Orange", 0xff9500),
    ("Yellow", 0xffcc00),
    ("Green", 0x34c759),
    ("Graphite", 0x8e8e93),
];

const ALERT_SOUNDS: &[&str] = &["Boop", "Breeze", "Bubble", "Crystal", "Funk", "Heroine", "Submarine"];

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("Search")
        });
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        let mk_slider = |cx: &mut Context<Self>, val: f32| {
            let s = cx.new(|_| SliderState::new().min(0.0).max(100.0).step(1.0).default_value(val));
            cx.observe(&s, |_, _, cx| cx.notify()).detach();
            s
        };
        let output_volume = mk_slider(cx, 72.0);
        let alert_volume = mk_slider(cx, 55.0);
        let balance = mk_slider(cx, 50.0);

        Self {
            account: account_name().into(),
            sysinfo: gather_sysinfo(),
            sections: categories(),
            selected: (1, 0), // General
            nav: Vec::new(),
            search,
            focus: cx.focus_handle(),
            focused_once: false,
            dragging: false,

            wifi_on: true,
            ask_to_join: true,
            joined: Some(0),
            networks: vec![
                WifiNetwork { name: "lmes-5G".into(), secure: true, strength: 3 },
                WifiNetwork { name: "lmes-guest".into(), secure: false, strength: 2 },
                WifiNetwork { name: "Studio".into(), secure: true, strength: 2 },
                WifiNetwork { name: "CoffeeHouse".into(), secure: false, strength: 1 },
            ],

            bluetooth_on: true,
            bt_discoverable: true,

            appearance: if appearance_is_dark() { Appearance::Dark } else { Appearance::Light },
            accent_idx: 0,
            show_color_in_menu: true,
            large_sidebar: false,

            output_volume,
            alert_volume,
            balance,
            mute: false,
            play_on_startup: true,
            play_ui_sounds: true,
            alert_idx: 0,

            handoff: true,
            airdrop_idx: 1,
            airplay_receiver: false,
        }
    }

    fn current(&self) -> &Category {
        &self.sections[self.selected.0][self.selected.1]
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        self.nav.pop();
        cx.notify();
    }

    fn push(&mut self, sub: SubPage, cx: &mut Context<Self>) {
        self.nav.push(sub);
        cx.notify();
    }

    // ---- chrome -------------------------------------------------------

    fn render_topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_back = !self.nav.is_empty();
        let back = div()
            .id("nav-back")
            .flex()
            .items_center()
            .justify_center()
            .w(px(26.0))
            .h(px(26.0))
            .rounded(px(6.0))
            .when(can_back, |el: Stateful<Div>| {
                el.hover(|h| h.bg(hsl(0x00000010)))
                    .cursor_pointer()
                    .on_click(cx.listener(|t, _, _, cx| t.go_back(cx)))
            })
            .child(glyph(
                "icons/chevron-left.svg",
                17.0,
                if can_back { accent() } else { hsl(0xc4c4c8) },
            ));

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
                    .pl_3()
                    .gap_1()
                    .child(back)
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
            .child(
                div()
                    .flex_1()
                    .child(gpui_component::input::Input::new(&self.search).appearance(false)),
            );
        let q = self.search.read(cx).value().to_lowercase();

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

        let mut first_section = true;
        for (si, section) in self.sections.iter().enumerate() {
            let matching: Vec<(usize, &Category)> = section
                .iter()
                .enumerate()
                .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q))
                .collect();
            if matching.is_empty() {
                continue;
            }
            if !first_section {
                col = col.child(div().h(px(14.0)));
            }
            first_section = false;
            for (ci, cat) in matching {
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
                            t.nav.clear();
                            cx.notify();
                        })),
                );
            }
        }
        col
    }

    // ---- detail dispatch ---------------------------------------------

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content: Div = if let Some(sub) = self.nav.last().cloned() {
            self.render_subpage(&sub, cx)
        } else {
            match self.current().name.as_ref() {
                "Wi-Fi" => self.render_wifi(cx),
                "Bluetooth" => self.render_bluetooth(cx),
                "General" => self.render_general(cx),
                "Appearance" => self.render_appearance(cx),
                "Sound" => self.render_sound(cx),
                _ => self.render_generic(cx),
            }
        };

        div()
            .id("detail-scroll")
            .flex_1()
            .h_full()
            .bg(pane_bg())
            .overflow_y_scroll()
            .child(div().max_w(px(560.0)).mx_auto().px_5().pb_8().child(content))
    }

    fn render_hero(&self) -> Div {
        let cat = self.current();
        div()
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
            )
    }

    fn pane(&self, cards: Vec<Div>) -> Div {
        div().v_flex().child(self.render_hero()).children(cards)
    }

    // ---- generic (read-only mockup) panes ----------------------------

    fn render_generic(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let cat = self.current();
        let cards: Vec<Div> = cat
            .cards
            .iter()
            .map(|rows| {
                let rows: Vec<AnyElement> = rows
                    .iter()
                    .map(|r| {
                        nav_row(
                            view.clone(),
                            r.icon,
                            r.color,
                            r.label.clone(),
                            None,
                            SubPage::Placeholder {
                                icon: r.icon,
                                color: r.color,
                                title: r.label.clone(),
                            },
                        )
                    })
                    .collect();
                card(rows)
            })
            .collect();
        self.pane(cards)
    }

    // ---- Wi-Fi --------------------------------------------------------

    fn render_wifi(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let on = self.wifi_on;

        let toggle = card(vec![switch_row(
            "icons/wifi.svg",
            accent(),
            "Wi-Fi".into(),
            None,
            self.wifi_on,
            cx,
            |s, v| s.wifi_on = v,
        )]);

        let mut cards = vec![toggle];

        if on {
            // Known / current network
            if let Some(j) = self.joined {
                let net = &self.networks[j];
                cards.push(card(vec![value_row(
                    "icons/wifi.svg",
                    accent(),
                    net.name.clone(),
                    "Connected".into(),
                )]));
            }

            // Other networks
            let other: Vec<AnyElement> = self
                .networks
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != self.joined)
                .map(|(i, net)| network_row(view.clone(), i, net.clone()))
                .collect();
            cards.push(section_header("Other Networks"));
            cards.push(card(other));

            cards.push(card(vec![switch_row(
                "icons/wifi.svg",
                secondary(),
                "Ask to join networks".into(),
                Some("Known networks are joined automatically.".into()),
                self.ask_to_join,
                cx,
                |s, v| s.ask_to_join = v,
            )]));
        }

        self.pane(cards)
    }

    // ---- Bluetooth ----------------------------------------------------

    fn render_bluetooth(&self, cx: &Context<Self>) -> Div {
        let on = self.bluetooth_on;
        let toggle = card(vec![switch_row(
            "icons/bluetooth.svg",
            accent(),
            "Bluetooth".into(),
            None,
            self.bluetooth_on,
            cx,
            |s, v| s.bluetooth_on = v,
        )]);
        let mut cards = vec![toggle];
        if on {
            cards.push(card(vec![switch_row(
                "icons/bluetooth.svg",
                secondary(),
                "Discoverable".into(),
                Some("This Mac can be discovered by nearby devices.".into()),
                self.bt_discoverable,
                cx,
                |s, v| s.bt_discoverable = v,
            )]));
            cards.push(section_header("My Devices"));
            cards.push(card(vec![
                value_row("icons/volume-2.svg", secondary(), "Studio Display Speakers".into(), "Connected".into()),
                value_row("icons/bluetooth.svg", secondary(), "Magic Keyboard".into(), "Connected".into()),
                value_row("icons/bluetooth.svg", secondary(), "Magic Trackpad".into(), "Not Connected".into()),
            ]));
        }
        self.pane(cards)
    }

    // ---- General ------------------------------------------------------

    fn render_general(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let cards = vec![
            card(vec![
                nav_row(view.clone(), "icons/info.svg", hsl(0x8e8e93), "About".into(),
                    Some(self.sysinfo.model.clone().into()), SubPage::About),
                nav_row(view.clone(), "icons/refresh-cw.svg", hsl(0x8e8e93), "Software Update".into(),
                    Some("Up to date".into()), SubPage::SoftwareUpdate),
                nav_row(view.clone(), "icons/database.svg", hsl(0x8e8e93), "Storage".into(),
                    None, SubPage::Storage),
            ]),
            card(vec![switch_row(
                "icons/folder-symlink.svg",
                accent(),
                "Allow Handoff between this Mac and your devices".into(),
                None,
                self.handoff,
                cx,
                |s, v| s.handoff = v,
            )]),
            {
                let mut c = div().v_flex().mb_3().rounded(px(10.0)).bg(card_bg()).border_1().border_color(sep());
                c = c.child(label_row("AirDrop", None));
                c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
                c = c.child(segmented(
                    view.clone(),
                    "airdrop-seg",
                    &["No One", "Contacts Only", "Everyone"],
                    self.airdrop_idx,
                    |s, i| s.airdrop_idx = i,
                ).p_3());
                c
            },
            card(vec![switch_row(
                "icons/app-window.svg",
                accent(),
                "AirPlay Receiver".into(),
                Some("Allow this Mac to receive AirPlay content.".into()),
                self.airplay_receiver,
                cx,
                |s, v| s.airplay_receiver = v,
            )]),
        ];
        self.pane(cards)
    }

    // ---- Appearance ---------------------------------------------------

    fn render_appearance(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();

        let appearance_card = {
            let opt = |id: &'static str, name: &'static str, ap: Appearance, swatch: Hsla| {
                let selected = self.appearance == ap;
                let v = view.clone();
                div()
                    .id(ElementId::from(id))
                    .v_flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(64.0))
                            .h(px(40.0))
                            .rounded(px(6.0))
                            .bg(swatch)
                            .border_2()
                            .border_color(if selected { accent() } else { sep() }),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if selected { accent() } else { label() })
                            .child(name),
                    )
                    .on_click(move |_, _, cx| {
                        v.update(cx, |s, cx| {
                            s.appearance = ap;
                            cx.notify();
                        });
                    })
            };
            div()
                .flex()
                .gap_5()
                .justify_center()
                .p_4()
                .rounded(px(10.0))
                .mb_3()
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(opt("ap-light", "Light", Appearance::Light, hsl(0xf5f5f7)))
                .child(opt("ap-dark", "Dark", Appearance::Dark, hsl(0x2c2c2e)))
                .child(opt("ap-auto", "Auto", Appearance::Auto, hsl(0x8e8e93)))
        };

        // Accent color swatches
        let accent_card = {
            let swatches: Vec<AnyElement> = ACCENTS
                .iter()
                .enumerate()
                .map(|(i, (_name, hex))| {
                    let selected = self.accent_idx == i;
                    let v = view.clone();
                    div()
                        .id(ElementId::from(SharedString::from(format!("accent-{i}"))))
                        .w(px(22.0))
                        .h(px(22.0))
                        .rounded_full()
                        .bg(hsl(*hex))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| el.border_2().border_color(white()).shadow_sm())
                        .when(selected, |el| {
                            el.child(glyph("icons/check.svg", 12.0, white()))
                        })
                        .on_click(move |_, _, cx| {
                            v.update(cx, |s, cx| {
                                s.accent_idx = i;
                                cx.notify();
                            });
                        })
                        .into_any_element()
                })
                .collect();
            div()
                .v_flex()
                .mb_3()
                .rounded(px(10.0))
                .bg(card_bg())
                .border_1()
                .border_color(sep())
                .child(label_row("Accent color", None))
                .child(div().h(px(1.0)).bg(sep()).mx_3())
                .child(div().flex().gap_2().items_center().flex_wrap().p_3().children(swatches))
        };

        let toggles = card(vec![
            switch_row(
                "icons/palette.svg",
                accent(),
                "Show color in menu bar".into(),
                None,
                self.show_color_in_menu,
                cx,
                |s, v| s.show_color_in_menu = v,
            ),
            switch_row(
                "icons/panel-top.svg",
                accent(),
                "Larger sidebar icons".into(),
                None,
                self.large_sidebar,
                cx,
                |s, v| s.large_sidebar = v,
            ),
        ]);

        let system_note = card(vec![value_row(
            "icons/info.svg",
            secondary(),
            "Current system appearance".into(),
            if appearance_is_dark() { "Dark".into() } else { "Light".into() },
        )]);

        self.pane(vec![appearance_card, accent_card, toggles, system_note])
    }

    // ---- Sound --------------------------------------------------------

    fn render_sound(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();

        let out = self.output_volume.read(cx).value().start().round() as i32;
        let alert = self.alert_volume.read(cx).value().start().round() as i32;
        let bal = self.balance.read(cx).value().start().round() as i32;

        let output_card = div()
            .v_flex()
            .mb_3()
            .rounded(px(10.0))
            .bg(card_bg())
            .border_1()
            .border_color(sep())
            .child(slider_row("Output volume", &self.output_volume, format!("{out}%").into()))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(slider_row("Balance", &self.balance, format!("{bal}").into()))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(switch_row(
                "icons/volume-2.svg",
                secondary(),
                "Mute".into(),
                None,
                self.mute,
                cx,
                |s, v| s.mute = v,
            ));

        let alert_card = div()
            .v_flex()
            .mb_3()
            .rounded(px(10.0))
            .bg(card_bg())
            .border_1()
            .border_color(sep())
            .child(label_row("Alert sound", Some(ALERT_SOUNDS[self.alert_idx].into())))
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(
                segmented_dynamic(
                    view.clone(),
                    "alert-seg",
                    ALERT_SOUNDS,
                    self.alert_idx,
                    |s, i| s.alert_idx = i,
                )
                .p_3(),
            )
            .child(div().h(px(1.0)).bg(sep()).mx_3())
            .child(slider_row("Alert volume", &self.alert_volume, format!("{alert}%").into()));

        let toggles = card(vec![
            switch_row(
                "icons/power.svg",
                secondary(),
                "Play sound on startup".into(),
                None,
                self.play_on_startup,
                cx,
                |s, v| s.play_on_startup = v,
            ),
            switch_row(
                "icons/bell.svg",
                secondary(),
                "Play user interface sound effects".into(),
                None,
                self.play_ui_sounds,
                cx,
                |s, v| s.play_ui_sounds = v,
            ),
        ]);

        self.pane(vec![output_card, alert_card, toggles])
    }

    // ---- subpages -----------------------------------------------------

    fn render_subpage(&self, sub: &SubPage, _cx: &Context<Self>) -> Div {
        let (title, body): (SharedString, Div) = match sub {
            SubPage::About => ("About".into(), self.about_body()),
            SubPage::SoftwareUpdate => (
                "Software Update".into(),
                card(vec![
                    value_row("icons/refresh-cw.svg", hsl(0x34c759), "macOS".into(),
                        format!("{} — up to date", self.sysinfo.os).into()),
                    value_row("icons/refresh-cw.svg", secondary(), "Automatic updates".into(), "On".into()),
                ]),
            ),
            SubPage::Storage => (
                "Storage".into(),
                card(vec![
                    value_row("icons/database.svg", accent(), "Macintosh HD".into(), "Available".into()),
                    value_row("icons/folder-symlink.svg", hsl(0xff9500), "Applications".into(), "—".into()),
                    value_row("icons/image.svg", hsl(0x30b0c7), "Photos".into(), "—".into()),
                    value_row("icons/database.svg", secondary(), "System Data".into(), "—".into()),
                ]),
            ),
            SubPage::Placeholder { icon, color, title } => (
                title.clone(),
                card(vec![value_row(*icon, *color, title.clone(), "Not configured".into())]),
            ),
        };

        let header = div()
            .v_flex()
            .items_center()
            .gap_1()
            .pt_6()
            .pb_4()
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(rmac_ui::mac::BOLD)
                    .text_color(label())
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(secondary())
                    .child("‹ Back, or press ⌘["),
            );

        div().v_flex().child(header).child(body)
    }

    fn about_body(&self) -> Div {
        let si = &self.sysinfo;
        card(vec![
            value_row("icons/info.svg", secondary(), "Name".into(), si.computer_name.clone().into()),
            value_row("icons/monitor.svg", secondary(), "Model".into(), si.model.clone().into()),
            value_row("icons/settings.svg", secondary(), "Chip".into(), si.chip.clone().into()),
            value_row("icons/database.svg", secondary(), "Memory".into(), si.memory.clone().into()),
            value_row("icons/refresh-cw.svg", secondary(), "macOS".into(), si.os.clone().into()),
        ])
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            window.focus(&self.focus);
        }
        div()
            .size_full()
            .v_flex()
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .on_action(cx.listener(|t, _: &GoBack, _, cx| t.go_back(cx)))
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

// ---- row / control builders ----------------------------------------------

fn row_base() -> Div {
    div().flex().items_center().gap_3().min_h(px(44.0)).px_3().py_2()
}

fn text_block(title: SharedString, sub: Option<SharedString>) -> Div {
    let mut b = div().v_flex().flex_1().child(
        div().text_size(px(13.0)).text_color(label()).child(title),
    );
    if let Some(s) = sub {
        b = b.child(div().text_size(px(11.0)).text_color(secondary()).child(s));
    }
    b
}

/// A plain card-section label row (no control).
fn label_row(title: &'static str, value: Option<SharedString>) -> Div {
    let mut r = row_base().child(
        div().flex_1().text_size(px(13.0)).text_color(label()).child(title),
    );
    if let Some(v) = value {
        r = r.child(div().text_size(px(13.0)).text_color(secondary()).child(v));
    }
    r
}

/// A read-only row with a right-aligned value.
fn value_row(icon: &'static str, color: Hsla, title: SharedString, value: SharedString) -> AnyElement {
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None))
        .child(div().text_size(px(13.0)).text_color(secondary()).child(value))
        .into_any_element()
}

/// A section header above a card (gray small caps-ish title).
fn section_header(title: &'static str) -> Div {
    div()
        .px_1()
        .pt_2()
        .pb_1()
        .text_size(px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .text_color(secondary())
        .child(title)
}

/// A switch row whose state lives in the view; `set` writes the new bool.
#[allow(clippy::too_many_arguments)]
fn switch_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    sub: Option<SharedString>,
    checked: bool,
    cx: &Context<Settings>,
    set: fn(&mut Settings, bool),
) -> AnyElement {
    let view = cx.entity();
    let id = ElementId::from(SharedString::from(format!("sw-{title}")));
    let sw = Switch::new(id).checked(checked).on_click(move |v, _, cx| {
        let nv = *v;
        view.update(cx, |s, cx| {
            set(s, nv);
            cx.notify();
        });
    });
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, sub))
        .child(sw)
        .into_any_element()
}

/// A slider row (state held in its own SliderState entity).
fn slider_row(title: &'static str, state: &Entity<SliderState>, value: SharedString) -> Div {
    row_base()
        .child(div().w(px(110.0)).flex_none().text_size(px(13.0)).text_color(label()).child(title))
        .child(div().flex_1().child(Slider::new(state).w_full()))
        .child(div().w(px(44.0)).flex_none().text_right().text_size(px(12.0)).text_color(secondary()).child(value))
}

/// A clickable navigation row that pushes a subpage onto the back stack.
fn nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
    target: SubPage,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("nav-{title}")));
    let mut r = row_base()
        .id(id)
        .cursor_pointer()
        .hover(|h| h.bg(hsl(0x00000006)))
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None));
    if let Some(v) = value {
        r = r.child(div().text_size(px(13.0)).text_color(secondary()).child(v));
    }
    r.child(glyph("icons/chevron-right.svg", 14.0, hsl(0xc4c4c8)))
        .on_click(move |_, _, cx| {
            let target = target.clone();
            view.update(cx, |s, cx| s.push(target, cx));
        })
        .into_any_element()
}

/// A Wi-Fi network row: click to "join" it.
fn network_row(view: Entity<Settings>, idx: usize, net: WifiNetwork) -> AnyElement {
    let bars = "▂▄▆".chars().take(net.strength as usize).collect::<String>();
    row_base()
        .id(ElementId::from(SharedString::from(format!("net-{idx}"))))
        .cursor_pointer()
        .hover(|h| h.bg(hsl(0x00000006)))
        .child(tile("icons/wifi.svg", accent(), 22.0))
        .child(text_block(net.name.clone(), None))
        .when(net.secure, |r| r.child(glyph("icons/lock.svg", 13.0, secondary())))
        .child(div().text_size(px(13.0)).text_color(secondary()).child(bars))
        .on_click(move |_, _, cx| {
            view.update(cx, |s, cx| {
                s.joined = Some(idx);
                cx.notify();
            });
        })
        .into_any_element()
}

/// A segmented control over a fixed set of options; `set` writes the index.
fn segmented(
    view: Entity<Settings>,
    id: &'static str,
    options: &[&'static str],
    selected: usize,
    set: fn(&mut Settings, usize),
) -> Div {
    let mut row = div().flex().gap_1().w_full();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let v = view.clone();
        row = row.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{i}"))))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .h(px(26.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .when(is_sel, |el| el.bg(accent()).text_color(white()))
                .when(!is_sel, |el| el.bg(hsl(0xe9e9ec)).text_color(label()).hover(|h| h.bg(hsl(0xdedee2))))
                .child(*opt)
                .on_click(move |_, _, cx| {
                    v.update(cx, |s, cx| {
                        set(s, i);
                        cx.notify();
                    });
                }),
        );
    }
    row
}

/// Like [`segmented`] but for a runtime slice (e.g. alert sound names).
fn segmented_dynamic(
    view: Entity<Settings>,
    id: &'static str,
    options: &[&'static str],
    selected: usize,
    set: fn(&mut Settings, usize),
) -> Div {
    let mut row = div().flex().flex_wrap().gap_1().w_full();
    for (i, opt) in options.iter().enumerate() {
        let is_sel = i == selected;
        let v = view.clone();
        row = row.child(
            div()
                .id(ElementId::from(SharedString::from(format!("{id}-{i}"))))
                .flex()
                .items_center()
                .justify_center()
                .px_2()
                .h(px(26.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .when(is_sel, |el| el.bg(accent()).text_color(white()))
                .when(!is_sel, |el| el.bg(hsl(0xe9e9ec)).text_color(label()).hover(|h| h.bg(hsl(0xdedee2))))
                .child(*opt)
                .on_click(move |_, _, cx| {
                    v.update(cx, |s, cx| {
                        set(s, i);
                        cx.notify();
                    });
                }),
        );
    }
    row
}

/// Build a rounded white card from rows, inserting inset separators.
fn card(rows: Vec<AnyElement>) -> Div {
    let mut c = div()
        .v_flex()
        .mb_3()
        .rounded(px(10.0))
        .bg(card_bg())
        .border_1()
        .border_color(sep());
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        c = c.child(r);
        if i + 1 < n {
            c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
        }
    }
    c
}

// ---- real macOS reads (best-effort, read-only) ---------------------------

fn cmd(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn appearance_is_dark() -> bool {
    cmd("defaults", &["read", "-g", "AppleInterfaceStyle"])
        .map(|s| s.eq_ignore_ascii_case("Dark"))
        .unwrap_or(false)
}

fn account_name() -> String {
    cmd("id", &["-F"])
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "User".into())
}

fn gather_sysinfo() -> SysInfo {
    let computer_name = cmd("scutil", &["--get", "ComputerName"])
        .or_else(|| cmd("hostname", &[]))
        .unwrap_or_else(|| "Mac".into());

    let os = {
        let name = cmd("sw_vers", &["-productName"]).unwrap_or_else(|| "macOS".into());
        let ver = cmd("sw_vers", &["-productVersion"]).unwrap_or_default();
        format!("{name} {ver}").trim().to_string()
    };

    let chip = cmd("sysctl", &["-n", "machdep.cpu.brand_string"]).unwrap_or_else(|| "—".into());

    let memory = cmd("sysctl", &["-n", "hw.memsize"])
        .and_then(|s| s.parse::<u64>().ok())
        .map(|bytes| format!("{} GB", bytes / 1024 / 1024 / 1024))
        .unwrap_or_else(|| "—".into());

    let model = cmd("sysctl", &["-n", "hw.model"]).unwrap_or_else(|| "Mac".into());

    SysInfo { computer_name, os, chip, memory, model }
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
        cx.bind_keys([KeyBinding::new("cmd-[", GoBack, Some("SystemSettings"))]);
        Settings::new(window, cx)
    });
}
