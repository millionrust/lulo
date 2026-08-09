//! Launcher query, selection, activation, and system-surface controller.

mod render;
mod surface;

use std::process::Command;
use std::sync::Arc;

use gpui::{px, size, AppContext as _, Context, Entity, Focusable as _, SharedString, Window};
use rmac_launcher::{ActivationMode, ResultId};
use rmac_launcher_runtime::{
    CatalogUpdate, Coordinator, KeyCommand, KeyEffect, Registry, ShortcutEffect,
};
use rmac_launcher_system::{BackendError, FailureKind, Surface, SystemBackend};
use rmac_ui::{InputEvent, InputState};

use crate::service;
use surface::SurfaceBridge;

pub(crate) struct LauncherView {
    token: u64,
    query: Entity<InputState>,
    coordinator: Coordinator,
    registry: Arc<Registry>,
    backend: Arc<SystemBackend<SurfaceBridge>>,
    settings_error: Option<SharedString>,
    was_active: bool,
    compact: bool,
}

pub(crate) struct OverlayEnvironment {
    pub(crate) token: u64,
    pub(crate) event: rmac_shortcuts::Event,
    pub(crate) registry: Arc<Registry>,
    pub(crate) settings: rmac_shell_settings::ShellSettings,
    pub(crate) settings_error: Option<SharedString>,
    pub(crate) clipboard: async_channel::Sender<String>,
}

impl LauncherView {
    /// Exact framework-neutral semantics for the future A5/A6 accessibility
    /// adapter. Pinned GPUI cannot publish this snapshot yet.
    #[allow(dead_code)]
    pub(crate) fn accessibility_snapshot(
        &self,
    ) -> Result<
        rmac_launcher_runtime::accessibility::LauncherAccessibilitySnapshot,
        rmac_launcher_runtime::accessibility::AccessibilityProjectionError,
    > {
        rmac_launcher_runtime::accessibility::project_launcher(
            &self.coordinator.snapshot(),
            rmac_launcher_runtime::accessibility::SurfaceStatus {
                settings_error: self.settings_error.as_ref().map(|error| error.as_ref()),
            },
        )
    }

    pub(crate) fn new(
        environment: OverlayEnvironment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let OverlayEnvironment {
            token,
            event,
            registry,
            settings,
            settings_error,
            clipboard,
        } = environment;
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rmac_launcher_runtime::accessibility::QUERY_NAME)
        });
        let window_handle = window.window_handle();
        cx.subscribe(&query, move |this, query, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = query.read(cx).value().to_string();
                let compact = value.is_empty();
                if this.compact != compact {
                    this.compact = compact;
                    let (width, height) = if compact {
                        (
                            rmac_launcher::surface::LOGICAL_WIDTH as f32,
                            rmac_launcher::surface::LOGICAL_HEIGHT as f32,
                        )
                    } else {
                        (
                            rmac_launcher::surface::EXPANDED_LOGICAL_WIDTH as f32,
                            rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32,
                        )
                    };
                    let _ = cx.update_window(window_handle, |_, window, _| {
                        window.resize(size(px(width), px(height)));
                    });
                }
                if let Some(request) = this.coordinator.set_query(value) {
                    this.dispatch(request, cx);
                }
                cx.notify();
            }
        })
        .detach();
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.was_active = true;
            } else if this.was_active {
                this.dismiss(window, cx);
            }
        })
        .detach();
        cx.on_release(move |_, cx| {
            service::release(token, cx);
        })
        .detach();

        let mut coordinator = Coordinator::new(registry.descriptors(), settings.providers.clone());
        let ShortcutEffect::Open(opened) = coordinator.handle_shortcut(&event) else {
            unreachable!("a fresh launcher surface starts from one launcher activation")
        };
        query.read(cx).focus_handle(cx).focus(window);
        let view = Self {
            token,
            query,
            coordinator,
            registry,
            backend: Arc::new(SystemBackend::new(SurfaceBridge { clipboard })),
            settings_error,
            was_active: false,
            compact: true,
        };
        Self::spawn_dispatch(view.registry.clone(), opened.request, cx);
        view
    }

    fn spawn_dispatch(
        registry: Arc<Registry>,
        request: rmac_launcher::Request,
        cx: &mut Context<Self>,
    ) {
        let capacity = request.providers.len().max(1);
        let (sender, receiver) = async_channel::bounded(capacity);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let dispatch = registry.dispatch(request, sender);
            let consume = async {
                while let Ok(batch) = receiver.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            if this.coordinator.apply(batch) {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            };
            futures_util::join!(dispatch, consume);
        })
        .detach();
    }

    fn dispatch(&self, request: rmac_launcher::Request, cx: &mut Context<Self>) {
        Self::spawn_dispatch(self.registry.clone(), request, cx);
    }

    pub(crate) fn apply_catalog(&mut self, update: CatalogUpdate, cx: &mut Context<Self>) {
        let effect = self.coordinator.apply_catalog(update);
        if let Some(request) = effect.request {
            self.dispatch(request, cx);
        }
        if effect.visible {
            cx.notify();
        }
    }

    pub(crate) fn apply_environment(
        &mut self,
        registry: Arc<Registry>,
        settings: rmac_shell_settings::ShellSettings,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.registry = registry;
        self.settings_error = error;
        if let Some(request) = self
            .coordinator
            .set_environment(self.registry.descriptors(), settings.providers)
        {
            self.dispatch(request, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_settings_error(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.settings_error = Some(message);
        cx.notify();
    }

    pub(crate) fn handle_shortcut(
        &mut self,
        event: &rmac_shortcuts::Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.coordinator.handle_shortcut(event),
            ShortcutEffect::Dismissed
        ) {
            self.dismiss(window, cx);
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.coordinator.handle_key(KeyCommand::Escape);
        service::release(self.token, cx);
        window.remove_window();
    }

    fn handle_key(&mut self, command: KeyCommand, window: &mut Window, cx: &mut Context<Self>) {
        let effect = self.coordinator.handle_key(command);
        self.apply_key_effect(effect, window, cx);
    }

    fn select_and_activate(
        &mut self,
        id: ResultId,
        mode: ActivationMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.coordinator.select(&id);
        let effect = self.coordinator.activate_selected(mode);
        self.apply_key_effect(effect, window, cx);
    }

    fn apply_key_effect(&mut self, effect: KeyEffect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            KeyEffect::None => {}
            KeyEffect::SelectionChanged => cx.notify(),
            KeyEffect::Dismissed => self.dismiss(window, cx),
            KeyEffect::Activate(activation) => {
                let backend = self.backend.clone();
                let window_handle = window.window_handle();
                cx.notify();
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    let result = rmac_launcher_system::execute(
                        activation.id,
                        &activation.action,
                        backend.as_ref(),
                    )
                    .await;
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        let _ = this.update(cx, |this, cx| {
                            if this.coordinator.finish_activation(result) {
                                if !this.coordinator.snapshot().open {
                                    service::release(this.token, cx);
                                    window.remove_window();
                                } else {
                                    cx.notify();
                                }
                            }
                        });
                    });
                })
                .detach();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SurfaceBridge;
    use rmac_launcher_system::{FailureKind, Surface as _};

    #[test]
    fn settings_surface_rejects_hidden_destinations() {
        let (sender, _receiver) = async_channel::bounded(1);
        let surface = SurfaceBridge { clipboard: sender };
        assert_eq!(
            surface.open_setting("assistant").unwrap_err().kind,
            FailureKind::InvalidAction
        );
        assert_eq!(
            surface.open_setting("screen-time").unwrap_err().kind,
            FailureKind::InvalidAction
        );
    }
}
