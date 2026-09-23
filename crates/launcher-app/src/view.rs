//! Launcher query, selection, activation, and system-surface controller.

mod actions;
mod completion;
mod panel;
mod render;
mod surface;

use std::process::Command;
use std::sync::Arc;

use gpui::{px, size, AppContext as _, Context, Entity, Focusable as _, SharedString, Window};
use rmac_launcher::{ActivationMode, ApplicationGroup, Category, MoveSelection, ResultId};
use rmac_launcher_runtime::{
    CatalogUpdate, Coordinator, KeyCommand, KeyEffect, Registry, Row, ShortcutEffect,
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
    browse_mode: Option<BrowseMode>,
    application_group: Option<ApplicationGroup>,
    application_view: ApplicationView,
    application_options_open: bool,
    /// The Actions (⌘3) or Clipboard (⌘4) panel, replacing bar and results.
    panel: Option<panel::Panel>,
    /// The quick-action circles show only while the pointer is over the bar.
    bar_hovered: bool,
    applications: rmac_launcher_providers::ApplicationProvider,
    /// Set by a left press on one of the drawn shapes before the press
    /// bubbles to the transparent surface, which dismisses on its own.
    press_inside: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationView {
    Grid,
    List,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BrowseMode {
    Applications,
    Files,
}

impl BrowseMode {
    pub(crate) fn category(self) -> Category {
        match self {
            Self::Applications => Category::Applications,
            Self::Files => Category::Files,
        }
    }
}

fn requested_browse_mode(event: &rmac_shortcuts::Event) -> Option<BrowseMode> {
    match event {
        rmac_shortcuts::Event::Activated { id, .. } if id.0 == "app-drawer" => {
            Some(BrowseMode::Applications)
        }
        _ => None,
    }
}

pub(crate) struct OverlayEnvironment {
    pub(crate) token: u64,
    pub(crate) event: rmac_shortcuts::Event,
    pub(crate) registry: Arc<Registry>,
    pub(crate) settings: rmac_shell_settings::ShellSettings,
    pub(crate) settings_error: Option<SharedString>,
    pub(crate) clipboard: async_channel::Sender<String>,
    pub(crate) applications: rmac_launcher_providers::ApplicationProvider,
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
            applications,
        } = environment;
        let initial_browse_mode = requested_browse_mode(&event);
        // The placeholder is drawn by the bar itself in the measured label
        // colour; the component's placeholder uses a lighter muted colour.
        let query = cx.new(|cx| InputState::new(window, cx));
        let window_handle = window.window_handle();
        cx.subscribe(&query, move |this, query, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = query.read(cx).value().to_string();
                this.panel_query_changed();
                let compact =
                    value.is_empty() && this.browse_mode.is_none() && this.panel.is_none();
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
        query.read(cx).focus_handle(cx).focus(window, cx);
        let compact = initial_browse_mode.is_none();
        if !compact {
            window.resize(size(
                px(rmac_launcher::surface::EXPANDED_LOGICAL_WIDTH as f32),
                px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
            ));
        }
        let mut view = Self {
            token,
            query,
            coordinator,
            registry,
            backend: Arc::new(SystemBackend::new(SurfaceBridge { clipboard })),
            settings_error,
            was_active: false,
            compact,
            browse_mode: initial_browse_mode,
            application_group: None,
            application_view: ApplicationView::Grid,
            application_options_open: false,
            panel: None,
            bar_hovered: false,
            applications,
            press_inside: false,
        };
        view.ensure_browse_selection();
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
                                this.ensure_browse_selection();
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
        if requested_browse_mode(event) == Some(BrowseMode::Applications)
            && self.browse_mode != Some(BrowseMode::Applications)
        {
            self.open_browse(BrowseMode::Applications, window, cx);
            return;
        }
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
        if self.panel.is_some() {
            match command {
                KeyCommand::ArrowDown => self.move_panel_selection(true, cx),
                KeyCommand::ArrowUp => self.move_panel_selection(false, cx),
                KeyCommand::Return | KeyCommand::AlternateReturn => {
                    let selected = self.panel.as_ref().map_or(0, |panel| panel.selected);
                    self.activate_panel_row(selected, window, cx);
                }
                KeyCommand::Escape => {
                    if self.query_text(cx).is_empty() {
                        self.dismiss(window, cx);
                    } else {
                        self.query
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        if let Some(request) = self.coordinator.set_query(String::new()) {
                            self.dispatch(request, cx);
                        }
                        self.panel_query_changed();
                        cx.notify();
                    }
                }
            }
            return;
        }
        if let Some(mode) = self.browse_mode {
            let direction = match command {
                KeyCommand::ArrowDown => Some(MoveSelection::Next),
                KeyCommand::ArrowUp => Some(MoveSelection::Previous),
                _ => None,
            };
            if let Some(direction) = direction {
                let changed = if mode == BrowseMode::Applications {
                    if let Some(group) = self.application_group {
                        self.coordinator
                            .move_selection_in_application_group(group, direction)
                    } else {
                        self.coordinator
                            .move_selection_in_category(mode.category(), direction)
                    }
                } else {
                    self.coordinator
                        .move_selection_in_category(mode.category(), direction)
                };
                if changed {
                    cx.notify();
                }
                return;
            }
        }
        let effect = self.coordinator.handle_key(command);
        self.apply_key_effect(effect, window, cx);
    }

    pub(crate) fn open_browse(
        &mut self,
        mode: BrowseMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browse_mode = Some(mode);
        self.panel = None;
        self.application_options_open = false;
        self.compact = false;
        window.resize(size(
            px(rmac_launcher::surface::EXPANDED_LOGICAL_WIDTH as f32),
            px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
        ));
        self.ensure_browse_selection();
        cx.notify();
    }

    pub(crate) fn set_application_group(
        &mut self,
        group: Option<ApplicationGroup>,
        cx: &mut Context<Self>,
    ) {
        self.application_group = group;
        self.ensure_browse_selection();
        cx.notify();
    }

    pub(crate) fn set_application_view(&mut self, view: ApplicationView, cx: &mut Context<Self>) {
        self.application_view = view;
        self.application_options_open = false;
        cx.notify();
    }

    fn ensure_browse_selection(&mut self) {
        let Some(mode) = self.browse_mode else {
            return;
        };
        let category = mode.category();
        let snapshot = self.coordinator.snapshot();
        let matches_group = |row: &rmac_launcher_runtime::Row| {
            mode != BrowseMode::Applications
                || self
                    .application_group
                    .is_none_or(|group| row.application_group == Some(group))
        };
        if snapshot
            .rows
            .iter()
            .any(|row| row.category == category && matches_group(row) && row.selected)
        {
            return;
        }
        if let Some(row) = snapshot
            .rows
            .iter()
            .find(|row| row.category == category && matches_group(row))
        {
            self.coordinator.select(&row.id);
        }
    }

    /// Rows the current mode shows, in result order.
    pub(crate) fn visible_rows(&self) -> Vec<Row> {
        self.coordinator
            .snapshot()
            .rows
            .iter()
            .filter(|row| {
                self.browse_mode
                    .is_none_or(|mode| row.category == mode.category())
            })
            .filter(|row| {
                self.browse_mode != Some(BrowseMode::Applications)
                    || self
                        .application_group
                        .is_none_or(|group| row.application_group == Some(group))
            })
            .cloned()
            .collect()
    }

    /// Tab accepts the inline completion: the field takes the top hit's
    /// full name, as on macOS.
    fn accept_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let value = self.query.read(cx).value().to_string();
        let top_title = if self.panel.is_some() {
            self.panel_rows(cx).into_iter().next().map(|row| row.title)
        } else {
            self.visible_rows().into_iter().next().map(|row| row.title)
        };
        let Some(title) =
            top_title.filter(|title| completion::inline_completion(&value, title).is_some())
        else {
            return false;
        };
        if title == value {
            return false;
        }
        self.query
            .update(cx, |state, cx| state.set_value(title.clone(), window, cx));
        // `set_value` does not emit a change event, so the query is
        // forwarded here.
        if let Some(request) = self.coordinator.set_query(title) {
            self.dispatch(request, cx);
        }
        cx.notify();
        true
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
