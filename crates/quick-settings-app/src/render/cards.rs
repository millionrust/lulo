//! Control Center pills, circles and Now Playing.

use gpui::ClickEvent;
use rmac_quick_settings::accessibility::CHANGING_LABEL;
use rmac_quick_settings::detail::{Detail, Module};
use rmac_quick_settings::{Command, FocusValue, PowerValue, Tile};

use super::*;

/// One icon glyph: asset path and its measured width and height.
type Glyph = (&'static str, f32, f32);

const WIFI: Glyph = ("cc/wifi.svg", 20.0, 14.5);
const BLUETOOTH: Glyph = ("cc/bluetooth.svg", 11.0, 17.5);
const MOON: Glyph = ("cc/moon.svg", 17.0, 17.0);
const SCREENSHOT: Glyph = ("cc/screenshot.svg", 22.0, 20.0);
const LOW_POWER: Glyph = ("cc/low-power.svg", 24.0, 12.0);

/// The 36 pt icon circle sits 14 in from the pill's rounded end.
const BADGE: f32 = 36.0;
const BADGE_INSET: f32 = 14.0;

/// What a pill shows.
struct Pill {
    id: &'static str,
    glyph: Glyph,
    on: bool,
    /// Glyph colour inside a white "on" circle.
    on_glyph: Hsla,
    enabled: bool,
    title: &'static str,
    /// `None` renders the single-line style Focus uses.
    subtitle: Option<String>,
    /// System Settings pane the label opens, unless it opens a list.
    pane: &'static str,
    /// The list the label opens in place of the grid (Wi-Fi, Bluetooth).
    detail: Option<Detail>,
    module: Module,
}

impl QuickSettingsView {
    fn subtitle<T>(&self, tile: &Tile<T>) -> String {
        if !self.received_snapshot {
            String::new()
        } else if tile.busy {
            CHANGING_LABEL.to_owned()
        } else {
            tile.summary.clone()
        }
    }

    fn pill(
        &self,
        x: f32,
        y: f32,
        pill: Pill,
        toggle: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (path, width, height) = pill.glyph;
        let centre = BADGE_INSET + BADGE / 2.0;
        let pane = pill.pane;
        let detail = pill.detail;
        let ring = self.ring(pill.module);
        let badge = div()
            .id(SharedString::from(format!("{}-toggle", pill.id)))
            .absolute()
            .left(px(BADGE_INSET))
            .top(px(BADGE_INSET))
            .size(px(BADGE))
            .rounded_full()
            .bg(if pill.on { mac::white() } else { circle_off() })
            .when(pill.enabled, |badge| {
                badge.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| toggle(this, cx)))
            })
            .child(glyph_at(
                path,
                BADGE / 2.0,
                BADGE / 2.0,
                width,
                height,
                if pill.on { pill.on_glyph } else { mac::white() },
            ));
        let label = div()
            .id(SharedString::from(format!("{}-label", pill.id)))
            .absolute()
            .left(px(centre + BADGE / 2.0))
            .top_0()
            .w(px(PILL_WIDTH - centre - BADGE / 2.0))
            .h(px(CELL))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| match detail {
                    Some(detail) => this.open_detail(detail, cx),
                    None => this.open_settings(Some(pane), window, cx),
                }),
            );
        let text = match pill.subtitle {
            Some(subtitle) => layer()
                .child(text_at(
                    56.5,
                    28.5,
                    13.0,
                    mac::SEMIBOLD,
                    title_text(),
                    pill.title,
                ))
                .child(text_at(
                    56.5,
                    43.5,
                    11.0,
                    mac::REGULAR,
                    subtitle_text(),
                    subtitle,
                )),
            None => layer().child(text_at(
                57.0,
                36.0,
                12.0,
                mac::SEMIBOLD,
                title_text(),
                pill.title,
            )),
        };
        module(x, y, PILL_WIDTH, CELL, CELL / 2.0)
            .overflow_hidden()
            .when(ring, |module| module.shadow(mac::focus_ring_shadow()))
            .when(!pill.enabled && self.received_snapshot, |module| {
                module.opacity(0.5)
            })
            .child(text)
            .child(label)
            .child(badge)
            .into_any_element()
    }

    pub(super) fn pill_wifi(
        &self,
        x: f32,
        y: f32,
        tile: &Tile<bool>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let next = !tile.value;
        self.pill(
            x,
            y,
            Pill {
                id: "wifi",
                glyph: WIFI,
                on: tile.available && tile.value,
                on_glyph: glyph_on(),
                enabled: tile.available && !tile.busy,
                title: "Wi-Fi",
                subtitle: Some(self.subtitle(tile)),
                pane: "wifi",
                detail: Some(Detail::Wifi),
                module: Module::Wifi,
            },
            move |this, cx| this.execute(Command::SetWifiEnabled(next), cx),
            cx,
        )
    }

    pub(super) fn pill_bluetooth(
        &self,
        x: f32,
        y: f32,
        tile: &Tile<bool>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let next = !tile.value;
        self.pill(
            x,
            y,
            Pill {
                id: "bluetooth",
                glyph: BLUETOOTH,
                on: tile.available && tile.value,
                on_glyph: glyph_on(),
                enabled: tile.available && !tile.busy,
                title: "Bluetooth",
                subtitle: Some(self.subtitle(tile)),
                pane: "bluetooth",
                detail: Some(Detail::Bluetooth),
                module: Module::Bluetooth,
            },
            move |this, cx| this.execute(Command::SetBluetoothPowered(next), cx),
            cx,
        )
    }

    pub(super) fn pill_focus(
        &self,
        x: f32,
        y: f32,
        tile: &Tile<FocusValue>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let next = !tile.value.enabled;
        self.pill(
            x,
            y,
            Pill {
                id: "focus",
                glyph: MOON,
                on: tile.available && tile.value.enabled,
                on_glyph: mac::system_indigo(),
                enabled: tile.available && !tile.busy,
                title: "Focus",
                subtitle: None,
                pane: "focus",
                detail: None,
                module: Module::Focus,
            },
            move |this, cx| this.execute(Command::SetFocusEnabled(next), cx),
            cx,
        )
    }

    /// A 64 pt circle module with one glyph. `on` fills it white, the way
    /// macOS draws an active circle control.
    #[allow(clippy::too_many_arguments)]
    fn circle(
        &self,
        id: &'static str,
        module_kind: Module,
        x: f32,
        y: f32,
        glyph: Glyph,
        on: bool,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let (path, width, height) = glyph;
        let ring = self.ring(module_kind);
        module(x, y, CELL, CELL, CELL / 2.0)
            .when(ring, |circle| circle.shadow(mac::focus_ring_shadow()))
            .when(on, |circle| circle.bg(mac::white()))
            .child(
                layer()
                    .id(id)
                    .rounded_full()
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| {
                            action(this, window, cx)
                        }),
                    )
                    .child(glyph_at(
                        path,
                        CELL / 2.0,
                        CELL / 2.0,
                        width,
                        height,
                        if on { mac::black() } else { mac::white() },
                    )),
            )
            .into_any_element()
    }

    /// Low Power Mode, backed by power-profiles-daemon.
    pub(super) fn low_power_circle(
        &self,
        x: f32,
        y: f32,
        tile: &Tile<PowerValue>,
        cx: &Context<Self>,
    ) -> AnyElement {
        self.circle(
            "low-power",
            Module::LowPower,
            x,
            y,
            LOW_POWER,
            layout::low_power_enabled(&tile.value),
            |this, _, cx| this.toggle_low_power(cx),
            cx,
        )
    }

    pub(super) fn screenshot_circle(&self, x: f32, y: f32, cx: &Context<Self>) -> AnyElement {
        self.circle(
            "screenshot",
            Module::Screenshot,
            x,
            y,
            SCREENSHOT,
            false,
            |this, window, cx| this.screenshot(window, cx),
            cx,
        )
    }

    /// Now Playing for the active MPRIS player: 140 × 140, two rows tall.
    pub(super) fn now_playing(&self, x: f32, y: f32, cx: &Context<Self>) -> AnyElement {
        let Some(player) = self.player.as_ref() else {
            return div().into_any_element();
        };
        let title = player
            .title
            .clone()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| player.identity.clone());
        let artist = player.artist.clone().filter(|artist| !artist.is_empty());
        let playing = player.status == rmac_media::PlaybackStatus::Playing;
        let text_width = PILL_WIDTH - 15.0 - 13.0;
        let text = match artist {
            Some(artist) => layer()
                .child(
                    text_at(15.0, 81.0, 13.0, mac::SEMIBOLD, title_text(), title)
                        .w(px(text_width))
                        .truncate(),
                )
                .child(
                    text_at(15.0, 95.0, 11.0, mac::REGULAR, subtitle_text(), artist)
                        .w(px(text_width))
                        .truncate(),
                ),
            None => layer().child(
                text_at(15.0, 85.5, 13.0, mac::SEMIBOLD, title_text(), title)
                    .w(px(text_width))
                    .truncate(),
            ),
        };
        let transport = [
            (
                "previous",
                ("cc/backward.svg", 22.0, 12.5),
                34.5,
                rmac_media::Command::Previous,
            ),
            (
                "play-pause",
                if playing {
                    ("cc/pause.svg", 17.5, 17.0)
                } else {
                    ("cc/play.svg", 17.5, 17.0)
                },
                70.5,
                rmac_media::Command::PlayPause,
            ),
            (
                "next",
                ("cc/forward.svg", 22.0, 12.5),
                106.5,
                rmac_media::Command::Next,
            ),
        ]
        .into_iter()
        .map(|(id, (path, width, height), centre, command)| {
            let supported = command.supported_by(player);
            div()
                .id(id)
                .absolute()
                .left(px(centre - 18.0))
                .top(px(113.0 - 15.0))
                .w(px(36.0))
                .h(px(30.0))
                .when(supported, |button| {
                    button.on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.media(command, cx)),
                    )
                })
                .child(glyph_at(
                    path,
                    18.0,
                    15.0,
                    width,
                    height,
                    if supported { mac::white() } else { dim_glyph() },
                ))
                .into_any_element()
        })
        .collect::<Vec<_>>();
        module(x, y, PILL_WIDTH, PITCH + CELL, layout::MODULE_RADIUS as f32)
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(14.0))
                    .top(px(14.0))
                    .size(px(40.0))
                    .rounded(px(8.0))
                    .bg(artwork_fill()),
            )
            .child(text)
            .children(transport)
            .into_any_element()
    }
}
