//! The sidebar, toolbar and content area as their own GPUI views
//! (SPEED-08).
//!
//! `FinderView` stays the one model and the window's root view; these thin
//! views render the sidebar, the toolbar and the content area (list, icon
//! grid, columns or gallery) from it, and the root draws them as cached
//! views. A wheel scroll in the list or the icon grid notifies only the
//! content view, so the frame re-renders the root shell (sheets, banners)
//! and the rows on screen, and reuses the sidebar's and toolbar's last
//! layout and paint. Before, the scroll notified `FinderView` and re-laid
//! out the whole window at about 15 ms a frame.
//!
//! All three views observe `FinderView`, so every `cx.notify()` on it repaints
//! the whole window exactly as before, and window refreshes (focus,
//! activation, resize, appearance) re-render cached views as well.

use super::*;

pub(super) struct FinderViews {
    pub(super) sidebar: Entity<SidebarView>,
    pub(super) toolbar: Entity<ToolbarView>,
    pub(super) content: Entity<ContentView>,
}

pub(super) struct ToolbarView {
    finder: gpui::WeakEntity<FinderView>,
}

pub(super) struct SidebarView {
    finder: gpui::WeakEntity<FinderView>,
}

pub(super) struct ContentView {
    finder: gpui::WeakEntity<FinderView>,
}

impl FinderViews {
    pub(super) fn new(finder: &Entity<FinderView>, cx: &mut gpui::App) -> Self {
        let sidebar = cx.new(|cx| {
            cx.observe(finder, |_, _, cx| cx.notify()).detach();
            SidebarView {
                finder: finder.downgrade(),
            }
        });
        let toolbar = cx.new(|cx| {
            cx.observe(finder, |_, _, cx| cx.notify()).detach();
            ToolbarView {
                finder: finder.downgrade(),
            }
        });
        let content = cx.new(|cx| {
            cx.observe(finder, |_, _, cx| cx.notify()).detach();
            ContentView {
                finder: finder.downgrade(),
            }
        });
        Self {
            sidebar,
            toolbar,
            content,
        }
    }
}

/// A cached view is laid out from this style, not measured, so both views
/// fill the box the root sizes for them. While an assistive technology is
/// connected every frame rebuilds the whole accessibility tree from
/// prepaint, which a reused subtree skips, so the views draw uncached then.
pub(super) fn view_element<V: Render>(view: &Entity<V>, cached: bool) -> gpui::AnyElement {
    if cached {
        view.clone()
            .cached(gpui::StyleRefinement::default().size_full())
            .into_any_element()
    } else {
        view.clone().into_any_element()
    }
}

impl FinderView {
    /// Repaint the content area (and the root shell around it) but not the
    /// sidebar: for a scroll, which only moves the rows on screen.
    pub(super) fn notify_content(&self, cx: &mut Context<Self>) {
        match &self.views {
            Some(views) => views.content.update(cx, |_, cx| cx.notify()),
            None => cx.notify(),
        }
    }
}

impl Render for SidebarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.finder.upgrade().map(|finder| {
            finder.update(cx, |finder, cx| {
                finder.render_sidebar(cx).into_any_element()
            })
        });
        div().size_full().flex().children(sidebar)
    }
}

impl Render for ToolbarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_width = f32::from(window.bounds().size.width);
        let toolbar = self.finder.upgrade().map(|finder| {
            finder.update(cx, |finder, cx| {
                let layout = responsive_layout::responsive_layout(
                    window_width,
                    finder.sidebar_visible,
                    finder.sidebar_width,
                );
                finder.render_toolbar(layout, cx).into_any_element()
            })
        });
        div().size_full().flex().children(toolbar)
    }
}

impl Render for ContentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_active = window.is_window_active();
        let window_height = f32::from(window.bounds().size.height);
        let window_width = f32::from(window.bounds().size.width);
        let content = self.finder.upgrade().map(|finder| {
            finder.update(cx, |finder, cx| {
                let layout = responsive_layout::responsive_layout(
                    window_width,
                    finder.sidebar_visible,
                    finder.sidebar_width,
                );
                let sidebar = if layout.sidebar_visible {
                    finder.sidebar_width
                } else {
                    0.0
                };
                finder
                    .render_list(window_active, window_height, window_width - sidebar, cx)
                    .into_any_element()
            })
        });
        div().size_full().v_flex().children(content)
    }
}
