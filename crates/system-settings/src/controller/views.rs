//! The sidebar and the detail pane as their own GPUI views (SPEED-02).
//!
//! `Settings` stays the one model and the window's root view; these two thin
//! views render the sidebar and the detail pane from it, and the root draws
//! them as cached views. A frame whose only dirty view is the pane (a
//! background load landed for the pane on screen) re-renders the root shell
//! and the pane and reuses the sidebar's last layout and paint.
//!
//! Both views observe `Settings`, so every `cx.notify()` on it still repaints
//! the whole window exactly as before. Window refreshes (focus, activation,
//! appearance, scale) re-render cached views as well; a resize re-renders
//! only a view whose own bounds changed or whose last render read the
//! window's geometry (ADR 0026), so the fixed-width sidebar keeps its last
//! frame through a live resize and takes its split from the root. Only
//! `Settings::notify_pane` and `Settings::notify_if_showing`, which load
//! completions use, leave the sidebar alone; the sidebar reads selection,
//! navigation, search, hardware availability, compact layout and window
//! focus, and every change to those goes through `cx.notify()` or a refresh.

use super::*;

pub(super) struct SettingsViews {
    pub(super) sidebar: Entity<SidebarView>,
    pub(super) pane: Entity<PaneView>,
}

pub(super) struct SidebarView {
    settings: gpui::WeakEntity<Settings>,
}

pub(super) struct PaneView {
    settings: gpui::WeakEntity<Settings>,
}

impl SettingsViews {
    pub(super) fn new(settings: &Entity<Settings>, cx: &mut App) -> Self {
        let sidebar = cx.new(|cx| {
            cx.observe(settings, |_, _, cx| {
                // SPEED-02: which notification dirtied the window.
                rmac_ui::trace_mark("settings_notified");
                cx.notify()
            })
            .detach();
            SidebarView {
                settings: settings.downgrade(),
            }
        });
        let pane = cx.new(|cx| {
            cx.observe(settings, |_, _, cx| cx.notify()).detach();
            PaneView {
                settings: settings.downgrade(),
            }
        });
        Self { sidebar, pane }
    }
}

/// A cached view needs a definite size: it is laid out from this style, not
/// measured. Both views fill the column the root sizes for them. While an
/// assistive technology is connected every frame rebuilds the whole
/// accessibility tree from prepaint, which a reused subtree skips, so the
/// views are drawn uncached then.
pub(super) fn view_element<V: Render>(view: &Entity<V>, cached: bool) -> AnyElement {
    if cached {
        view.clone()
            .cached(gpui::StyleRefinement::default().size_full())
            .into_any_element()
    } else {
        view.clone().into_any_element()
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rmac_ui::trace_mark("settings_sidebar_render");
        let sidebar = self.settings.upgrade().map(|settings| {
            settings.update(cx, |settings, cx| {
                // The root renders first and records its split; reading the
                // window's width here would tie this view to every resize.
                let layout = settings
                    .rendered_layout
                    .unwrap_or_else(|| settings.layout(window));
                settings
                    .render_sidebar(layout, window, cx)
                    .into_any_element()
            })
        });
        div().size_full().flex().children(sidebar)
    }
}

impl Render for PaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rmac_ui::trace_mark("settings_pane_render");
        let detail = self
            .settings
            .upgrade()
            .map(|settings| settings.update(cx, |settings, cx| settings.render_detail(cx)));
        div().size_full().v_flex().children(detail)
    }
}
