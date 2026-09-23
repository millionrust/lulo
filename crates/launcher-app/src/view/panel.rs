//! The one-panel Actions (⌘3) and Clipboard (⌘4) views.

use std::path::PathBuf;

use gpui::{Context, SharedString, Window};

use super::actions::{self, ActionItem};
use super::LauncherView;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelMode {
    Actions,
    Clipboard,
}

impl PanelMode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Clipboard => "Clipboard",
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Actions => "spotlight/shortcuts.svg",
            Self::Clipboard => "spotlight/clipboard.svg",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClipboardState {
    Loading,
    /// History has not been allowed: show the first-use prompt.
    Disabled,
    Unavailable,
    Items(Vec<rmac_clipboard::Item>),
}

/// Everything the open panel shows and runs.
pub(crate) struct Panel {
    pub(crate) mode: PanelMode,
    pub(crate) selected: usize,
    pub(crate) catalog: Vec<ActionItem>,
    pub(crate) clipboard: ClipboardState,
    pub(crate) busy: bool,
    /// Arrow keys were used, so the idle (sectioned) list shows a selection.
    pub(crate) navigated: bool,
    pub(crate) error: Option<SharedString>,
}

/// One drawn row, whichever view it came from.
#[derive(Clone, Debug)]
pub(crate) struct PanelRow {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) section: String,
    pub(crate) icon: PanelIcon,
}

#[derive(Clone, Debug)]
pub(crate) enum PanelIcon {
    Image(PathBuf),
    Glyph(&'static str),
}

impl Panel {
    fn new(mode: PanelMode) -> Self {
        Self {
            mode,
            selected: 0,
            catalog: Vec::new(),
            clipboard: ClipboardState::Loading,
            busy: false,
            navigated: false,
            error: None,
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

impl LauncherView {
    pub(crate) fn open_panel(
        &mut self,
        mode: PanelMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browse_mode = None;
        self.application_options_open = false;
        self.compact = false;
        self.panel = Some(Panel::new(mode));
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        window.resize(gpui::size(
            gpui::px(rmac_launcher::surface::EXPANDED_LOGICAL_WIDTH as f32),
            gpui::px(rmac_launcher::surface::EXPANDED_LOGICAL_HEIGHT as f32),
        ));
        match mode {
            PanelMode::Actions => self.load_actions(cx),
            PanelMode::Clipboard => self.load_clipboard(cx),
        }
        cx.notify();
    }

    /// Leave the panel for the idle bar (Esc on an empty query, Not Now).
    pub(crate) fn close_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        self.compact = true;
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        window.resize(gpui::size(
            gpui::px(rmac_launcher::surface::LOGICAL_WIDTH as f32),
            gpui::px(rmac_launcher::surface::LOGICAL_HEIGHT as f32),
        ));
        cx.notify();
    }

    fn load_actions(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let system = blocking::unblock(actions::load_system_state).await;
            let frontmost = actions::load_frontmost().await;
            let _ = this.update(cx, |this, cx| {
                let applications = this.applications.clone();
                if let Some(panel) = this.panel.as_mut() {
                    panel.catalog = actions::catalog(&system, frontmost.as_ref(), &applications);
                    panel.selected = 0;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_clipboard(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let state = match rmac_clipboard_linux::client::snapshot().await {
                Ok(rmac_clipboard_linux::client::Snapshot::Disabled) => ClipboardState::Disabled,
                Ok(rmac_clipboard_linux::client::Snapshot::Items(items)) => {
                    ClipboardState::Items(items)
                }
                Err(_) => ClipboardState::Unavailable,
            };
            let _ = this.update(cx, |this, cx| {
                let query = this.query_text(cx);
                if let Some(panel) = this.panel.as_mut() {
                    panel.clipboard = state;
                    panel.selected = panel
                        .selected
                        .min(rows(panel, &query).len().saturating_sub(1));
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn query_text(&self, cx: &gpui::App) -> String {
        self.query.read(cx).value().to_string()
    }

    /// The rows the panel shows for the current query.
    pub(crate) fn panel_rows(&self, cx: &gpui::App) -> Vec<PanelRow> {
        match self.panel.as_ref() {
            Some(panel) => rows(panel, &self.query_text(cx)),
            None => Vec::new(),
        }
    }

    pub(crate) fn panel_query_changed(&mut self) {
        if let Some(panel) = self.panel.as_mut() {
            panel.selected = 0;
            panel.error = None;
        }
    }

    pub(crate) fn move_panel_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.panel_rows(cx).len();
        if let Some(panel) = self.panel.as_mut() {
            if count == 0 {
                return;
            }
            panel.navigated = true;
            panel.selected = if forward {
                (panel.selected + 1).min(count - 1)
            } else {
                panel.selected.saturating_sub(1)
            };
            cx.notify();
        }
    }

    /// Run the selected row: perform the action, or put the clipboard
    /// item back on the clipboard. Spotlight closes when it succeeds.
    pub(crate) fn activate_panel_row(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.query_text(cx);
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        if panel.busy {
            return;
        }
        panel.selected = index;
        let window_handle = window.window_handle();
        match panel.mode {
            PanelMode::Actions => {
                let Some(item) = actions::filter(&panel.catalog, &query)
                    .into_iter()
                    .nth(index)
                else {
                    return;
                };
                panel.busy = true;
                let backend = self.backend.clone();
                let notes = self
                    .applications
                    .application(rmac_apps::identity::NOTES)
                    .map(|app| app.launch);
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    let result = actions::execute(item.command, backend, notes).await;
                    finish_panel_action(&this, result, window_handle, cx);
                })
                .detach();
            }
            PanelMode::Clipboard => {
                let Some(id) = clipboard_items(panel, &query)
                    .get(index)
                    .map(|item| item.entry.id)
                else {
                    return;
                };
                panel.busy = true;
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    let result = rmac_clipboard_linux::client::copy(id)
                        .await
                        .map_err(|error| error.to_string());
                    finish_panel_action(&this, result, window_handle, cx);
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// ⌘⌫ on a clipboard row forgets it.
    pub(crate) fn remove_clipboard_row(&mut self, cx: &mut Context<Self>) {
        let query = self.query_text(cx);
        let Some(panel) = self.panel.as_ref() else {
            return;
        };
        if panel.mode != PanelMode::Clipboard {
            return;
        }
        let Some(id) = clipboard_items(panel, &query)
            .get(panel.selected)
            .map(|item| item.entry.id)
        else {
            return;
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let removed = rmac_clipboard_linux::client::remove(id).await;
            let _ = this.update(cx, |this, cx| {
                if removed.is_err() {
                    if let Some(panel) = this.panel.as_mut() {
                        panel.error = Some("The item could not be removed.".into());
                    }
                }
                this.load_clipboard(cx);
            });
        })
        .detach();
    }

    /// The first-use prompt's buttons.
    pub(crate) fn answer_clipboard_prompt(
        &mut self,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !allow {
            self.close_panel(window, cx);
            return;
        }
        if let Some(panel) = self.panel.as_mut() {
            panel.clipboard = ClipboardState::Loading;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_clipboard_linux::client::set_enabled(true).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.load_clipboard(cx),
                    Err(_) => {
                        if let Some(panel) = this.panel.as_mut() {
                            panel.clipboard = ClipboardState::Unavailable;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

/// Close Spotlight after a successful action; otherwise show why it failed.
fn finish_panel_action(
    this: &gpui::WeakEntity<LauncherView>,
    result: Result<(), String>,
    window: gpui::AnyWindowHandle,
    cx: &mut gpui::AsyncApp,
) {
    let _ = cx.update_window(window, |_, window, cx| {
        let _ = this.update(cx, |this, cx| {
            let succeeded = result.is_ok();
            if let Some(panel) = this.panel.as_mut() {
                panel.busy = false;
                if let Err(detail) = &result {
                    panel.error = Some(detail.clone().into());
                }
            }
            if succeeded {
                this.dismiss(window, cx);
            } else {
                cx.notify();
            }
        });
    });
}

fn clipboard_items<'a>(panel: &'a Panel, query: &str) -> Vec<&'a rmac_clipboard::Item> {
    let ClipboardState::Items(items) = &panel.clipboard else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| rmac_launcher::query_matches(query.trim(), &item.entry.title, None))
        .collect()
}

fn rows(panel: &Panel, query: &str) -> Vec<PanelRow> {
    match panel.mode {
        PanelMode::Actions => actions::filter(&panel.catalog, query)
            .into_iter()
            .map(|item| PanelRow {
                title: item.title,
                subtitle: item.subtitle,
                section: item.section,
                icon: item
                    .icon
                    .map(PanelIcon::Image)
                    .unwrap_or(PanelIcon::Glyph("spotlight/shortcuts.svg")),
            })
            .collect(),
        PanelMode::Clipboard => {
            let now = now_ms();
            clipboard_items(panel, query)
                .into_iter()
                .map(|item| PanelRow {
                    title: item.entry.title.clone(),
                    subtitle: rmac_clipboard::subtitle(&item.entry, now),
                    section: String::new(),
                    icon: match item.entry.kind {
                        rmac_clipboard::Kind::Image => PanelIcon::Image(item.payload.clone()),
                        rmac_clipboard::Kind::Files => PanelIcon::Glyph("spotlight/folder.svg"),
                        rmac_clipboard::Kind::Text => PanelIcon::Glyph("spotlight/clipboard.svg"),
                    },
                })
                .collect()
        }
    }
}
