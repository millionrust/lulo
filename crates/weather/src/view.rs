//! The Weather window: a sidebar of saved cities and the selected city's
//! conditions, hourly strip, ten-day list and details.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rgb, rgba, svg,
    AnyElement, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowControlArea,
};
use rmac_ui::{mac, InputEvent, InputState, SearchField};
use rmac_weather::fetch::{self, FetchError};
use rmac_weather::forecast::{self, Forecast, Sky};
use rmac_weather::geocode::{self, Place};
use rmac_weather::metrics as m;
use rmac_weather::store::{self, Cached, Settings};
use rmac_weather::summary::{self, Column, Unit};

use crate::{CloseWindow, FindCity, Refresh, UseCelsius, UseFahrenheit};

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
const REFRESH_EVERY: Duration = Duration::from_secs(15 * 60);
const CLOCK_TICK: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Loading,
    Ready,
    Offline,
    Failed,
}

struct Loaded {
    forecast: Forecast,
    fetched_at: i64,
}

pub(crate) struct WeatherView {
    pub(crate) focus: FocusHandle,
    settings: Settings,
    unit: Unit,
    loaded: HashMap<String, Loaded>,
    status: HashMap<String, Status>,
    search: Entity<InputState>,
    results: Vec<Place>,
    search_generation: u64,
    search_failed: bool,
    settings_error: Option<SharedString>,
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

impl WeatherView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search for a city")
                .clean_on_escape()
        });
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => this.schedule_search(cx),
            InputEvent::PressEnter { .. } => {
                if let Some(place) = this.results.first().cloned() {
                    this.add_place(place, cx);
                }
            }
            _ => {}
        })
        .detach();
        let (settings, settings_error) = match store::load_settings() {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some(SharedString::from(format!(
                    "Weather could not read its saved cities ({error})."
                ))),
            ),
        };
        let unit = match settings.fahrenheit {
            Some(true) => Unit::Fahrenheit,
            Some(false) => Unit::Celsius,
            None => Unit::from_environment(),
        };
        let mut view = Self {
            focus: cx.focus_handle(),
            settings,
            unit,
            loaded: HashMap::new(),
            status: HashMap::new(),
            search,
            results: Vec::new(),
            search_generation: 0,
            search_failed: false,
            settings_error,
        };
        for place in view.settings.places.clone() {
            view.load_cache(&place);
        }
        if view.settings.places.is_empty() {
            view.search.update(cx, |state, cx| state.focus(window, cx));
        }
        view.refresh_all(false, cx);
        view.start_timers(cx);
        view
    }

    fn start_timers(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(REFRESH_EVERY).await;
            if this
                .update(cx, |view, cx| view.refresh_all(false, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
        // Keep the cities' local times current.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(CLOCK_TICK).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
    }

    fn load_cache(&mut self, place: &Place) {
        let Some(cached) = store::cache_path(place).and_then(|path| store::read_cache_from(&path))
        else {
            return;
        };
        if let Ok(forecast) = Forecast::parse(cached.body.as_bytes()) {
            self.loaded.insert(
                place.key(),
                Loaded {
                    forecast,
                    fetched_at: cached.fetched_at,
                },
            );
        }
    }

    fn refresh_all(&mut self, force: bool, cx: &mut Context<Self>) {
        for place in self.settings.places.clone() {
            self.refresh(place, force, cx);
        }
    }

    fn refresh(&mut self, place: Place, force: bool, cx: &mut Context<Self>) {
        let key = place.key();
        let now = now_seconds();
        let fresh = self
            .loaded
            .get(&key)
            .is_some_and(|loaded| now - loaded.fetched_at < store::FRESH_SECONDS);
        if (fresh && !force) || self.status.get(&key) == Some(&Status::Loading) {
            return;
        }
        self.status.insert(key.clone(), Status::Loading);
        let task = cx.background_executor().spawn(async move {
            let body = fetch::get(&forecast::forecast_url(place.latitude, place.longitude))?;
            let forecast = Forecast::parse(&body).map_err(|_| FetchError::Service)?;
            let fetched_at = now_seconds();
            if let (Some(path), Ok(body)) = (store::cache_path(&place), String::from_utf8(body)) {
                let _ = store::write_cache_to(&path, &Cached { fetched_at, body });
            }
            Ok::<_, FetchError>((forecast, fetched_at))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                let status = match result {
                    Ok((forecast, fetched_at)) => {
                        view.loaded.insert(
                            key.clone(),
                            Loaded {
                                forecast,
                                fetched_at,
                            },
                        );
                        Status::Ready
                    }
                    Err(FetchError::Offline) => Status::Offline,
                    Err(_) => Status::Failed,
                };
                view.status.insert(key, status);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_generation += 1;
        let generation = self.search_generation;
        let query = self.search.read(cx).value().to_string();
        let Some(url) = geocode::search_url(&query) else {
            self.results.clear();
            self.search_failed = false;
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let current = this
                .update(cx, |view, _| view.search_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let task = cx.background_executor().spawn(async move {
                fetch::get(&url)
                    .ok()
                    .and_then(|body| geocode::parse_results(&body))
            });
            let results = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.search_generation != generation {
                    return;
                }
                view.search_failed = results.is_none();
                view.results = results.unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_generation += 1;
        self.results.clear();
        self.search_failed = false;
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
    }

    fn add_place(&mut self, place: Place, cx: &mut Context<Self>) {
        self.settings.add(place.clone());
        self.results.clear();
        self.search_generation += 1;
        self.load_cache(&place);
        self.save(cx);
        self.refresh(place, false, cx);
        cx.notify();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.settings.places.len() {
            self.settings.selected = index;
            self.save(cx);
            let place = self.settings.places[index].clone();
            self.refresh(place, false, cx);
            cx.notify();
        }
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        self.settings.remove(index);
        self.save(cx);
        cx.notify();
    }

    fn set_unit(&mut self, unit: Unit, cx: &mut Context<Self>) {
        self.unit = unit;
        self.settings.fahrenheit = Some(unit == Unit::Fahrenheit);
        self.save(cx);
        cx.notify();
    }

    fn save(&self, cx: &mut Context<Self>) {
        if self.settings_error.is_some() {
            return;
        }
        let settings = self.settings.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = store::save_settings(&settings) {
                    eprintln!("rmac-weather: could not save cities: {error}");
                }
            })
            .detach();
    }

    // ------------------------------------------------------------ pieces

    fn search_results(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let query = self.search.read(cx).value().to_string();
        if geocode::search_url(&query).is_none() {
            return None;
        }
        let rows = self
            .results
            .iter()
            .enumerate()
            .map(|(index, place)| {
                let chosen = place.clone();
                div()
                    .id(SharedString::from(format!("weather-result-{index}")))
                    .h(px(40.0))
                    .px(px(10.0))
                    .rounded(px(7.0))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .hover(|style| style.bg(mac::accent()))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(mac::text())
                            .child(place.name.clone()),
                    )
                    .when(!place.region.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(mac::text_secondary())
                                .child(place.region.clone()),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.clear_search(window, cx);
                        this.add_place(chosen.clone(), cx);
                    }))
            })
            .collect::<Vec<_>>();
        let message = if self.search_failed {
            Some("Search needs an internet connection.")
        } else if rows.is_empty() {
            Some("No Results")
        } else {
            None
        };
        Some(
            div()
                .id("weather-results")
                .w_full()
                .p(px(6.0))
                .rounded(px(12.0))
                .bg(mac::material_popover())
                .border_1()
                .border_color(mac::separator())
                .shadow_lg()
                .flex()
                .flex_col()
                .children(rows)
                .when_some(message, |panel, message| {
                    panel.child(
                        div()
                            .h(px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(mac::text_secondary())
                            .child(message),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_first_run(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (top, bottom) = summary::backdrop(Sky::Clear, false);
        div()
            .absolute()
            .size_full()
            .bg(linear_gradient(
                180.0,
                linear_color_stop(rgb(top), 0.0),
                linear_color_stop(rgb(bottom), 1.0),
            ))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(196.0))
            .child(
                svg()
                    .path("icons/weather/location.svg")
                    .size(px(44.0))
                    .text_color(mac::white()),
            )
            .child(
                div()
                    .mt(px(16.0))
                    .text_size(px(22.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::white())
                    .child("Choose a Location"),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .w(px(400.0))
                    .text_size(px(13.0))
                    .line_height(px(18.0))
                    .text_center()
                    .text_color(rgba(0xFFFF_FFCC))
                    .child(
                        "Weather shows the forecast for the cities you add. It never looks up \
                         where this computer is.",
                    ),
            )
            .child(
                div()
                    .mt(px(20.0))
                    .w(px(300.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(SearchField::new(&self.search))
                    .children(self.search_results(cx)),
            )
            .child(div().flex_1())
            .child(attribution(None))
    }

    fn render_sidebar(&self, now: i64, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self
            .settings
            .places
            .iter()
            .enumerate()
            .map(|(index, place)| {
                let loaded = self.loaded.get(&place.key());
                let selected = index == self.settings.selected;
                let (top, bottom) = loaded
                    .map(|loaded| {
                        summary::backdrop(loaded.forecast.current.sky, loaded.forecast.current.day)
                    })
                    .unwrap_or((0x3A3F4A, 0x4A505C));
                let time = loaded
                    .map(|loaded| summary::time_label(now, loaded.forecast.utc_offset, true))
                    .unwrap_or_default();
                let condition = loaded
                    .map(|loaded| {
                        loaded
                            .forecast
                            .current
                            .sky
                            .description(loaded.forecast.current.day)
                    })
                    .unwrap_or("");
                let temperature = loaded
                    .map(|loaded| self.unit.degrees(loaded.forecast.current.temperature))
                    .unwrap_or_else(|| "--".to_owned());
                let range = loaded
                    .and_then(|loaded| loaded.forecast.days.first())
                    .map(|day| {
                        format!(
                            "H:{} L:{}",
                            self.unit.degrees(day.high),
                            self.unit.degrees(day.low)
                        )
                    })
                    .unwrap_or_default();
                let small = |text: String| {
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgba(0xFFFF_FFD9))
                        .child(text)
                };
                div()
                    .id(SharedString::from(format!("weather-place-{index}")))
                    .group("place")
                    .relative()
                    .flex_none()
                    .h(px(m::CARD_HEIGHT))
                    .rounded(px(m::CARD_RADIUS))
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(rgb(top), 0.0),
                        linear_color_stop(rgb(bottom), 1.0),
                    ))
                    .when(selected, |card| {
                        card.border_2().border_color(rgba(0xFFFF_FF8C))
                    })
                    .child(
                        div()
                            .absolute()
                            .left(px(12.0))
                            .top(px(10.0))
                            .right(px(100.0))
                            .text_size(px(17.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(mac::white())
                            .truncate()
                            .child(place.name.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(12.0))
                            .top(px(32.0))
                            .child(small(time)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(12.0))
                            .bottom(px(10.0))
                            .child(small(condition.to_owned())),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(12.0))
                            .top(px(4.0))
                            .text_size(px(40.0))
                            .font_weight(FontWeight::LIGHT)
                            .text_color(mac::white())
                            .child(temperature),
                    )
                    .child(
                        div()
                            .absolute()
                            .right(px(12.0))
                            .bottom(px(10.0))
                            .child(small(range)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("weather-remove-{index}")))
                            .absolute()
                            .right(px(-4.0))
                            .top(px(-4.0))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(rgb(0x5A5A5A))
                            .opacity(0.0)
                            .group_hover("place", |style| style.opacity(1.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(mac::white())
                            .child("×")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.remove(index, cx);
                            })),
                    )
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.select(index, cx)),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .absolute()
            .top_0()
            .left_0()
            .bottom_0()
            .w(px(m::SIDEBAR_WIDTH))
            .bg(rgb(m::SIDEBAR_FILL))
            .child(
                div()
                    .id("weather-places")
                    .absolute()
                    .top(px(m::TOOLBAR_HEIGHT + m::SEARCH_HEIGHT + 16.0))
                    .left(px(m::CARD_INSET))
                    .right(px(m::CARD_INSET))
                    .bottom_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(m::CARD_GAP))
                    .pt(px(4.0))
                    .pb(px(12.0))
                    .children(cards),
            )
            .child(
                div()
                    .absolute()
                    .top(px(m::TOOLBAR_HEIGHT))
                    .left(px(m::SEARCH_INSET))
                    .right(px(m::SEARCH_INSET))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(SearchField::new(&self.search).small())
                    .children(self.search_results(cx)),
            )
    }

    fn render_main(&self, width: f32, now: i64, cx: &mut Context<Self>) -> impl IntoElement {
        let left = m::SIDEBAR_WIDTH;
        let place = self.settings.current().cloned();
        let key = place.as_ref().map(Place::key).unwrap_or_default();
        let loaded = self.loaded.get(&key);
        let status = self.status.get(&key).cloned();
        let backdrop = loaded
            .map(|loaded| {
                summary::backdrop(loaded.forecast.current.sky, loaded.forecast.current.day)
            })
            .unwrap_or((0x2D3F5E, 0x4A6FA5));
        let base = div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(left))
            .right_0()
            .bg(linear_gradient(
                180.0,
                linear_color_stop(rgb(backdrop.0), 0.0),
                linear_color_stop(rgb(backdrop.1), 1.0),
            ));
        let Some(loaded) = loaded else {
            let message = match status {
                Some(Status::Offline) => {
                    "Weather is offline. Connect to the internet to see this forecast."
                }
                Some(Status::Failed) => "The forecast is unavailable right now.",
                _ => "Loading…",
            };
            return base
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .when(status == Some(Status::Offline), |column| {
                    column.child(
                        svg()
                            .path("icons/weather/offline.svg")
                            .size(px(32.0))
                            .text_color(mac::white()),
                    )
                })
                .child(
                    div()
                        .text_size(px(15.0))
                        .text_color(mac::white())
                        .child(message),
                )
                .into_any_element();
        };
        let forecast = &loaded.forecast;
        let current = &forecast.current;
        let unit = self.unit;
        let panel_width = (width - left - 2.0 * m::PANEL_INSET).max(300.0);
        let header = div()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(m::HEADER_TOP))
            .text_color(mac::white())
            .child(
                div()
                    .text_size(px(m::CITY_SIZE))
                    .line_height(px(40.0))
                    .child(place.map(|place| place.name).unwrap_or_default()),
            )
            .child(
                div()
                    .text_size(px(m::TEMPERATURE_SIZE))
                    .line_height(px(110.0))
                    .font_weight(FontWeight::THIN)
                    .child(unit.degrees(current.temperature)),
            )
            .child(
                div()
                    .text_size(px(m::CONDITION_SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgba(0xFFFF_FFE6))
                    .child(current.sky.description(current.day)),
            )
            .children(forecast.days.first().map(|day| {
                div()
                    .text_size(px(m::CONDITION_SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!(
                        "H:{}  L:{}",
                        unit.degrees(day.high),
                        unit.degrees(day.low)
                    ))
            }));
        let columns = summary::hourly_strip(forecast, now, 24)
            .into_iter()
            .map(|column| hour_column(column, unit))
            .collect::<Vec<_>>();
        let hourly = panel()
            .w(px(panel_width))
            .child(
                div()
                    .px(px(16.0))
                    .pt(px(12.0))
                    .pb(px(10.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgba(0xFFFF_FFE6))
                    .child(summary::outlook(forecast, now)),
            )
            .child(rule())
            .child(
                div()
                    .id("weather-hourly")
                    .overflow_x_scroll()
                    .flex()
                    .px(px(8.0))
                    .py(px(12.0))
                    .children(columns),
            );
        let days = forecast
            .days
            .iter()
            .map(|day| {
                let (from, to) = summary::range_fractions(day, &forecast.days);
                let today = summary::day_label(day.time, now, forecast.utc_offset) == "Today";
                let dot =
                    today.then(|| summary::current_fraction(current.temperature, &forecast.days));
                div()
                    .h(px(m::DAY_ROW))
                    .flex()
                    .items_center()
                    .px(px(16.0))
                    .border_t_1()
                    .border_color(rgba(m::PANEL_RULE))
                    .text_size(px(17.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(mac::white())
                    .child(div().w(px(96.0)).child(summary::day_label(
                        day.time,
                        now,
                        forecast.utc_offset,
                    )))
                    .child(
                        div()
                            .w(px(64.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(sky_icon(day.sky, true, m::DAY_ICON))
                            .children(
                                day.precipitation_chance
                                    .filter(|chance| *chance >= 30.0)
                                    .map(|chance| {
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(0x8FD3FF))
                                            .child(format!("{chance:.0}%"))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .w(px(56.0))
                            .flex()
                            .justify_end()
                            .text_color(rgba(0xFFFF_FF8C))
                            .child(unit.degrees(day.low)),
                    )
                    .child(range_bar(from, to, dot))
                    .child(
                        div()
                            .w(px(44.0))
                            .flex()
                            .justify_end()
                            .child(unit.degrees(day.high)),
                    )
            })
            .collect::<Vec<_>>();
        let ten_day = panel()
            .w(px(panel_width))
            .child(caption("10-DAY FORECAST"))
            .children(days);
        let today = forecast.days.first();
        let sun = today
            .and_then(|day| Some((day.sunrise?, day.sunset?)))
            .map(|(rise, set)| {
                (
                    summary::time_label(rise, forecast.utc_offset, true),
                    format!(
                        "Sunset: {}",
                        summary::time_label(set, forecast.utc_offset, true)
                    ),
                )
            });
        let mut tiles = vec![
            tile(
                "FEELS LIKE",
                unit.degrees(current.feels_like),
                feels_note(current.feels_like, current.temperature),
            ),
            tile(
                "HUMIDITY",
                if current.humidity.is_finite() {
                    format!("{:.0}%", current.humidity)
                } else {
                    "--".into()
                },
                String::new(),
            ),
            tile(
                "WIND",
                if current.wind_speed.is_finite() {
                    summary::wind(current.wind_speed, unit)
                } else {
                    "--".into()
                },
                summary::compass(current.wind_direction).to_owned(),
            ),
        ];
        if let Some(uv) = today.and_then(|day| day.uv_index) {
            tiles.push(tile("UV INDEX", format!("{uv:.0}"), uv_note(uv).to_owned()));
        }
        if let Some((rise, set)) = sun {
            tiles.push(tile("SUNRISE", rise, set));
        }
        if let Some(chance) = today.and_then(|day| day.precipitation_chance) {
            tiles.push(tile(
                "PRECIPITATION",
                format!("{chance:.0}%"),
                "Chance today".into(),
            ));
        }
        let status_line = match status {
            Some(Status::Offline) => Some(format!(
                "Offline · {}",
                store::age_text(loaded.fetched_at, now)
            )),
            Some(Status::Failed) => Some(format!(
                "Couldn’t refresh · {}",
                store::age_text(loaded.fetched_at, now)
            )),
            _ => None,
        };
        base.child(
            div()
                .id("weather-main")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(16.0))
                .pb(px(20.0))
                .child(header)
                .child(hourly)
                .child(ten_day)
                .child(
                    div()
                        .w(px(panel_width))
                        .flex()
                        .flex_wrap()
                        .gap(px(12.0))
                        .children(tiles),
                )
                .child(attribution(status_line)),
        )
        .into_any_element()
    }
}

fn panel() -> gpui::Div {
    div()
        .flex_none()
        .rounded(px(m::PANEL_RADIUS))
        .bg(rgba(m::PANEL_FILL))
        .flex()
        .flex_col()
        .overflow_hidden()
}

fn rule() -> impl IntoElement {
    div().mx(px(16.0)).h(px(0.5)).bg(rgba(m::PANEL_RULE))
}

fn caption(text: &'static str) -> impl IntoElement {
    div()
        .px(px(16.0))
        .pt(px(12.0))
        .pb(px(8.0))
        .text_size(px(m::CAPTION_SIZE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgba(0xFFFF_FF8C))
        .child(text)
}

fn tile(title: &'static str, value: String, note: String) -> impl IntoElement {
    div()
        .w(px(160.0))
        .h(px(m::TILE_HEIGHT))
        .rounded(px(m::PANEL_RADIUS))
        .bg(rgba(m::PANEL_FILL))
        .p(px(14.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .text_color(mac::white())
        .child(
            div()
                .text_size(px(m::CAPTION_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(0xFFFF_FF8C))
                .child(title),
        )
        .child(div().text_size(px(28.0)).child(value))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(rgba(0xFFFF_FFCC))
                .child(note),
        )
}

fn feels_note(feels_like: f64, actual: f64) -> String {
    let difference = feels_like - actual;
    if difference >= 2.0 {
        "Humidity is making it feel warmer.".into()
    } else if difference <= -2.0 {
        "Wind is making it feel cooler.".into()
    } else {
        "Similar to the actual temperature.".into()
    }
}

fn uv_note(uv: f64) -> &'static str {
    match uv {
        uv if uv < 3.0 => "Low",
        uv if uv < 6.0 => "Moderate",
        uv if uv < 8.0 => "High",
        uv if uv < 11.0 => "Very High",
        _ => "Extreme",
    }
}

fn attribution(status: Option<String>) -> impl IntoElement {
    div()
        .py(px(14.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.0))
        .text_size(px(11.0))
        .text_color(rgba(0xFFFF_FF8C))
        .children(status.map(|status| div().text_color(mac::white()).child(status)))
        .child("Weather data by Open-Meteo.com")
}

/// A weather glyph built from the bundled parts, coloured like the Mac's
/// multicolour symbols.
fn sky_icon(sky: Sky, day: bool, size: f32) -> impl IntoElement {
    const WHITE: u32 = 0xFFFFFF;
    const SUN: u32 = 0xFFD60A;
    const RAIN: u32 = 0x5AC8FA;
    let parts: &[(&str, u32, f32, f32, f32)] = match (sky, day) {
        (Sky::Clear, true) => &[("sun", SUN, 0.0, 0.0, 1.0)],
        (Sky::Clear, false) => &[("moon", WHITE, 0.0, 0.0, 1.0)],
        (Sky::MostlyClear | Sky::PartlyCloudy, true) => &[
            ("sun-small", SUN, 0.1, 0.02, 0.55),
            ("cloud-front", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::MostlyClear | Sky::PartlyCloudy, false) => &[
            ("moon-small", WHITE, 0.1, 0.02, 0.55),
            ("cloud-front", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Cloudy, _) => &[("cloud", WHITE, 0.0, 0.0, 1.0)],
        (Sky::Fog, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("fog", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Snow | Sky::HeavySnow | Sky::SnowShowers, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("snow", WHITE, 0.0, 0.0, 1.0),
        ],
        (Sky::Thunderstorms, _) => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("bolt", SUN, 0.0, 0.0, 1.0),
        ],
        _ => &[
            ("cloud-top", WHITE, 0.0, 0.0, 1.0),
            ("rain", RAIN, 0.0, 0.0, 1.0),
        ],
    };
    div()
        .relative()
        .size(px(size))
        .children(parts.iter().map(|(name, color, dx, dy, scale)| {
            svg()
                .absolute()
                .left(px(dx * size))
                .top(px(dy * size))
                .size(px(size * scale))
                .path(SharedString::from(format!("icons/weather/{name}.svg")))
                .text_color(rgb(*color))
        }))
}

fn hour_column(column: Column, unit: Unit) -> impl IntoElement {
    let (label, icon, value, chance): (String, AnyElement, String, Option<f64>) = match column {
        Column::Hour {
            label,
            sky,
            day,
            temperature,
            precipitation_chance,
        } => (
            label,
            sky_icon(sky, day, m::HOUR_ICON).into_any_element(),
            unit.degrees(temperature),
            precipitation_chance,
        ),
        Column::Sunrise { label } => (
            label,
            sky_icon(Sky::Clear, true, m::HOUR_ICON).into_any_element(),
            "Sunrise".into(),
            None,
        ),
        Column::Sunset { label } => (
            label,
            sky_icon(Sky::Clear, true, m::HOUR_ICON).into_any_element(),
            "Sunset".into(),
            None,
        ),
    };
    div()
        .flex_none()
        .w(px(m::HOUR_WIDTH))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .text_color(mac::white())
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(
            div()
                .h(px(m::HOUR_ICON + 12.0))
                .flex()
                .flex_col()
                .items_center()
                .child(icon)
                .children(chance.map(|chance| {
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8FD3FF))
                        .child(format!("{chance:.0}%"))
                })),
        )
        .child(
            div()
                .text_size(px(17.0))
                .font_weight(FontWeight::MEDIUM)
                .child(value),
        )
}

/// The ten-day temperature bar: the day's range on the shared scale, with
/// today's current temperature as a dot.
fn range_bar(from: f32, to: f32, dot: Option<f32>) -> impl IntoElement {
    let width = m::RANGE_WIDTH;
    let warm: Hsla = rgb(0xF2C94C).into();
    div()
        .mx(px(12.0))
        .relative()
        .w(px(width))
        .h(px(4.0))
        .rounded(px(2.0))
        .bg(rgba(0x0000_0033))
        .child(
            div()
                .absolute()
                .left(px(from * width))
                .w(px(((to - from) * width).max(4.0)))
                .h_full()
                .rounded(px(2.0))
                .bg(linear_gradient(
                    90.0,
                    linear_color_stop(rgb(0x8FD16A), 0.0),
                    linear_color_stop(warm, 1.0),
                )),
        )
        .children(dot.map(|dot| {
            div()
                .absolute()
                .left(px(dot * width - 3.0))
                .top(px(-1.0))
                .size(px(6.0))
                .rounded_full()
                .bg(mac::white())
                .border_1()
                .border_color(rgba(0x0000_0066))
        }))
}

impl Render for WeatherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let now = now_seconds();
        let (light_x, light_y) = m::TRAFFIC_LIGHT_CENTER;
        let body = if self.settings.places.is_empty() {
            self.render_first_run(cx).into_any_element()
        } else {
            div()
                .size_full()
                .child(self.render_main(width, now, cx))
                .child(self.render_sidebar(now, cx))
                .into_any_element()
        };
        let error = self.settings_error.clone();
        div()
            .id("weather")
            .track_focus(&self.focus)
            .key_context("Weather")
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh_all(true, cx)))
            .on_action(cx.listener(|this, _: &FindCity, window, cx| {
                this.search.update(cx, |state, cx| state.focus(window, cx));
            }))
            .on_action(cx.listener(|this, _: &UseCelsius, _, cx| this.set_unit(Unit::Celsius, cx)))
            .on_action(
                cx.listener(|this, _: &UseFahrenheit, _, cx| this.set_unit(Unit::Fahrenheit, cx)),
            )
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(m::SIDEBAR_FILL))
            .child(body)
            .child(
                div()
                    .id("weather-drag")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(m::TOOLBAR_HEIGHT - 8.0))
                    .window_control_area(WindowControlArea::Drag)
                    .on_double_click(|_, window, _| window.zoom_window()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(light_x - mac::traffic_light_hit_width() / 2.0))
                    .top(px(light_y - mac::traffic_light_hit_height() / 2.0))
                    .child(rmac_ui::traffic_lights_active(window.is_window_active())),
            )
            .children(error.map(|message| {
                div()
                    .absolute()
                    .bottom(px(8.0))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(px(12.0))
                    .text_color(mac::danger())
                    .child(message)
            }))
    }
}
