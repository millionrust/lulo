//! Weather ▸ Settings…: the saved temperature-unit preference.

use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowHandle,
};
use rmac_ui::{Button, Root, StyledExt as _};

use crate::view::WeatherView;

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 300.0;

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
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::WEATHER, WIDTH, HEIGHT, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("Settings");
        let view = cx.new(|cx| SettingsView::new(main, cx));
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
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Notifications => "Notifications",
        }
    }
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    main: Entity<WeatherView>,
}

impl SettingsView {
    fn new(main: Entity<WeatherView>, cx: &mut Context<Self>) -> Self {
        cx.observe(&main, |_, _, cx| cx.notify()).detach();
        Self {
            focus: cx.focus_handle(),
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
        let preference = self.main.read(cx).unit_preference();
        div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Temperature Unit"),
            )
            .child(
                div().flex().gap_2().children(
                    [
                        ("Automatic", None),
                        ("Celsius", Some(false)),
                        ("Fahrenheit", Some(true)),
                    ]
                    .into_iter()
                    .map(|(label, value)| {
                        Button::new(format!("weather-unit-{label}"), label)
                            .small()
                            .selected(preference == value)
                            .on_click(cx.listener(move |this, _, _, cx| this.set_unit(value, cx)))
                    }),
                ),
            )
            .child(
                div()
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Automatic follows the system's temperature unit."),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::General => self.render_general(cx).into_any_element(),
            Tab::Notifications => div()
                .text_color(rmac_ui::mac::text_secondary())
                .child("Weather alerts are not available for this forecast source.")
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus)
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
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .children([Tab::General, Tab::Notifications].into_iter().map(|tab| {
                        Button::new(format!("weather-settings-{}", tab.label()), tab.label())
                            .small()
                            .selected(self.tab == tab)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = tab;
                                cx.notify();
                            }))
                    })),
            )
            .child(div().flex_1().px_4().py_4().child(body))
    }
}
