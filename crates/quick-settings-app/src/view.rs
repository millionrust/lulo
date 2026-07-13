use std::process::Command as ProcessCommand;
use std::time::Duration;

use gpui::{
    AppContext as _, BorrowAppContext as _, Context, Entity, FocusHandle, SharedString, Window,
};
use rmac_quick_settings::{Command, Control, Operation, State};
use rmac_ui::{SliderEvent, SliderState};

use crate::QuickSettingsService;

pub(crate) struct QuickSettingsView {
    pub(crate) state: State,
    pub(crate) volume: Entity<SliderState>,
    pub(crate) stream_error: Option<SharedString>,
    pub(crate) operation_error: Option<SharedString>,
    pub(crate) received_snapshot: bool,
    pub(crate) focus: FocusHandle,
    volume_generation: u64,
    was_active: bool,
}

impl QuickSettingsView {
    pub(crate) fn new(token: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let volume = Self::volume_slider(cx, 0.0);
        let focus = cx.focus_handle();
        focus.focus(window);
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

        Self {
            state: State::default(),
            volume,
            stream_error: None,
            operation_error: None,
            received_snapshot: false,
            focus,
            volume_generation: 0,
            was_active: false,
        }
    }

    fn volume_slider(cx: &mut Context<Self>, value: f32) -> Entity<SliderState> {
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(1.0)
                .default_value(value)
        });
        cx.subscribe(&slider, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event;
            this.schedule_volume(value.start(), cx);
        })
        .detach();
        slider
    }

    fn apply_update(&mut self, update: rmac_shell_runtime::Update, cx: &mut Context<Self>) {
        let before = self.state.view().sound.value;
        self.state.refresh(update.snapshot.quick_settings);
        self.received_snapshot = true;
        self.stream_error = None;
        let sound = self.state.view().sound;
        if !sound.busy && sound.value != before {
            self.volume = Self::volume_slider(cx, f32::from(sound.value.volume));
        }
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
        if control == Control::Sound {
            let sound = self.state.view().sound;
            self.volume = Self::volume_slider(cx, f32::from(sound.value.volume));
        }
        cx.notify();
    }

    fn schedule_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        if !self.state.view().sound.available {
            return;
        }
        self.volume_generation = self.volume_generation.wrapping_add(1);
        let generation = self.volume_generation;
        let volume = volume.round().clamp(0.0, 100.0) as u8;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.volume_generation == generation && !this.state.view().sound.busy {
                    this.execute(Command::SetOutputVolume(volume), cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn dismiss_error(&mut self, control: Control, cx: &mut Context<Self>) {
        if self.state.dismiss_error(control) {
            cx.notify();
        }
    }

    pub(crate) fn dismiss_operation_error(&mut self, cx: &mut Context<Self>) {
        if self.operation_error.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.operation_error = None;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(|| {
                let executable = std::env::current_exe()?.with_file_name("rmac-system-settings");
                ProcessCommand::new(executable).spawn().map(drop)
            })
            .await;
            if result.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.operation_error = Some("Could not open System Settings".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }
}
