//! Setup Assistant pages, drawn with rmac-ui (design-lab/setup-assistant.html;
//! every value S).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    div, img, prelude::FluentBuilder as _, px, rgb, svg, AnyElement, App, AppContext as _,
    ClickEvent, Context, Div, Entity, FocusHandle, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_setup_assistant::flow::{Availability, Completion, Event, Flow, Outcome, Step};
use rmac_setup_assistant::names::{self, LocaleChoice};
use rmac_setup_assistant::services::{self, Account};
use rmac_setup_assistant::{greeting, mac_shortcuts, marker};
use rmac_ui::{mac, text_px, Button, InputState, List, ListRow, StyledExt as _, TextField, Toggle};

/// Page metrics (design-lab/setup-assistant.html header; all S).
mod metrics {
    pub const WIDTH: f32 = 800.0;
    pub const HEIGHT: f32 = 600.0;
    pub const HERO: f32 = 64.0;
    pub const HERO_TOP: f32 = 48.0;
    pub const HERO_GLYPH: f32 = 30.0;
    pub const TITLE: f32 = 26.0;
    pub const TITLE_LINE: f32 = 32.0;
    pub const BODY: f32 = 13.0;
    pub const BODY_LINE: f32 = 18.0;
    pub const CONTENT_WIDTH: f32 = 520.0;
    pub const EDGE: f32 = 24.0;
    pub const BUTTON_HEIGHT: f32 = 28.0;
    pub const ROW: f32 = 32.0;
    pub const WELL_RADIUS: f32 = 10.0;
    pub const WELL_ROWS: f32 = 7.0;
    pub const GREETING: f32 = 64.0;
    pub const GO: f32 = 44.0;
    pub const FACE: f32 = 56.0;
    pub const LOOK_WIDTH: f32 = 136.0;
    pub const LOOK_HEIGHT: f32 = 88.0;
    pub const TICK_MS: u64 = 33;
}

const WELL: u32 = 0x29272D;
const WELL_RULE: u32 = 0x3A3840;
const GREY_TILE: u32 = 0x8E8E93;
const ORANGE_TILE: u32 = 0xFF9F0A;
const INDIGO_TILE: u32 = 0x5E5CE6;
const GREEN_TILE: u32 = 0x30D158;

pub struct SetupView {
    focus: FocusHandle,
    flow: Flow,
    greeting_started: Instant,
    busy: bool,
    error: Option<SharedString>,

    locale: Option<rmac_locale::Snapshot>,
    locale_choices: Vec<LocaleChoice>,
    language: Option<String>,
    region: Option<String>,

    keyboard: Option<rmac_keyboard::Status>,
    /// The Mac Shortcuts page's toggle: defaults on, the user can untick it.
    /// Applying the choice happens on Continue, through the same
    /// `rmac_keyboard::apply` System Settings › Keyboard calls.
    mac_shortcuts_enabled: bool,
    /// Set after Continue could not turn Mac shortcuts on (the admin
    /// password was cancelled or denied, or the helper otherwise failed).
    /// Setup keeps going regardless; this only explains why the switch is
    /// off.
    mac_shortcuts_note: Option<SharedString>,

    wifi: Option<rmac_network::WifiSnapshot>,
    wifi_selected: Option<rmac_network::WifiNetworkId>,
    wifi_password: Entity<InputState>,

    account: Option<Account>,
    real_name: Entity<InputState>,
    faces: Vec<PathBuf>,
    picture: Option<PathBuf>,

    scheme: Option<rmac_theme::SchemePreference>,
}

impl SetupView {
    pub fn new(
        availability: Availability,
        wifi: Option<rmac_network::WifiSnapshot>,
        account: Option<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let real_name_value = account
            .as_ref()
            .map(|account| account.real_name.clone())
            .unwrap_or_default();
        let real_name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(real_name_value)
                .placeholder("Full name")
        });
        let wifi_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });

        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = view.update(cx, |view, cx| view.handle(Event::Close, cx));
            true
        });

        let focus = cx.focus_handle();
        focus.focus(window, cx);

        let mut this = Self {
            focus,
            flow: Flow::new(availability),
            greeting_started: Instant::now(),
            busy: false,
            error: None,
            locale: None,
            locale_choices: Vec::new(),
            language: None,
            region: None,
            keyboard: None,
            mac_shortcuts_enabled: true,
            mac_shortcuts_note: None,
            wifi,
            wifi_selected: None,
            wifi_password,
            account,
            real_name,
            faces: services::faces(),
            picture: None,
            scheme: None,
        };
        this.load(cx);
        this.start_greeting(cx);
        this
    }

    /// Read every page's current state off the UI thread.
    fn load(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let locale = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Ok(snapshot) = locale {
                    view.locale_choices = names::choices(&snapshot.installed_locales);
                    view.language = view
                        .locale_choices
                        .iter()
                        .find(|choice| names::same_locale(&choice.code, snapshot.language()))
                        .map(|choice| choice.code.clone());
                    view.region = view
                        .locale_choices
                        .iter()
                        .find(|choice| names::same_locale(&choice.code, snapshot.region_locale()))
                        .map(|choice| choice.code.clone());
                    view.locale = Some(snapshot);
                }
                cx.notify();
            });
            let keyboard = cx
                .background_executor()
                .spawn(async { rmac_keyboard::status() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.keyboard = keyboard.ok();
                cx.notify();
            });
            let scheme = cx
                .background_executor()
                .spawn(async { services::color_scheme().await })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.scheme = scheme.ok();
                cx.notify();
            });
        })
        .detach();
    }

    /// Redraw the greeting about 30 times a second while Welcome shows.
    fn start_greeting(&mut self, cx: &mut Context<Self>) {
        self.greeting_started = Instant::now();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_millis(metrics::TICK_MS))
                .await;
            let showing = this
                .update(cx, |view, cx| {
                    let showing = view.flow.ended().is_none() && view.flow.step() == Step::Welcome;
                    if showing {
                        cx.notify();
                    }
                    showing
                })
                .unwrap_or(false);
            if !showing {
                break;
            }
        })
        .detach();
    }

    fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match self.flow.handle(event) {
            Outcome::Ignored => {}
            Outcome::Moved(step) => {
                self.error = None;
                if step == Step::Welcome {
                    self.start_greeting(cx);
                }
                if step == Step::WiFi {
                    self.refresh_wifi(cx);
                }
                cx.notify();
            }
            Outcome::Ended(completion) => self.finish(completion, cx),
        }
    }

    fn finish(&mut self, completion: Completion, cx: &mut Context<Self>) {
        if let Err(error) = marker::mark_complete(completion) {
            eprintln!("rmac-setup-assistant: could not record that setup ran: {error}");
        }
        cx.quit();
    }

    /// Run `work` off the UI thread; on success move on, on failure stay and
    /// say why.
    fn apply_then_continue(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(()) => view.handle(Event::Continue, cx),
                    Err(error) => {
                        view.error = Some(error.into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn continue_pressed(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match self.flow.step() {
            Step::LanguageRegion => {
                let (Some(snapshot), Some(language), Some(region)) = (
                    self.locale.clone(),
                    self.language.clone(),
                    self.region.clone(),
                ) else {
                    return self.handle(Event::Continue, cx);
                };
                self.apply_then_continue(cx, move || {
                    services::apply_locale(&snapshot, &language, &region).map(|_| ())
                });
            }
            Step::MacShortcuts => self.apply_mac_shortcuts_choice(cx),
            Step::WiFi => self.join_selected_wifi(cx),
            Step::Account => {
                let name = self.real_name.read(cx).value().trim().to_owned();
                let current = self
                    .account
                    .as_ref()
                    .map(|account| account.real_name.clone())
                    .unwrap_or_default();
                let picture = self.picture.clone();
                if name == current && picture.is_none() {
                    return self.handle(Event::Continue, cx);
                }
                self.apply_then_continue(cx, move || {
                    if !name.is_empty() && name != current {
                        services::set_real_name(&name)?;
                    }
                    if let Some(picture) = picture {
                        services::set_icon_file(&picture)?;
                    }
                    Ok(())
                });
            }
            _ => self.handle(Event::Continue, cx),
        }
    }

    fn refresh_wifi(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async {
                    let _ = rmac_network::request_scan();
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Ok(snapshot) = snapshot {
                    view.wifi = Some(snapshot);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn selected_network(&self) -> Option<&rmac_network::WifiNetwork> {
        let selected = self.wifi_selected.as_ref()?;
        self.wifi
            .as_ref()?
            .networks
            .iter()
            .find(|network| &network.id == selected)
    }

    fn join_selected_wifi(&mut self, cx: &mut Context<Self>) {
        let Some(network) = self.selected_network().cloned() else {
            return self.handle(Event::Continue, cx);
        };
        if network.connected {
            return self.handle(Event::Continue, cx);
        }
        let password = network
            .needs_password()
            .then(|| self.wifi_password.read(cx).value().to_string());
        self.apply_then_continue(cx, move || {
            let result = match password {
                Some(password) => {
                    let password = rmac_network::WifiPassword::new(password, &network.id)
                        .map_err(|error| error.to_string())?;
                    rmac_network::connect_with_password(
                        &network.id,
                        password,
                        &rmac_network::WifiCancellation::new(),
                    )
                }
                None => rmac_network::connect(&network.id),
            };
            result.map(|_| ()).map_err(|error| error.to_string())
        });
    }

    fn set_keyboard(
        &mut self,
        change: impl FnOnce(&mut rmac_keyboard::MacKeyboard),
        cx: &mut Context<Self>,
    ) {
        let Some(status) = &self.keyboard else {
            return;
        };
        if self.busy {
            return;
        }
        let mut target = status.state;
        change(&mut target);
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_keyboard::apply(&target) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(status) => view.keyboard = Some(status),
                    Err(error) => view.error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Continue on the Mac Shortcuts page: apply the toggle's choice through
    /// the same `rmac_keyboard::apply` System Settings › Keyboard calls
    /// (`docs/decisions/0017-mac-keyboard.md`), then move on regardless of
    /// the result. A cancelled or denied password, or keyd being
    /// unavailable, never blocks setup — it only turns the choice off and
    /// says so.
    fn apply_mac_shortcuts_choice(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.keyboard.clone() else {
            return self.handle(Event::Continue, cx);
        };
        let Some(target) = mac_shortcuts::target(&status, self.mac_shortcuts_enabled) else {
            return self.handle(Event::Continue, cx);
        };
        self.busy = true;
        self.mac_shortcuts_note = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_keyboard::apply(&target) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(status) => {
                        view.keyboard = Some(status);
                        view.handle(Event::Continue, cx);
                    }
                    Err(error) => {
                        // Don't block setup over an optional convenience:
                        // turn the choice off, explain why, and let the next
                        // Continue move on (it will find nothing left to
                        // change).
                        view.mac_shortcuts_enabled = false;
                        view.mac_shortcuts_note = Some(mac_shortcuts::declined_note(&error).into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn set_scheme(&mut self, scheme: rmac_theme::SchemePreference, cx: &mut Context<Self>) {
        if self.busy || self.scheme == Some(scheme) {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { services::set_color_scheme(scheme).await })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                match result {
                    Ok(()) => view.scheme = Some(scheme),
                    Err(error) => view.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------------------------------------------------------------- pages

    fn page(&self, icon: &'static str, tint: Hsla, subtitle: &'static str) -> Div {
        let step = self.flow.step();
        div()
            .size_full()
            .v_flex()
            .items_center()
            .pt(px(metrics::HERO_TOP))
            .child(hero(icon, tint))
            .child(
                div()
                    .mt(px(16.0))
                    .text_size(text_px(metrics::TITLE))
                    .line_height(px(metrics::TITLE_LINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::text())
                    .child(step.title()),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .w(px(metrics::CONTENT_WIDTH))
                    .text_center()
                    .text_size(text_px(metrics::BODY))
                    .line_height(px(metrics::BODY_LINE))
                    .text_color(mac::text_secondary())
                    .child(subtitle),
            )
    }

    fn content(&self) -> Div {
        div().mt(px(24.0)).w(px(metrics::CONTENT_WIDTH)).v_flex()
    }

    fn render_welcome(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let elapsed = self.greeting_started.elapsed().as_millis() as u64;
        let frame = greeting::frame(elapsed, rmac_ui::theme::current().motion.spatial_motion);
        let get_started_focus = window
            .use_keyed_state("get-started", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let get_started_focused = get_started_focus.is_focused(window);
        div()
            .size_full()
            .v_flex()
            .items_center()
            .child(
                div()
                    .mt(px(200.0 + frame.offset))
                    .h(px(90.0))
                    .text_size(px(metrics::GREETING))
                    .line_height(px(90.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(greeting::color(frame.word)))
                    .opacity(frame.opacity)
                    .child(greeting::WORDS[frame.word]),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("get-started")
                    .role(Role::Button)
                    .aria_label("Get Started")
                    .size(px(metrics::GO))
                    .rounded_full()
                    .bg(mac::accent())
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .track_focus(&get_started_focus.tab_stop(true).tab_index(0))
                    .when(get_started_focused, |el| {
                        el.shadow(mac::focus_ring_shadow())
                    })
                    .child(
                        svg()
                            .path("icons/arrow-right.svg")
                            .size(px(20.0))
                            .text_color(mac::on_accent()),
                    )
                    // GPUI maps Space/Return to a click on any focused element
                    // with an `on_click` handler, so this alone makes the
                    // circle keyboard-operable once it's a tab stop above.
                    .on_click(
                        cx.listener(|view, _: &ClickEvent, _, cx| view.handle(Event::Continue, cx)),
                    ),
            )
            .child(
                div()
                    .mt(px(10.0))
                    .mb(px(92.0 - metrics::BODY_LINE))
                    .text_size(text_px(metrics::BODY))
                    .text_color(mac::text_secondary())
                    .child("Get Started"),
            )
    }

    fn render_language(&self, cx: &mut Context<Self>) -> Div {
        let column = |title: &'static str,
                      id: &'static str,
                      selected: &Option<String>,
                      label: fn(&LocaleChoice) -> String,
                      pick: fn(&mut Self, String),
                      cx: &mut Context<Self>| {
            let rows = self.locale_choices.iter().map(|choice| {
                let code = choice.code.clone();
                ListRow::new(
                    SharedString::from(format!("{id}-{}", choice.code)),
                    div()
                        .px(px(12.0))
                        .text_size(text_px(metrics::BODY))
                        .child(label(choice)),
                )
                .selected(selected.as_deref() == Some(choice.code.as_str()))
                .h(px(metrics::ROW))
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    pick(view, code.clone());
                    cx.notify();
                }))
            });
            div().flex_1().v_flex().child(section_label(title)).child(
                well()
                    .id(id)
                    .h(px(metrics::ROW * metrics::WELL_ROWS))
                    .overflow_y_scroll()
                    .child(List::new(rows)),
            )
        };
        let body = if self.locale_choices.is_empty() {
            self.content().child(note(if self.locale.is_some() {
                "No languages are installed besides the system default."
            } else {
                "Loading languages…"
            }))
        } else {
            let language = column(
                "Language",
                "languages",
                &self.language,
                |choice| choice.language.clone(),
                |view, code| view.language = Some(code),
                cx,
            );
            let region = column(
                "Region",
                "regions",
                &self.region,
                |choice| choice.region.clone(),
                |view, code| view.region = Some(code),
                cx,
            );
            self.content()
                .child(div().flex().gap(px(16.0)).child(language).child(region))
        };
        self.page(
            "setup/globe.svg",
            mac::accent(),
            "Choose the language Lulo OS uses and the region for dates, times and numbers.",
        )
        .child(body)
    }

    fn render_keyboard(&self, cx: &mut Context<Self>) -> Div {
        let mut body = self.content();
        let source = self
            .locale
            .as_ref()
            .map(|snapshot| {
                if snapshot.x11_layout.is_empty() {
                    "Not reported".to_owned()
                } else {
                    snapshot.x11_layout.replace(',', ", ")
                }
            })
            .unwrap_or_else(|| "Loading…".to_owned());
        match &self.keyboard {
            Some(status) => {
                let swap = status.state.layout.swap_command_option;
                // The PC keys left of the space bar, labelled with the Mac
                // key each one now is.
                let (windows_role, alt_role) = if swap {
                    (("⌥", "option"), ("⌘", "command"))
                } else {
                    (("⌘", "command"), ("⌥", "option"))
                };
                body = body.child(
                    div()
                        .flex()
                        .justify_center()
                        .gap(px(6.0))
                        .mb(px(18.0))
                        .child(keycap("⌃", "control", "Ctrl", false))
                        .child(keycap(windows_role.0, windows_role.1, "Windows", !swap))
                        .child(keycap(alt_role.0, alt_role.1, "Alt", swap))
                        .child(keycap("", "", "", false).w(px(190.0))),
                );
                body = body.child(well().child(value_row("Input source", source)).child(
                    switch_row(
                        "setup-swap",
                        "⌘ Command next to the space bar",
                        "Alt works as ⌘ Command and the Windows key as ⌥ Option",
                        swap,
                        !self.busy,
                        cx.listener(|view, on: &bool, _, cx| {
                            let on = *on;
                            view.set_keyboard(|t| t.layout.swap_command_option = on, cx);
                        }),
                    ),
                ));
            }
            None => {
                body = body.child(well().child(value_row("Input source", source)));
            }
        }
        self.page(
            "setup/keyboard.svg",
            rgb(GREY_TILE).into(),
            "Lulo OS works like a Mac. On a PC keyboard it can put ⌘ Command next to the space bar.",
        )
        .child(body)
    }

    fn render_mac_shortcuts(&self, cx: &mut Context<Self>) -> Div {
        let mut body = self.content().child(
            div()
                .flex()
                .justify_center()
                .gap(px(28.0))
                .mb(px(20.0))
                .child(key_mapping("Ctrl", "⌃", "Control"))
                .child(key_mapping("Alt", "⌥", "Option"))
                .child(key_mapping("Win", "⌘", "Command")),
        );
        match &self.keyboard {
            Some(status) => {
                let reason = mac_shortcuts::unavailable_reason(status);
                let can_toggle = reason.is_none();
                body = body.child(well().child(switch_row(
                    "setup-mac-shortcuts",
                    "Use Mac shortcuts in all apps",
                    "⌘C, ⌘V and the other ⌘ shortcuts work in apps made for PC keyboards. \
                     Terminals keep ⌃C for interrupting.",
                    self.mac_shortcuts_enabled && can_toggle,
                    !self.busy && can_toggle,
                    cx.listener(|view, on: &bool, _, cx| {
                        view.mac_shortcuts_enabled = *on;
                        view.mac_shortcuts_note = None;
                        cx.notify();
                    }),
                )));
                if let Some(reason) = reason {
                    body = body.child(hint(reason));
                } else if let Some(note) = &self.mac_shortcuts_note {
                    body = body.child(hint(note.clone()));
                }
            }
            None => body = body.child(note("Loading keyboard settings…")),
        }
        self.page(
            "setup/keyboard.svg",
            rgb(GREY_TILE).into(),
            "On a PC keyboard, the key next to the space bar works as ⌘ — in every app, not just Lulo OS apps.",
        )
        .child(body)
    }

    fn render_wifi(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let _ = window;
        let mut body = self.content();
        match &self.wifi {
            Some(snapshot) if !snapshot.enabled => {
                body = body.child(note("Wi-Fi is turned off.")).child(
                    div().mt(px(12.0)).flex().justify_center().child(
                        Button::new("setup-wifi-on", "Turn Wi-Fi On")
                            .disabled(self.busy)
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                                view.busy = true;
                                cx.notify();
                                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                                    let result = cx
                                        .background_executor()
                                        .spawn(async { rmac_network::set_enabled(true) })
                                        .await;
                                    let _ = this.update(cx, |view, cx| {
                                        view.busy = false;
                                        if let Err(error) = result {
                                            view.error = Some(error.to_string().into());
                                        }
                                        view.refresh_wifi(cx);
                                    });
                                })
                                .detach();
                            })),
                    ),
                );
            }
            Some(snapshot) => {
                let mut networks = snapshot.networks.clone();
                networks.sort_by(|a, b| {
                    b.connected
                        .cmp(&a.connected)
                        .then(b.strength.cmp(&a.strength))
                });
                let mut seen = std::collections::HashSet::new();
                networks.retain(|network| seen.insert(network.ssid.clone()));
                let rows = networks.into_iter().take(12).map(|network| {
                    let id = network.id.clone();
                    let selected = self.wifi_selected.as_ref() == Some(&network.id);
                    let usable = network.connected || network.can_connect();
                    ListRow::new(
                        SharedString::from(format!("wifi-{}", network.ssid)),
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(12.0))
                            .text_size(text_px(metrics::BODY))
                            .child(div().flex_1().child(network.ssid.clone()))
                            .when(network.connected, |row| row.child("Connected"))
                            .when(network.security.is_secure(), |row| {
                                row.child(svg().path("setup/lock.svg").size(px(12.0)).text_color(
                                    if selected {
                                        mac::on_accent()
                                    } else {
                                        mac::text_secondary()
                                    },
                                ))
                            })
                            .child(strength_bars(network.strength, selected)),
                    )
                    .selected(selected)
                    .disabled(!usable || self.busy)
                    .h(px(metrics::ROW))
                    .on_click(cx.listener(
                        move |view, _: &ClickEvent, _, cx| {
                            view.wifi_selected = Some(id.clone());
                            view.error = None;
                            cx.notify();
                        },
                    ))
                });
                body = body.child(
                    well()
                        .id("wifi-networks")
                        .max_h(px(metrics::ROW * metrics::WELL_ROWS))
                        .overflow_y_scroll()
                        .child(List::new(rows)),
                );
                if let Some(network) = self.selected_network() {
                    if network.needs_password() {
                        body = body.child(labelled_field(
                            "Password",
                            TextField::new(&self.wifi_password).into_any_element(),
                        ));
                    } else if network.needs_enterprise_setup() {
                        body = body.child(note(
                            "This network needs a user name and certificate. Join it later in System Settings › Wi-Fi.",
                        ));
                    }
                }
            }
            None => body = body.child(note("Looking for networks…")),
        }
        self.page(
            "setup/wifi.svg",
            mac::accent(),
            "Connect to the internet for updates, the time and the weather.",
        )
        .child(body)
    }

    fn render_account(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let account = self.account.clone().unwrap_or_default();
        let typed = self.real_name.read(cx).value().to_string();
        let monogram = names::monogram(&typed, &account.user_name);
        let mut pictures = div().mt(px(16.0)).flex().justify_center().gap(px(12.0));
        // First the current picture (or the monogram), then the system set.
        let current_selected = self.picture.is_none();
        let current = match &account.icon_file {
            Some(path) => img(path.clone())
                .size(px(metrics::FACE))
                .rounded_full()
                .into_any_element(),
            None => monogram_face(&monogram).into_any_element(),
        };
        pictures = pictures.child(
            face_frame(
                "face-current",
                "Current picture",
                current_selected,
                current,
                window,
                cx,
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.picture = None;
                cx.notify();
            })),
        );
        for (index, face) in self.faces.iter().enumerate() {
            let path = face.clone();
            let selected = self.picture.as_ref() == Some(face);
            pictures = pictures.child(
                face_frame(
                    SharedString::from(format!("face-{index}")),
                    SharedString::from(format!("Picture {}", index + 1)),
                    selected,
                    img(face.clone())
                        .size(px(metrics::FACE))
                        .rounded_full()
                        .into_any_element(),
                    window,
                    cx,
                )
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.picture = Some(path.clone());
                    cx.notify();
                })),
            );
        }
        let body = self
            .content()
            .child(labelled_field(
                "Full name",
                TextField::new(&self.real_name).into_any_element(),
            ))
            .child(
                div()
                    .mt(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(field_label("Account name"))
                    .child(
                        div()
                            .text_size(text_px(metrics::BODY))
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "{} · set when this computer was installed",
                                account.user_name
                            )),
                    ),
            )
            .child(pictures);
        self.page(
            "setup/user.svg",
            rgb(GREY_TILE).into(),
            "Choose how your name and picture appear on the lock screen and in Settings.",
        )
        .child(body)
    }

    fn render_appearance(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        use rmac_theme::SchemePreference as Scheme;

        let current_index = APPEARANCE_OPTIONS
            .iter()
            .position(|(_, _, scheme)| Some(*scheme) == self.scheme)
            .unwrap_or(0);

        let mut row = div()
            .flex()
            .justify_center()
            .gap(px(28.0))
            .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
                let Some(next) =
                    appearance_roving_target(current_index, event.keystroke.key.as_str())
                else {
                    return;
                };
                window.prevent_default();
                cx.stop_propagation();
                let (id, _, scheme) = APPEARANCE_OPTIONS[next];
                let handle = window
                    .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone();
                handle.focus(window, cx);
                view.set_scheme(scheme, cx);
            }));
        for (index, (id, name, scheme)) in APPEARANCE_OPTIONS.into_iter().enumerate() {
            let picture = match scheme {
                Scheme::Light => div().bg(rgb(0xF2F2F7)),
                Scheme::Dark => div().bg(rgb(0x2C2A31)),
                Scheme::Automatic => div()
                    .flex()
                    .overflow_hidden()
                    .child(div().flex_1().h_full().bg(rgb(0xF2F2F7)))
                    .child(div().flex_1().h_full().bg(rgb(0x2C2A31))),
            };
            let selected = self.scheme == Some(scheme);
            let focus = window
                .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone();
            let focused = focus.is_focused(window);
            row = row.child(
                div()
                    .id(id)
                    .role(Role::RadioButton)
                    .aria_selected(selected)
                    .aria_label(name)
                    .v_flex()
                    .items_center()
                    .gap(px(8.0))
                    .cursor_pointer()
                    .track_focus(&focus.tab_stop(true).tab_index(index as isize))
                    .when(focused, |el| el.shadow(mac::focus_ring_shadow()))
                    .child(
                        picture
                            .w(px(metrics::LOOK_WIDTH))
                            .h(px(metrics::LOOK_HEIGHT))
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(rgb(WELL_RULE))
                            .when(selected, |picture| {
                                picture.border_2().border_color(mac::accent())
                            }),
                    )
                    .child(
                        div()
                            .text_size(text_px(metrics::BODY))
                            .text_color(mac::text())
                            .child(name),
                    )
                    .on_click(
                        cx.listener(move |view, _: &ClickEvent, _, cx| view.set_scheme(scheme, cx)),
                    ),
            );
        }
        let body = self.content().mt(px(50.0)).child(row);
        self.page(
            "setup/palette.svg",
            rgb(0x1C1C1E).into(),
            "You can change this at any time in System Settings › Appearance.",
        )
        .child(body)
    }

    fn render_tips(&self) -> Div {
        self.page(
            "setup/app-window.svg",
            rgb(ORANGE_TILE).into(),
            "A few things that work just like on a Mac.",
        )
        .child(
            self.content()
                .child(tip(
                    "setup/app-window.svg",
                    mac::accent(),
                    "The Dock",
                    "Your apps sit at the bottom of the screen. Click one to open it; a dot shows that it is running.",
                ))
                .child(tip(
                    "setup/panel-top.svg",
                    rgb(GREY_TILE).into(),
                    "The menu bar",
                    "The app you are using shows its menus at the top left. Wi-Fi, sound, Control Centre and the clock are at the top right.",
                ))
                .child(tip(
                    "setup/search.svg",
                    rgb(INDIGO_TILE).into(),
                    "Spotlight",
                    "Press ⌘ Space to find apps, files and settings, or to do a quick calculation.",
                )),
        )
    }

    fn render_privacy(&self) -> Div {
        self.page(
            "setup/shield.svg",
            mac::accent(),
            "Lulo OS is designed to keep your information on this computer.",
        )
        .child(
            self.content()
                .child(tip(
                    "setup/lock.svg",
                    rgb(GREEN_TILE).into(),
                    "Stays on your computer",
                    "Lulo OS collects no usage data. Notes, clipboard history and notifications are stored only in your home folder.",
                ))
                .child(tip(
                    "setup/user.svg",
                    rgb(ORANGE_TILE).into(),
                    "Apps ask first",
                    "Apps ask before they use the camera, microphone, screen or location.",
                ))
                .child(tip(
                    "setup/shield.svg",
                    rgb(GREY_TILE).into(),
                    "You stay in control",
                    "Review and change what apps can use in System Settings › Privacy & Security.",
                )),
        )
    }

    fn render_done(&self) -> Div {
        div()
            .size_full()
            .v_flex()
            .items_center()
            .pt(px(150.0))
            .child(hero("icons/check.svg", rgb(GREEN_TILE).into()))
            .child(
                div()
                    .mt(px(16.0))
                    .text_size(text_px(metrics::TITLE))
                    .line_height(px(metrics::TITLE_LINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(mac::text())
                    .child(Step::Done.title()),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(text_px(metrics::BODY))
                    .text_color(mac::text_secondary())
                    .child("You can run Setup Assistant again from System Settings › General."),
            )
    }

    fn bottom_bar(&self, cx: &mut Context<Self>) -> Div {
        let step = self.flow.step();
        let mut bar = div()
            .absolute()
            .left(px(metrics::EDGE))
            .right(px(metrics::EDGE))
            .bottom(px(metrics::EDGE))
            .h(px(metrics::BUTTON_HEIGHT))
            .flex()
            .items_center()
            .gap(px(16.0));
        if step == Step::Welcome {
            return bar.child(Button::new("skip-setup", "Skip Setup").ghost().on_click(
                cx.listener(|view, _: &ClickEvent, _, cx| view.handle(Event::SkipSetup, cx)),
            ));
        }
        if self.flow.can_go_back() {
            bar = bar.child(
                Button::new("back", "Back")
                    .h(px(metrics::BUTTON_HEIGHT))
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(|view, _: &ClickEvent, _, cx| view.handle(Event::Back, cx)),
                    ),
            );
        }
        bar = bar.child(div().flex_1());
        if step.can_set_up_later() {
            bar = bar.child(
                Button::new("later", "Set Up Later")
                    .ghost()
                    .disabled(self.busy)
                    .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                        view.handle(Event::SetUpLater, cx)
                    })),
            );
        }
        let label = if step == Step::Done {
            "Get Started"
        } else {
            "Continue"
        };
        bar.child(
            Button::new("continue", label)
                .primary()
                .h(px(metrics::BUTTON_HEIGHT))
                .busy(self.busy)
                .disabled(self.busy)
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.continue_pressed(cx))),
        )
    }
}

impl Render for SetupView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.flow.step() {
            Step::Welcome => self.render_welcome(window, cx),
            Step::LanguageRegion => self.render_language(cx),
            Step::Keyboard => self.render_keyboard(cx),
            Step::MacShortcuts => self.render_mac_shortcuts(cx),
            Step::WiFi => self.render_wifi(window, cx),
            Step::Account => self.render_account(window, cx),
            Step::Appearance => self.render_appearance(window, cx),
            Step::Tips => self.render_tips(),
            Step::Privacy => self.render_privacy(),
            Step::Done => self.render_done(),
        };
        let page = match &self.error {
            Some(error) => page.child(
                div()
                    .mt(px(12.0))
                    .w(px(metrics::CONTENT_WIDTH))
                    .text_center()
                    .text_size(text_px(12.0))
                    .text_color(mac::danger())
                    .child(error.clone()),
            ),
            None => page,
        };
        div()
            .track_focus(&self.focus)
            .key_context("SetupAssistant")
            .on_action(cx.listener(|view, _: &crate::Continue, _, cx| view.continue_pressed(cx)))
            .relative()
            .size_full()
            .bg(mac::window())
            .font_family(rmac_ui::UI_FONT)
            .text_color(mac::text())
            .child(page)
            .child(self.bottom_bar(cx))
    }
}

// ------------------------------------------------------------- pieces

fn hero(icon: &'static str, tint: Hsla) -> Div {
    div()
        .size(px(metrics::HERO))
        .rounded_full()
        .bg(tint)
        .flex()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path(icon)
                .size(px(metrics::HERO_GLYPH))
                .text_color(mac::white()),
        )
}

fn well() -> Div {
    div()
        .w_full()
        .rounded(px(metrics::WELL_RADIUS))
        .bg(rgb(WELL))
        .overflow_hidden()
        .v_flex()
}

fn section_label(title: &'static str) -> Div {
    div()
        .mb(px(6.0))
        .ml(px(4.0))
        .text_size(text_px(metrics::BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(mac::text_secondary())
        .child(title)
}

fn note(text: &'static str) -> Div {
    hint(text)
}

/// Like [`note`], but for text built at run time (an error's own words, a
/// footnote that names a package).
fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(12.0))
        .text_center()
        .text_size(text_px(12.0))
        .text_color(mac::text_secondary())
        .child(text.into())
}

fn value_row(title: &'static str, value: String) -> Div {
    div()
        .h(px(metrics::ROW))
        .flex()
        .items_center()
        .px(px(12.0))
        .text_size(text_px(metrics::BODY))
        .child(div().flex_1().child(title))
        .child(div().text_color(mac::text_secondary()).child(value))
}

fn switch_row(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    checked: bool,
    enabled: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(rgb(WELL_RULE))
        .child(
            div()
                .flex_1()
                .v_flex()
                .child(div().text_size(text_px(metrics::BODY)).child(title))
                .child(
                    div()
                        .text_size(text_px(11.0))
                        .line_height(px(14.0))
                        .text_color(mac::text_secondary())
                        .child(subtitle),
                ),
        )
        .child(
            Toggle::new(id)
                .checked(checked)
                .disabled(!enabled)
                .on_click(on_change),
        )
}

fn keycap(glyph: &'static str, role: &'static str, pc: &'static str, command: bool) -> Div {
    div()
        .h(px(44.0))
        .min_w(px(52.0))
        .rounded(px(7.0))
        .bg(rgb(WELL_RULE))
        .when(command, |key| key.border_2().border_color(mac::accent()))
        .v_flex()
        .justify_end()
        .px(px(7.0))
        .py(px(5.0))
        .text_size(text_px(11.0))
        .text_color(mac::text_secondary())
        .child(
            div()
                .text_size(text_px(15.0))
                .text_color(mac::text())
                .child(glyph),
        )
        .child(if pc.is_empty() {
            String::new()
        } else {
            format!("{role} · {pc}")
        })
}

/// One PC key → Mac key pair in the Mac Shortcuts illustration (original
/// drawing; design-lab/setup-assistant.html, Mac Shortcuts).
fn key_mapping(pc: &'static str, mac_glyph: &'static str, mac_name: &'static str) -> Div {
    div()
        .v_flex()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(keycap(pc, "", "", false))
                .child(
                    div()
                        .text_size(text_px(14.0))
                        .text_color(mac::text_secondary())
                        .child("→"),
                )
                .child(keycap(mac_glyph, "", "", false)),
        )
        .child(
            div()
                .text_size(text_px(11.0))
                .text_color(mac::text_secondary())
                .child(mac_name),
        )
}

fn field_label(text: &'static str) -> Div {
    div()
        .w(px(120.0))
        .text_right()
        .text_size(text_px(metrics::BODY))
        .child(text)
}

fn labelled_field(label: &'static str, field: AnyElement) -> Div {
    div()
        .mt(px(14.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(field_label(label))
        .child(div().flex_1().child(field))
}

fn strength_bars(strength: u8, selected: bool) -> Div {
    let lit = 1 + usize::from(strength.min(100)) * 2 / 100;
    let color = if selected {
        mac::on_accent()
    } else {
        mac::text_secondary()
    };
    let mut bars = div().flex().items_end().gap(px(2.0)).h(px(10.0));
    for index in 0..3 {
        bars = bars.child(
            div()
                .w(px(3.0))
                .h(px(4.0 + 3.0 * index as f32))
                .rounded(px(1.0))
                .bg(if index < lit {
                    color
                } else {
                    color.opacity(0.3)
                }),
        );
    }
    bars
}

/// The Appearance page's three choices, in on-screen (and tab/arrow) order.
const APPEARANCE_OPTIONS: [(&str, &str, rmac_theme::SchemePreference); 3] = [
    ("look-light", "Light", rmac_theme::SchemePreference::Light),
    ("look-dark", "Dark", rmac_theme::SchemePreference::Dark),
    ("look-auto", "Auto", rmac_theme::SchemePreference::Automatic),
];

/// Index into [`APPEARANCE_OPTIONS`] that Left/Up, Right/Down, Home or End
/// move to from `current`, matching the ARIA `radiogroup` convention
/// `rmac_ui::RadioGroup` also uses. `None` for any other key.
fn appearance_roving_target(current: usize, key: &str) -> Option<usize> {
    let len = APPEARANCE_OPTIONS.len();
    match key {
        "left" | "up" => Some((current + len - 1) % len),
        "right" | "down" => Some((current + 1) % len),
        "home" => Some(0),
        "end" => Some(len - 1),
        _ => None,
    }
}

fn monogram_face(monogram: &str) -> Div {
    div()
        .size(px(metrics::FACE))
        .rounded_full()
        .bg(rgb(0x979CA6))
        .flex()
        .items_center()
        .justify_center()
        .text_size(text_px(22.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(mac::white())
        .child(monogram.to_owned())
}

fn face_frame(
    id: impl Into<gpui::ElementId>,
    name: impl Into<SharedString>,
    selected: bool,
    picture: AnyElement,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<Div> {
    let id: gpui::ElementId = id.into();
    let focus = window
        .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone();
    let focused = focus.is_focused(window);
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_selected(selected)
        .aria_label(name)
        .p(px(3.0))
        .rounded_full()
        .border_2()
        .border_color(if selected {
            mac::accent()
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .track_focus(&focus.tab_stop(true).tab_index(0))
        .when(focused, |el| el.shadow(mac::focus_ring_shadow()))
        .child(picture)
}

fn tip(icon: &'static str, tint: Hsla, title: &'static str, text: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(14.0))
        .mb(px(18.0))
        .child(
            div()
                .size(px(40.0))
                .flex_none()
                .rounded(px(10.0))
                .bg(tint)
                .flex()
                .items_center()
                .justify_center()
                .child(svg().path(icon).size(px(20.0)).text_color(mac::white())),
        )
        .child(
            div()
                .flex_1()
                .v_flex()
                .child(
                    div()
                        .text_size(text_px(metrics::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(text_px(12.0))
                        .line_height(px(16.0))
                        .text_color(mac::text_secondary())
                        .child(text),
                ),
        )
}

pub const WINDOW_SIZE: (f32, f32) = (metrics::WIDTH, metrics::HEIGHT);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_roving_wraps_at_both_ends() {
        assert_eq!(appearance_roving_target(0, "left"), Some(2));
        assert_eq!(appearance_roving_target(0, "up"), Some(2));
        assert_eq!(appearance_roving_target(2, "right"), Some(0));
        assert_eq!(appearance_roving_target(2, "down"), Some(0));
    }

    #[test]
    fn appearance_roving_steps_by_one_in_the_middle() {
        assert_eq!(appearance_roving_target(0, "right"), Some(1));
        assert_eq!(appearance_roving_target(1, "left"), Some(0));
        assert_eq!(appearance_roving_target(1, "right"), Some(2));
    }

    #[test]
    fn appearance_roving_home_and_end_jump_to_the_edges() {
        assert_eq!(appearance_roving_target(1, "home"), Some(0));
        assert_eq!(appearance_roving_target(1, "end"), Some(2));
    }

    #[test]
    fn appearance_roving_ignores_unrelated_keys() {
        assert_eq!(appearance_roving_target(1, "tab"), None);
        assert_eq!(appearance_roving_target(1, "escape"), None);
        assert_eq!(appearance_roving_target(1, "enter"), None);
    }

    #[test]
    fn appearance_options_cover_every_scheme_preference() {
        assert_eq!(APPEARANCE_OPTIONS.len(), 3);
        assert_eq!(APPEARANCE_OPTIONS[0].2, rmac_theme::SchemePreference::Light);
        assert_eq!(APPEARANCE_OPTIONS[1].2, rmac_theme::SchemePreference::Dark);
        assert_eq!(
            APPEARANCE_OPTIONS[2].2,
            rmac_theme::SchemePreference::Automatic
        );
    }
}
