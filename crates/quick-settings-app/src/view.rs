use std::process::Command as ProcessCommand;
use std::time::{Duration, Instant};

use gpui::{Context, FocusHandle, KeyDownEvent, SharedString, Window};
use rmac_quick_settings::detail::{self, Detail, Module, Panel, RowAction, Target};
use rmac_quick_settings::layout::Modules;
use rmac_quick_settings::{Command, Control, Operation, State};
use rmac_ui::SliderBulge;

/// How often Now Playing re-reads the active MPRIS player while open.
const MEDIA_POLL: Duration = Duration::from_millis(1000);
/// Coalesce slider drags into one system write per pause.
const SLIDER_SETTLE: Duration = Duration::from_millis(120);

/// A Control Center slider being dragged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SliderKind {
    Brightness,
    Volume,
    /// The volume slider at the top of the Sound detail view.
    DetailVolume,
    /// The brightness slider at the top of the Display detail view.
    DetailBrightness,
}

impl SliderKind {
    const ALL: [Self; 4] = [
        Self::Brightness,
        Self::Volume,
        Self::DetailVolume,
        Self::DetailBrightness,
    ];

    fn index(self) -> usize {
        match self {
            Self::Brightness => 0,
            Self::Volume => 1,
            Self::DetailVolume => 2,
            Self::DetailBrightness => 3,
        }
    }
}

/// Two "bulges" per slider (CC-13). `hover` thickens the track and shows
/// the knob while the pointer is over the slider or it is held; `press`
/// springs the whole module a little larger while it is held, as macOS 26
/// does, and settles back on release. Each runs only while it changes, so
/// an idle Control Centre draws no frames.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SliderBulges {
    hover: [SliderBulge; 4],
    press: [SliderBulge; 4],
}

impl Default for SliderBulges {
    fn default() -> Self {
        Self {
            hover: [SliderBulge::default(); 4],
            press: [SliderBulge::spring(); 4],
        }
    }
}

impl SliderBulges {
    fn get(&self, kind: SliderKind) -> SliderBulge {
        self.hover[kind.index()]
    }

    fn get_mut(&mut self, kind: SliderKind) -> &mut SliderBulge {
        &mut self.hover[kind.index()]
    }

    fn press(&self, kind: SliderKind) -> SliderBulge {
        self.press[kind.index()]
    }

    fn press_mut(&mut self, kind: SliderKind) -> &mut SliderBulge {
        &mut self.press[kind.index()]
    }

    fn is_animating(&self, now_ms: u64) -> bool {
        SliderKind::ALL.iter().any(|kind| {
            self.get(*kind).is_animating(now_ms) || self.press(*kind).is_animating(now_ms)
        })
    }
}

pub(crate) struct QuickSettingsView {
    pub(crate) token: u64,
    previous_window: Option<rmac_compositor::WindowId>,
    pub(crate) state: State,
    pub(crate) stream_error: Option<SharedString>,
    pub(crate) operation_error: Option<SharedString>,
    pub(crate) received_snapshot: bool,
    pub(crate) focus: FocusHandle,
    pub(crate) initial_control_focus: FocusHandle,
    /// Real Tab stops for the Display and Sound sliders. `key_down` reads
    /// these directly (`is_focused`) so Left/Right nudge whichever slider
    /// Tab actually landed on, independent of the module-grid's own
    /// mouse/arrow roving focus (`module_focus`), which Tab never updates.
    pub(crate) display_slider_focus: FocusHandle,
    pub(crate) sound_slider_focus: FocusHandle,
    /// Real Tab stops for the Wi-Fi, Bluetooth and Focus toggle circles,
    /// read the same way by `key_down` for Space/Enter.
    pub(crate) wifi_toggle_focus: FocusHandle,
    pub(crate) bluetooth_toggle_focus: FocusHandle,
    pub(crate) focus_toggle_focus: FocusHandle,
    /// Real Tab stops for the Display and Sound module titles, which open
    /// their detail views.
    pub(crate) display_title_focus: FocusHandle,
    pub(crate) sound_title_focus: FocusHandle,
    /// Dark Mode as just chosen in the Display view, shown until the theme
    /// store's change arrives.
    pub(crate) dark_mode_override: Option<bool>,
    /// The Display or Sound module the pointer is over: its title shows
    /// the › that opens its detail view.
    pub(crate) hovered_module: Option<Module>,
    /// Backlight level in percent; `None` hides the Display module.
    pub(crate) brightness: Option<u8>,
    /// The player Now Playing shows; `None` hides the module.
    pub(crate) player: Option<rmac_media::Player>,
    /// Volume shown while a drag or its write is in flight.
    pub(crate) volume_preview: Option<u8>,
    pub(crate) dragging: Option<SliderKind>,
    /// The slider the pointer is currently over, independent of `dragging`
    /// (a drag can continue once the pointer strays off the hit rect).
    pub(crate) hovered_slider: Option<SliderKind>,
    /// Pointer/touch "bulge" progress per slider (CC-13).
    pub(crate) slider_bulges: SliderBulges,
    /// Monotonic clock epoch for slider-bulge animation timing.
    epoch: Instant,
    /// Logical height the layer surface was last sized to.
    pub(crate) surface_height: f32,
    /// The Wi-Fi, Bluetooth or Sound list shown in place of the grid.
    pub(crate) detail: Option<Detail>,
    /// Wi-Fi's "Other Networks" disclosure is open.
    pub(crate) others_expanded: bool,
    /// Keyboard focus in the grid and in a detail view. Rings show only
    /// after a key was pressed, as on the Mac.
    pub(crate) module_focus: Option<Module>,
    pub(crate) detail_focus: Option<Target>,
    pub(crate) keyboard: bool,
    /// Request one follow-up frame when AccessKit attaches after first paint.
    pub(crate) a11y_active_last_frame: bool,
    volume_generation: u64,
    brightness_generation: u64,
}

impl QuickSettingsView {
    /// Framework-neutral accessibility contract; render publishes the
    /// corresponding controls through GPUI's AccessKit nodes.
    #[allow(dead_code)]
    pub(crate) fn accessibility_snapshot(
        &self,
    ) -> Result<
        rmac_quick_settings::accessibility::QuickSettingsAccessibilitySnapshot,
        rmac_quick_settings::accessibility::AccessibilityProjectionError,
    > {
        rmac_quick_settings::accessibility::project_quick_settings(
            &self.state.view(),
            rmac_quick_settings::accessibility::SurfaceStatus {
                received_snapshot: self.received_snapshot,
                stream_error: self.stream_error.as_ref().map(|error| error.as_ref()),
                operation_error: self.operation_error.as_ref().map(|error| error.as_ref()),
            },
        )
    }

    pub(crate) fn new(
        token: u64,
        previous_window: Option<rmac_compositor::WindowId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let initial_control_focus = cx.focus_handle();
        let display_slider_focus = cx.focus_handle();
        let sound_slider_focus = cx.focus_handle();
        let wifi_toggle_focus = cx.focus_handle();
        let bluetooth_toggle_focus = cx.focus_handle();
        let focus_toggle_focus = cx.focus_handle();
        let display_title_focus = cx.focus_handle();
        let sound_title_focus = cx.focus_handle();
        focus.focus(window, cx);
        let first_control = initial_control_focus.clone();
        window.on_next_frame(move |window, cx| window.focus(&first_control, cx));
        // The outside catcher handles pointer dismissal. Compositor focus can
        // move back to an open menu-bar menu while this panel remains visible.
        // Windows has no catcher: a press elsewhere takes the foreground away,
        // which closes the panel.
        #[cfg(windows)]
        {
            let mut was_active = false;
            cx.observe_window_activation(window, move |this, window, cx| {
                if window.is_window_active() {
                    was_active = true;
                } else if was_active {
                    this.dismiss(window, cx);
                }
            })
            .detach();
        }
        cx.on_release(move |_, cx| {
            crate::clear_active_popover(token, cx);
        })
        .detach();

        let (snapshot_tx, snapshot_rx) = async_channel::bounded(4);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let watch = rmac_shell_runtime::watch(snapshot_tx);
            let consume = async {
                while let Ok(update) = snapshot_rx.recv().await {
                    if this
                        .update(cx, |this, cx| this.apply_update(update, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            };
            let (result, ()) = futures_util::join!(watch, consume);
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.stream_error = Some(
                        "Live system updates stopped; showing the last received values".into(),
                    );
                    cx.notify();
                });
            }
        })
        .detach();

        // The Display module exists only when a backlight can be read.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let level = blocking::unblock(rmac_osd::brightness).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.brightness = level;
                cx.notify();
            });
        })
        .detach();

        // Now Playing follows the active MPRIS player while the panel is open.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            let player = blocking::unblock(|| rmac_media::active_player().ok().flatten()).await;
            if this
                .update(cx, |this, cx| {
                    if this.player != player {
                        this.player = player;
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(MEDIA_POLL).await;
        })
        .detach();

        Self {
            token,
            previous_window,
            state: State::default(),
            stream_error: None,
            operation_error: None,
            received_snapshot: false,
            focus,
            initial_control_focus,
            display_slider_focus,
            sound_slider_focus,
            wifi_toggle_focus,
            bluetooth_toggle_focus,
            focus_toggle_focus,
            display_title_focus,
            sound_title_focus,
            dark_mode_override: None,
            hovered_module: None,
            brightness: None,
            player: None,
            volume_preview: None,
            dragging: None,
            hovered_slider: None,
            slider_bulges: SliderBulges::default(),
            epoch: Instant::now(),
            surface_height: rmac_quick_settings::surface::LOGICAL_HEIGHT as f32,
            detail: None,
            others_expanded: false,
            module_focus: None,
            detail_focus: None,
            keyboard: false,
            a11y_active_last_frame: false,
            volume_generation: 0,
            brightness_generation: 0,
        }
    }

    /// The open detail view, cut to fit the tallest surface.
    pub(crate) fn panel(&self) -> Option<Panel> {
        let detail = self.detail?;
        let mut panel = detail::panel(
            detail,
            self.state.inputs(),
            self.others_expanded,
            Some(self.dark_mode()),
        );
        panel.fit(rmac_quick_settings::layout::MAX_SURFACE_HEIGHT as f32);
        Some(panel)
    }

    /// Replace the grid with a module's list. Opening Wi-Fi asks for a fresh
    /// scan; the results arrive through the live Wi-Fi watch.
    ///
    /// The control that opened the list (a pill, the Sound Outputs button)
    /// is not drawn any more, and GPUI sends keys for an undrawn focus to
    /// the window root, past the panel's own key handling: Esc, arrows and
    /// Return did nothing in the list. The panel takes keyboard focus
    /// instead; `detail_focus` tracks the highlighted row.
    pub(crate) fn open_detail(
        &mut self,
        detail: Detail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        self.detail = Some(detail);
        self.others_expanded = false;
        self.hovered_module = None;
        // From the keyboard, the first control takes the focus ring, so the
        // next key acts inside the view, as on the Mac.
        self.detail_focus = if self.keyboard {
            self.panel()
                .and_then(|panel| panel.targets().first().copied())
        } else {
            None
        };
        if detail == Detail::Wifi && self.state.view().wifi.value {
            cx.background_executor()
                .spawn(async {
                    if let Err(error) =
                        blocking::unblock(rmac_quick_settings_system::request_wifi_scan).await
                    {
                        eprintln!("Control Centre could not scan for Wi-Fi networks: {error}");
                    }
                })
                .detach();
        }
        cx.notify();
    }

    /// Back to the grid, keeping keyboard focus on the module.
    pub(crate) fn close_detail(&mut self, cx: &mut Context<Self>) {
        if let Some(detail) = self.detail.take() {
            self.module_focus = Some(match detail {
                Detail::Wifi => Module::Wifi,
                Detail::Bluetooth => Module::Bluetooth,
                Detail::Display => Module::Display,
                Detail::Sound => Module::Sound,
            });
            self.detail_focus = None;
            cx.notify();
        }
    }

    pub(crate) fn toggle_others(&mut self, cx: &mut Context<Self>) {
        self.others_expanded = !self.others_expanded;
        cx.notify();
    }

    /// The title switch of the Wi-Fi or Bluetooth list.
    pub(crate) fn toggle_detail_switch(&mut self, cx: &mut Context<Self>) {
        let view = self.state.view();
        match self.detail {
            Some(Detail::Wifi) if view.wifi.available && !view.wifi.busy => {
                self.execute(Command::SetWifiEnabled(!view.wifi.value), cx)
            }
            Some(Detail::Bluetooth) if view.bluetooth.available && !view.bluetooth.busy => {
                self.execute(Command::SetBluetoothPowered(!view.bluetooth.value), cx)
            }
            _ => {}
        }
    }

    pub(crate) fn run_row(
        &mut self,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            // execute() refuses, visibly, while the control is changing.
            RowAction::Run(command) => self.execute(command, cx),
            RowAction::OpenSettings(pane) => self.open_settings(Some(pane), window, cx),
        }
    }

    /// Whether `module` shows the keyboard focus ring.
    pub(crate) fn ring(&self, module: Module) -> bool {
        self.keyboard && self.detail.is_none() && self.module_focus == Some(module)
    }

    /// The grid's keyboard order, as the modules are laid out.
    pub(crate) fn module_order(&self) -> Vec<Module> {
        let view = self.state.view();
        let mut order = vec![Module::Wifi, Module::Bluetooth];
        if rmac_quick_settings::layout::low_power_available(&view.power.value)
            && view.power.available
        {
            order.push(Module::LowPower);
        }
        order.extend([Module::Screenshot, Module::Focus]);
        if self.brightness.is_some() {
            order.push(Module::Display);
        }
        order.push(Module::Sound);
        order
    }

    fn activate_module(&mut self, module: Module, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(detail) = module.detail() {
            self.open_detail(detail, window, cx);
            return;
        }
        match module {
            Module::LowPower => self.toggle_low_power(cx),
            Module::Screenshot => self.screenshot(window, cx),
            Module::Focus => {
                let focus = self.state.view().focus;
                if focus.available && !focus.busy {
                    self.execute(Command::SetFocusEnabled(!focus.value.enabled), cx);
                }
            }
            _ => {}
        }
    }

    /// Space/Enter on the focused Wi-Fi toggle circle: flips the switch,
    /// the same command the pointer and AT-SPI Click paths already run.
    fn toggle_wifi(&mut self, cx: &mut Context<Self>) {
        let wifi = self.state.view().wifi;
        if wifi.available && !wifi.busy {
            self.execute(Command::SetWifiEnabled(!wifi.value), cx);
        }
    }

    fn toggle_bluetooth(&mut self, cx: &mut Context<Self>) {
        let bluetooth = self.state.view().bluetooth;
        if bluetooth.available && !bluetooth.busy {
            self.execute(Command::SetBluetoothPowered(!bluetooth.value), cx);
        }
    }

    fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        let focus = self.state.view().focus;
        if focus.available && !focus.busy {
            self.execute(Command::SetFocusEnabled(!focus.value.enabled), cx);
        }
    }

    /// Whether Lulo is in Dark Mode, including a choice still being saved.
    pub(crate) fn dark_mode(&self) -> bool {
        self.dark_mode_override
            .unwrap_or_else(rmac_ui::mac::is_dark)
    }

    /// The Display view's Dark Mode toggle: save Light or Dark the way
    /// System Settings ▸ Appearance does (leaving Auto, as the Mac's toggle
    /// does), then tell third-party toolkits. The store write and the
    /// toolkit sync run off the UI thread; every Lulo surface, this one
    /// included, repaints from the theme store's change notification.
    pub(crate) fn toggle_dark_mode(&mut self, cx: &mut Context<Self>) {
        let dark = !self.dark_mode();
        self.dark_mode_override = Some(dark);
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let host = cx
                .background_executor()
                .spawn(rmac_appearance_portal::snapshot())
                .await
                .unwrap_or_else(|_| {
                    rmac_appearance::Snapshot::unavailable(
                        "The desktop Settings portal is temporarily unavailable.",
                    )
                });
            let result = blocking::unblock(move || save_color_scheme(dark, &host)).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.dark_mode_override = None;
                    this.operation_error = Some(format!("Dark Mode: {error}").into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn nudge_slider(&mut self, kind: SliderKind, up: bool, cx: &mut Context<Self>) {
        let current = match kind {
            SliderKind::Brightness | SliderKind::DetailBrightness => match self.brightness {
                Some(level) => level,
                None => return,
            },
            SliderKind::Volume | SliderKind::DetailVolume => self
                .volume_preview
                .unwrap_or(self.state.view().sound.value.volume),
        };
        self.slide(kind, detail::nudge(current, up), cx);
    }

    pub(crate) fn activate_detail_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.panel() else {
            return;
        };
        match target {
            Target::Switch => self.toggle_detail_switch(cx),
            Target::Notice => self.open_settings(Some("wifi"), window, cx),
            Target::Slider => {}
            Target::Toggle(index) => match panel.toggles.get(index).map(|toggle| toggle.kind) {
                Some(detail::DisplayToggle::DarkMode) => self.toggle_dark_mode(cx),
                None => {}
            },
            Target::Row { .. } | Target::Other(_) => {
                if let Some(action) = panel.row(target).and_then(|row| row.action.clone()) {
                    self.run_row(action, window, cx);
                }
            }
            Target::Disclosure => self.toggle_others(cx),
            Target::Settings => {
                let (_, pane) = panel.detail.settings();
                self.open_settings(Some(pane), window, cx);
            }
        }
    }

    /// Arrow keys, Tab, Return, Space and Esc inside Control Centre.
    pub(crate) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        if key == "escape" {
            if self.detail.is_some() {
                self.close_detail(cx);
            } else {
                self.dismiss_and_restore_focus(window, cx);
            }
            return true;
        }
        if !matches!(
            key,
            "up" | "down" | "left" | "right" | "tab" | "enter" | "space"
        ) {
            return false;
        }
        self.keyboard = true;
        if let Some(panel) = self.panel() {
            let targets = panel.targets();
            let current = self.detail_focus.filter(|target| targets.contains(target));
            match (key, current) {
                ("left" | "right", Some(Target::Slider)) => {
                    let kind = if panel.detail == Detail::Display {
                        SliderKind::DetailBrightness
                    } else {
                        SliderKind::DetailVolume
                    };
                    self.nudge_slider(kind, key == "right", cx)
                }
                ("up", _) => self.detail_focus = detail::step(&targets, current, false),
                ("tab", _) if shift => self.detail_focus = detail::step(&targets, current, false),
                ("down" | "tab", _) => {
                    self.detail_focus = detail::step(&targets, current, true);
                }
                ("left", _) => self.close_detail(cx),
                ("enter" | "space", Some(target)) => {
                    self.activate_detail_target(target, window, cx)
                }
                _ => {}
            }
        } else if matches!(key, "left" | "right") && self.display_slider_focus.is_focused(window) {
            // Real Tab focus is on the Display slider: nudge it directly,
            // regardless of the module grid's own mouse/arrow roving state
            // (`module_focus`, below), which Tab never updates. Sync
            // `module_focus` too, so the focus ring (`ring()`) matches.
            self.module_focus = Some(Module::Display);
            self.nudge_slider(SliderKind::Brightness, key == "right", cx);
        } else if matches!(key, "left" | "right") && self.sound_slider_focus.is_focused(window) {
            self.module_focus = Some(Module::Sound);
            self.nudge_slider(SliderKind::Volume, key == "right", cx);
        } else if matches!(key, "enter" | "space") && self.wifi_toggle_focus.is_focused(window) {
            // Real Tab focus is on the Wi-Fi toggle circle itself, not the
            // "Wi-Fi details" label `activate_module` below would open:
            // Space/Enter here flips the switch, as on the Mac.
            self.toggle_wifi(cx);
        } else if matches!(key, "enter" | "space") && self.bluetooth_toggle_focus.is_focused(window)
        {
            self.toggle_bluetooth(cx);
        } else if matches!(key, "enter" | "space") && self.focus_toggle_focus.is_focused(window) {
            self.toggle_focus_mode(cx);
        } else if matches!(key, "enter" | "space") && self.display_title_focus.is_focused(window) {
            self.open_detail(Detail::Display, window, cx);
        } else if matches!(key, "enter" | "space") && self.sound_title_focus.is_focused(window) {
            self.open_detail(Detail::Sound, window, cx);
        } else {
            let order = self.module_order();
            let current = self.module_focus.filter(|module| order.contains(module));
            match (key, current) {
                ("left" | "right", Some(module)) if module.is_slider() => {
                    let kind = if module == Module::Display {
                        SliderKind::Brightness
                    } else {
                        SliderKind::Volume
                    };
                    self.nudge_slider(kind, key == "right", cx);
                }
                ("up" | "left", _) => self.module_focus = detail::step(&order, current, false),
                ("tab", _) if shift => self.module_focus = detail::step(&order, current, false),
                ("down" | "right" | "tab", _) => {
                    self.module_focus = detail::step(&order, current, true)
                }
                ("enter" | "space", Some(module)) => self.activate_module(module, window, cx),
                _ => {}
            }
        }
        cx.notify();
        true
    }

    /// Error banners shown above the modules, newest concerns first.
    pub(crate) fn banners(&self) -> Vec<(Option<Control>, SharedString)> {
        let view = self.state.view();
        let mut banners = Vec::new();
        if let Some(error) = &self.stream_error {
            banners.push((None, error.clone()));
        }
        if let Some(error) = &self.operation_error {
            banners.push((None, error.clone()));
        }
        let tiles = [
            (Control::Wifi, view.wifi.error),
            (Control::Bluetooth, view.bluetooth.error),
            (Control::Focus, view.focus.error),
            (Control::Power, view.power.error),
            (Control::Sound, view.sound.error),
        ];
        for (control, error) in tiles {
            if let Some(error) = error {
                banners.push((Some(control), error.into()));
            }
        }
        banners
    }

    pub(crate) fn modules(&self) -> Modules {
        Modules {
            now_playing: self.player.is_some(),
            display: self.brightness.is_some(),
            banners: self.banners().len(),
        }
    }

    fn apply_update(&mut self, update: rmac_shell_runtime::Update, cx: &mut Context<Self>) {
        self.state.refresh(update.snapshot.quick_settings);
        self.received_snapshot = true;
        self.stream_error = None;
        cx.notify();
    }

    pub(crate) fn execute(&mut self, command: Command, cx: &mut Context<Self>) {
        let operation = match self.state.begin(command) {
            Ok(operation) => operation,
            Err(error) => {
                self.operation_error = Some(error.to_string().into());
                cx.notify();
                return;
            }
        };
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (operation, result) = blocking::unblock(move || {
                let result = rmac_quick_settings_system::execute(
                    &operation,
                    &rmac_quick_settings_system::SystemBackend,
                );
                (operation, result)
            })
            .await;
            let _ = this.update(cx, |this, cx| this.finish(operation, result, cx));
        })
        .detach();
    }

    fn finish(
        &mut self,
        operation: Operation,
        result: Result<rmac_quick_settings::Inputs, rmac_quick_settings_system::Error>,
        cx: &mut Context<Self>,
    ) {
        let control = operation.command.control();
        match result {
            Ok(inputs) => {
                self.state.complete(&operation, inputs);
                self.operation_error = None;
            }
            Err(error) => {
                self.state.fail(&operation, error.to_string());
            }
        }
        if control == Control::Sound
            && !matches!(
                self.dragging,
                Some(SliderKind::Volume | SliderKind::DetailVolume)
            )
        {
            self.volume_preview = None;
        }
        cx.notify();
    }

    /// Move a slider to `value` percent while it is dragged or clicked.
    pub(crate) fn slide(&mut self, kind: SliderKind, value: u8, cx: &mut Context<Self>) {
        match kind {
            SliderKind::Volume | SliderKind::DetailVolume => self.schedule_volume(value, cx),
            SliderKind::Brightness | SliderKind::DetailBrightness => {
                self.schedule_brightness(value, cx)
            }
        }
    }

    fn schedule_volume(&mut self, volume: u8, cx: &mut Context<Self>) {
        if !self.state.view().sound.available {
            return;
        }
        self.volume_preview = Some(volume);
        cx.notify();
        self.volume_generation = self.volume_generation.wrapping_add(1);
        let generation = self.volume_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(SLIDER_SETTLE).await;
            let _ = this.update(cx, |this, cx| {
                if this.volume_generation == generation && !this.state.view().sound.busy {
                    this.execute(Command::SetOutputVolume(volume), cx);
                }
            });
        })
        .detach();
    }

    fn schedule_brightness(&mut self, level: u8, cx: &mut Context<Self>) {
        if self.brightness.is_none() {
            return;
        }
        self.brightness = Some(level);
        cx.notify();
        self.brightness_generation = self.brightness_generation.wrapping_add(1);
        let generation = self.brightness_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(SLIDER_SETTLE).await;
            let current = this
                .update(cx, |this, _| this.brightness_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let result = blocking::unblock(move || rmac_osd::set_brightness(level)).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(level) if this.brightness_generation == generation => {
                        this.brightness = Some(level);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        this.operation_error = Some(format!("Display: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn end_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(kind) = self.dragging.take() {
            self.sync_slider_bulge(kind, cx);
            cx.notify();
        }
    }

    /// Milliseconds since the view was created, for slider-bulge timing
    /// (CC-13). A monotonic clock, not wall time, so it never jumps.
    pub(crate) fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Whether any slider's bulge is still easing toward its target. The
    /// renderer requests another frame while this is true, and stops once
    /// it is false — idle CPU returns to ~0 once every bulge has settled.
    pub(crate) fn sliders_are_animating(&self) -> bool {
        self.slider_bulges.is_animating(self.now_ms())
    }

    /// Current eased bulge progress for `kind`'s slider, 0 (resting) ..= 1
    /// (fully bulged), for the renderer to size the track with.
    pub(crate) fn slider_bulge(&self, kind: SliderKind) -> f32 {
        self.slider_bulges.get(kind).progress(self.now_ms())
    }

    /// Spring progress of `kind`'s press, 0 (released) to 1 (held),
    /// briefly past 1 as it grows: the module's grow factor.
    pub(crate) fn slider_press(&self, kind: SliderKind) -> f32 {
        self.slider_bulges.press(kind).progress(self.now_ms())
    }

    /// A slider is "active" — bulged, per CC-13 — while the pointer is over
    /// it or it is being dragged (a drag can continue once the pointer
    /// strays off the hit rect; a touch press never hovers at all, but
    /// pressing down already sets `dragging`).
    fn is_slider_active(&self, kind: SliderKind) -> bool {
        self.hovered_slider == Some(kind) || self.dragging == Some(kind)
    }

    /// Updates a slider's bulge target from its current hover/drag state
    /// and schedules a render to start the transition. This runs from
    /// pointer and accessibility event handlers, outside render, where
    /// `Window::request_animation_frame` panics (it needs the view being
    /// rendered). The render itself keeps requesting frames while
    /// `sliders_are_animating`, so idle CPU returns to ~0 once the bulge
    /// settles.
    pub(crate) fn sync_slider_bulge(&mut self, kind: SliderKind, cx: &mut Context<Self>) {
        let now = self.now_ms();
        let active = self.is_slider_active(kind);
        self.slider_bulges.get_mut(kind).set_active(active, now);
        let pressed = self.dragging == Some(kind);
        self.slider_bulges.press_mut(kind).set_active(pressed, now);
        if self.slider_bulges.get(kind).is_animating(now)
            || self.slider_bulges.press(kind).is_animating(now)
        {
            cx.notify();
        }
    }

    /// Pointer entered or left a slider's hit rect (CC-13).
    pub(crate) fn set_slider_hovered(
        &mut self,
        kind: SliderKind,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.hovered_slider = if hovered {
            Some(kind)
        } else if self.hovered_slider == Some(kind) {
            None
        } else {
            self.hovered_slider
        };
        self.sync_slider_bulge(kind, cx);
        cx.notify();
    }

    /// Pointer entered or left the Display or Sound module.
    pub(crate) fn set_module_hovered(
        &mut self,
        module: Module,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(module)
        } else if self.hovered_module == Some(module) {
            None
        } else {
            self.hovered_module
        };
        if next != self.hovered_module {
            self.hovered_module = next;
            cx.notify();
        }
    }

    pub(crate) fn toggle_low_power(&mut self, cx: &mut Context<Self>) {
        let power = self.state.view().power;
        if power.busy {
            return;
        }
        if let Some(command) = rmac_quick_settings::layout::low_power_toggle(&power.value) {
            self.execute(command, cx);
        }
    }

    pub(crate) fn media(&mut self, command: rmac_media::Command, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || rmac_media::send(command)).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(player) => this.player = Some(player),
                    Err(error) => this.operation_error = Some(error.to_string().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Screenshot: close Control Center first, then open the ⇧⌘5 screenshot
    /// toolbar, as the Mac's Screenshot control does.
    pub(crate) fn screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.background_executor()
            .spawn(async {
                blocking::unblock(|| {
                    std::thread::sleep(Duration::from_millis(250));
                    let executable = std::env::current_exe()?.with_file_name("rmac-screenshot");
                    ProcessCommand::new(executable)
                        .arg("toolbar")
                        .spawn()
                        .map(drop)
                })
                .await
            })
            .detach();
        self.dismiss(window, cx);
    }

    pub(crate) fn dismiss_error(&mut self, control: Option<Control>, cx: &mut Context<Self>) {
        let dismissed = match control {
            Some(control) => self.state.dismiss_error(control),
            None => self.stream_error.take().is_some() || self.operation_error.take().is_some(),
        };
        if dismissed {
            cx.notify();
        }
    }

    /// Open System Settings, at `pane` when given (see its `--pane` routes).
    pub(crate) fn open_settings(
        &mut self,
        pane: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.operation_error = None;
        cx.background_executor()
            .spawn(async move {
                blocking::unblock(move || {
                    let executable =
                        std::env::current_exe()?.with_file_name("rmac-system-settings");
                    let mut command = ProcessCommand::new(executable);
                    if let Some(pane) = pane {
                        command.arg("--pane").arg(pane);
                    }
                    command.spawn().map(drop)
                })
                .await
            })
            .detach();
        self.dismiss(window, cx);
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::clear_active_popover(self.token, cx);
        window.remove_window();
    }

    pub(crate) fn dismiss_and_restore_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss(window, cx);
        if let Some(previous_window) = self.previous_window {
            cx.spawn(async move |_, _| {
                let action = rmac_compositor::Action::FocusWindow {
                    window: previous_window,
                };
                if let Err(error) = rmac_compositor_system::execute_action(&action).await {
                    eprintln!("could not return focus from Control Centre: {error:?}");
                }
            })
            .detach();
        }
    }
}

/// Save Light or Dark the way System Settings ▸ Appearance does, then tell
/// third-party toolkits. Runs on the blocking pool.
fn save_color_scheme(dark: bool, host: &rmac_appearance::Snapshot) -> Result<(), String> {
    let store = rmac_theme::ThemeStore::from_environment()
        .map_err(|_| "the appearance preferences are unavailable".to_owned())?;
    let mut preferences = store
        .load(host)
        .map_err(|_| "the appearance preferences could not be read".to_owned())?
        .preferences;
    preferences.color_scheme = if dark {
        rmac_theme::SchemePreference::Dark
    } else {
        rmac_theme::SchemePreference::Light
    };
    store
        .save(&preferences, host)
        .map_err(|_| "the appearance preference could not be saved".to_owned())?;
    #[cfg(target_os = "linux")]
    if let Err(error) = rmac_gtk_settings::sync_toolkit_appearance(&preferences) {
        eprintln!("Control Centre could not update toolkit appearance: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_ui::SLIDER_BULGE_MS;

    #[test]
    fn slider_bulges_track_each_slider_independently() {
        let mut bulges = SliderBulges::default();
        bulges.get_mut(SliderKind::Brightness).set_active(true, 0);
        assert!(bulges.get(SliderKind::Brightness).progress(0) >= 0.0);
        assert_eq!(bulges.get(SliderKind::Volume).progress(0), 0.0);
        assert_eq!(bulges.get(SliderKind::DetailVolume).progress(0), 0.0);
        assert_eq!(bulges.get(SliderKind::DetailBrightness).progress(0), 0.0);
        assert!(bulges.is_animating(0));
        assert!(!bulges.is_animating(SLIDER_BULGE_MS));
        // A press springs the module only while it settles, then stops.
        bulges.press_mut(SliderKind::Volume).set_active(true, 1_000);
        assert!(bulges.is_animating(1_000));
        assert!(!bulges.is_animating(1_000 + rmac_ui::SLIDER_PRESS_MS));
        assert_eq!(
            bulges
                .press(SliderKind::Volume)
                .progress(1_000 + rmac_ui::SLIDER_PRESS_MS),
            1.0
        );
    }

    #[test]
    fn slider_bulges_default_to_resting_and_not_animating() {
        let bulges = SliderBulges::default();
        assert_eq!(bulges.get(SliderKind::Brightness).progress(0), 0.0);
        assert!(!bulges.is_animating(0));
    }
}
