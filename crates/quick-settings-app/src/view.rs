use std::process::Command as ProcessCommand;
use std::time::Duration;

use gpui::{BorrowAppContext as _, Context, FocusHandle, KeyDownEvent, SharedString, Window};
use rmac_quick_settings::detail::{self, Detail, Module, Panel, RowAction, Target};
use rmac_quick_settings::layout::Modules;
use rmac_quick_settings::{Command, Control, Operation, State};

use crate::QuickSettingsService;

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
}

pub(crate) struct QuickSettingsView {
    pub(crate) state: State,
    pub(crate) stream_error: Option<SharedString>,
    pub(crate) operation_error: Option<SharedString>,
    pub(crate) received_snapshot: bool,
    pub(crate) focus: FocusHandle,
    /// Backlight level in percent; `None` hides the Display module.
    pub(crate) brightness: Option<u8>,
    /// The player Now Playing shows; `None` hides the module.
    pub(crate) player: Option<rmac_media::Player>,
    /// Volume shown while a drag or its write is in flight.
    pub(crate) volume_preview: Option<u8>,
    pub(crate) dragging: Option<SliderKind>,
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
    volume_generation: u64,
    brightness_generation: u64,
    was_active: bool,
}

impl QuickSettingsView {
    /// Exact framework-neutral semantics for the future A5/A6 accessibility
    /// adapter. Pinned GPUI cannot publish this snapshot yet.
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

    pub(crate) fn new(token: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();
        cx.on_release(move |_, cx| {
            if cx.has_global::<QuickSettingsService>() {
                cx.update_global::<QuickSettingsService, _>(|service, _| {
                    if service
                        .active
                        .as_ref()
                        .is_some_and(|active| active.token == token)
                    {
                        service.active = None;
                    }
                });
            }
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
            state: State::default(),
            stream_error: None,
            operation_error: None,
            received_snapshot: false,
            focus,
            brightness: None,
            player: None,
            volume_preview: None,
            dragging: None,
            surface_height: rmac_quick_settings::surface::LOGICAL_HEIGHT as f32,
            detail: None,
            others_expanded: false,
            module_focus: None,
            detail_focus: None,
            keyboard: false,
            volume_generation: 0,
            brightness_generation: 0,
            was_active: false,
        }
    }

    /// The open detail view, cut to fit the tallest surface.
    pub(crate) fn panel(&self) -> Option<Panel> {
        let detail = self.detail?;
        let mut panel = detail::panel(detail, self.state.inputs(), self.others_expanded);
        panel.fit(rmac_quick_settings::layout::MAX_SURFACE_HEIGHT as f32);
        Some(panel)
    }

    /// Replace the grid with a module's list. Opening Wi-Fi asks for a fresh
    /// scan; the results arrive through the live Wi-Fi watch.
    pub(crate) fn open_detail(&mut self, detail: Detail, cx: &mut Context<Self>) {
        self.detail = Some(detail);
        self.others_expanded = false;
        self.detail_focus = None;
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
            self.open_detail(detail, cx);
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

    fn nudge_slider(&mut self, kind: SliderKind, up: bool, cx: &mut Context<Self>) {
        let current = match kind {
            SliderKind::Brightness => match self.brightness {
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
                self.dismiss(window, cx);
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
                    self.nudge_slider(SliderKind::DetailVolume, key == "right", cx)
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
            SliderKind::Brightness => self.schedule_brightness(value, cx),
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
        if self.dragging.take().is_some() {
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

    pub(crate) fn dismiss(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}
