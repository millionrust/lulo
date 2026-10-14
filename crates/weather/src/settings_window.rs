//! Weather ▸ Settings…: "Weather Settings", with the Mac's General and
//! Notifications panes (`settings/General`, 2026-10-13 robot capture,
//! 488 pt wide): the Temperature radios, the Other Units pop-ups (wind,
//! precipitation, pressure, distance), Default Units ▸ Restore Defaults.
//!
//! The Mac's Locations ▸ "Home and Work" row is left out: it labels places
//! from the person's Contacts card, which Lulo has no store for.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, Entity, FocusHandle,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_ui::{Button, PopUpButton, PopupMenuItem, Radio, Root, StyledExt as _};
use rmac_weather::summary::Unit;
use rmac_weather::units::{DistanceUnit, PrecipitationUnit, PressureUnit, WindUnit};

use crate::{view::WeatherView, CloseWindow, ShowSettings};

/// The Mac's panel width; the height is the Mac's 661 pt less the
/// Locations section Lulo leaves out.
const WIDTH: f32 = 488.0;
const HEIGHT: f32 = 571.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn show(main: Entity<WeatherView>, cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    // Not `window_options_for_app`: Settings would then inherit whatever
    // size the main Weather window last saved under the same app_id (the
    // UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::WEATHER, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Weather Settings");
        let view = cx.new(|cx| SettingsView::new(main, window, cx));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-weather: could not open Settings: {error}"),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Tab {
    General,
    Notifications,
}

impl Tab {
    const ALL: [Self; 2] = [Self::General, Self::Notifications];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Notifications => "Notifications",
        }
    }

    /// The icon toolbar's glyph: a gear and a bell, drawn as plain glyphs
    /// the way Finder Settings' toolbar does.
    fn glyph(self) -> &'static str {
        const GLYPH_GENERAL: &str = "⚙";
        const GLYPH_NOTIFICATIONS: &str = "🔔";
        match self {
            Self::General => GLYPH_GENERAL,
            Self::Notifications => GLYPH_NOTIFICATIONS,
        }
    }
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    main: Entity<WeatherView>,
}

/// One Other Units row: the label on the left and its pop-up at the Mac's
/// x 362, 96 pt wide, 45 pt apart.
fn unit_row<T: Copy + PartialEq + 'static>(
    id: &'static str,
    label: &'static str,
    current: T,
    options: &'static [T],
    name: fn(T) -> &'static str,
    choose: impl Fn(T, &mut App) + Clone + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(label)
        .h(px(45.0))
        .flex()
        .items_center()
        .justify_between()
        .child(div().text_size(rmac_ui::text_px(13.0)).child(label))
        .child(
            div()
                .w(px(96.0))
                .child(PopUpButton::new(id, name(current)).w_full().dropdown_menu(
                    move |menu, _, _| {
                        options.iter().fold(menu, |menu, &option| {
                            let choose = choose.clone();
                            menu.item(
                                PopupMenuItem::new(name(option))
                                    .checked(option == current)
                                    .on_click(move |_, _, cx| choose(option, cx)),
                            )
                        })
                    },
                )),
        )
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .text_size(rmac_ui::text_px(13.0))
        .font_weight(FontWeight::BOLD)
        .child(text)
}

impl SettingsView {
    fn new(main: Entity<WeatherView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `register_menu_target` below already tracks this as a key-window
        // candidate; `observe_window_state` would additionally persist its
        // geometry under `app_id::WEATHER`, the same key the main window
        // uses, letting Settings' own fixed size overwrite the main
        // window's restored one on close (the UIA-06/UIA-09 bug).
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        rmac_ui::register_menu_target(window, &focus, cx);
        Self {
            focus,
            tab: Tab::General,
            main,
        }
    }

    fn set_unit(&mut self, preference: Option<bool>, cx: &mut Context<Self>) {
        self.main.update(cx, |weather, cx| {
            weather.set_unit_preference(preference, cx);
        });
        cx.notify();
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weather = self.main.read(cx);
        let preference = weather.unit_preference();
        let choices = weather.unit_choices();
        let shown = choices.resolve(weather.region_units());
        let at_defaults = preference.is_none() && choices.is_default();
        let system = match Unit::from_environment() {
            Unit::Celsius => "Use system setting (°C)",
            Unit::Fahrenheit => "Use system setting (°F)",
        };
        let main = self.main.clone();
        div()
            .v_flex()
            .child(heading("Temperature"))
            .child(
                div().pt(px(6.0)).v_flex().children(
                    [
                        ("Celsius (°C)", Some(false)),
                        ("Fahrenheit (°F)", Some(true)),
                        (system, None),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, value))| {
                        div().h(px(26.0)).flex().items_center().child(
                            Radio::new(("weather-temperature", index))
                                .label(label)
                                .selected(preference == value)
                                .on_change(
                                    cx.listener(move |this, _, _, cx| this.set_unit(value, cx)),
                                ),
                        )
                    }),
                ),
            )
            .child(div().pt(px(35.0)).child(heading("Other Units")))
            .child(
                div()
                    .pt(px(8.0))
                    .v_flex()
                    .child(unit_row(
                        "weather-units-wind",
                        "Wind",
                        shown.wind,
                        &WindUnit::ALL,
                        WindUnit::label,
                        {
                            let main = main.clone();
                            move |unit, cx: &mut App| {
                                main.update(cx, |weather, cx| {
                                    let mut choices = weather.unit_choices();
                                    choices.wind = Some(unit);
                                    weather.set_unit_choices(choices, cx);
                                })
                            }
                        },
                    ))
                    .child(unit_row(
                        "weather-units-precipitation",
                        "Precipitation",
                        shown.precipitation,
                        &PrecipitationUnit::ALL,
                        PrecipitationUnit::label,
                        {
                            let main = main.clone();
                            move |unit, cx: &mut App| {
                                main.update(cx, |weather, cx| {
                                    let mut choices = weather.unit_choices();
                                    choices.precipitation = Some(unit);
                                    weather.set_unit_choices(choices, cx);
                                })
                            }
                        },
                    ))
                    .child(unit_row(
                        "weather-units-pressure",
                        "Pressure",
                        shown.pressure,
                        &PressureUnit::ALL,
                        PressureUnit::label,
                        {
                            let main = main.clone();
                            move |unit, cx: &mut App| {
                                main.update(cx, |weather, cx| {
                                    let mut choices = weather.unit_choices();
                                    choices.pressure = Some(unit);
                                    weather.set_unit_choices(choices, cx);
                                })
                            }
                        },
                    ))
                    .child(unit_row(
                        "weather-units-distance",
                        "Distance",
                        shown.distance,
                        &DistanceUnit::ALL,
                        DistanceUnit::label,
                        {
                            let main = main.clone();
                            move |unit, cx: &mut App| {
                                main.update(cx, |weather, cx| {
                                    let mut choices = weather.unit_choices();
                                    choices.distance = Some(unit);
                                    weather.set_unit_choices(choices, cx);
                                })
                            }
                        },
                    )),
            )
            .child(div().pt(px(17.0)).child(heading("Default Units")))
            .child(
                div()
                    .pt(px(1.0))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Set all weather units to the defaults for your region."),
            )
            .child(
                div().pt(px(10.0)).child(
                    Button::new("weather-restore-default-units", "Restore Defaults")
                        .small()
                        .disabled(at_defaults)
                        .on_click(move |_, _, cx| {
                            main.update(cx, |weather, cx| weather.restore_default_units(cx));
                        }),
                ),
            )
    }

    /// Lulo Weather sends no notifications (no severe-weather or daily
    /// forecast alerts from this forecast source), so the pane says so in
    /// the Mac's own "Notifications Are Off" layout, without the Mac's
    /// "Go to System Settings…" button, which would enable nothing here.
    fn render_notifications(&self) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(rmac_ui::text_px(17.0))
                    .font_weight(FontWeight::BOLD)
                    .child("Notifications Are Off"),
            )
            .child(
                div()
                    .max_w(px(422.0))
                    .text_center()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child(
                        "Weather does not send notifications: severe weather and daily \
                         forecast alerts are not available from this forecast source.",
                    ),
            )
    }

    /// General and Notifications as glyph-over-label tiles, the selected one
    /// lifted in the accent colour (the Mac's icon toolbar, 56 pt).
    fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .h(px(56.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(1.0))
            .border_b_1()
            .border_color(rmac_ui::mac::separator())
            .children(Tab::ALL.into_iter().map(|tab| {
                let selected = tab == self.tab;
                let tint = if selected {
                    rmac_ui::mac::accent()
                } else {
                    rmac_ui::mac::text_secondary()
                };
                div()
                    .id(SharedString::from(format!(
                        "weather-settings-{}",
                        tab.label()
                    )))
                    .role(Role::Tab)
                    .aria_label(tab.label())
                    .aria_selected(selected)
                    .min_w(px(55.0))
                    .px(px(6.0))
                    .h(px(48.0))
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .when(selected, |tile| tile.bg(rmac_ui::mac::raised()))
                    .when(!selected, |tile| {
                        tile.hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .text_color(tint)
                            .child(tab.glyph()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(tint)
                            .child(tab.label()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    }))
            }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::General => self.render_general(cx).into_any_element(),
            Tab::Notifications => self.render_notifications().into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .key_context("Weather")
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, _, cx| cx.quit()))
            .on_action(cx.listener(|_, _: &ShowSettings, window, _| window.activate_window()))
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .text_color(rmac_ui::mac::text())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Weather Settings"),
            ))
            .child(self.render_tab_strip(cx))
            .child(
                div()
                    .flex_1()
                    .px(px(30.0))
                    .pt(px(16.0))
                    .pb(px(16.0))
                    .child(body),
            )
    }
}
