use std::process::Command as ProcessCommand;
use std::time::Duration;

use gpui::{BorrowAppContext as _, Context, FocusHandle, SharedString, Window};
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
            volume_generation: 0,
            brightness_generation: 0,
            was_active: false,
        }
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
        if control == Control::Sound && self.dragging != Some(SliderKind::Volume) {
            self.volume_preview = None;
        }
        cx.notify();
    }

    /// Move a slider to `value` percent while it is dragged or clicked.
    pub(crate) fn slide(&mut self, kind: SliderKind, value: u8, cx: &mut Context<Self>) {
        match kind {
            SliderKind::Volume => self.schedule_volume(value, cx),
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

    /// Screenshot: close Control Center first so it is not in the capture,
    /// then take the same full-screen capture as ⇧⌘3.
    pub(crate) fn screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.background_executor()
            .spawn(async {
                blocking::unblock(|| {
                    std::thread::sleep(Duration::from_millis(250));
                    let executable = std::env::current_exe()?.with_file_name("rmac-sound");
                    ProcessCommand::new(executable)
                        .arg("screenshot-screen")
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
