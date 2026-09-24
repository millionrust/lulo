//! Which window a desktop menu-bar command acts on.
//!
//! The top bar runs in its own process. Clicking it can take keyboard focus
//! away from the app, so GPUI then has no active window and `App::dispatch_action`
//! falls back to global handlers only, and the command silently does nothing.
//! Even with an active window, nothing inside it may hold focus, so the
//! command never reaches the view that handles it. macOS sends menu commands
//! to the key window's first responder; this module keeps the equivalent: the
//! app's most recently active window, and the focus handle of the content that
//! handles its commands.

use gpui::{Action, AnyWindowHandle, App, Context, FocusHandle, Global, Window};

/// Keys in order of recency, least recent first.
#[derive(Debug)]
struct Recency<K, V> {
    entries: Vec<(K, V)>,
}

impl<K, V> Default for Recency<K, V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<K: Copy + PartialEq, V: Clone> Recency<K, V> {
    /// Add or replace `key` as the most recent entry.
    fn remember(&mut self, key: K, value: V) {
        self.entries.retain(|(existing, _)| *existing != key);
        self.entries.push((key, value));
    }

    /// Make an already remembered `key` the most recent entry.
    fn promote(&mut self, key: K) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(existing, _)| *existing == key)
        {
            let entry = self.entries.remove(index);
            self.entries.push(entry);
        }
    }

    fn forget_where(&mut self, mut predicate: impl FnMut(&K) -> bool) {
        self.entries.retain(|(key, _)| !predicate(key));
    }

    fn value(&self, key: K) -> Option<V> {
        self.entries
            .iter()
            .find(|(existing, _)| *existing == key)
            .map(|(_, value)| value.clone())
    }

    /// The window a command goes to: the platform's active window when there
    /// is one, otherwise the most recently active remembered window.
    fn target(&self, active: Option<K>) -> Option<(K, Option<V>)> {
        match active {
            Some(active) => Some((active, self.value(active))),
            None => self
                .entries
                .last()
                .map(|(key, value)| (*key, Some(value.clone()))),
        }
    }
}

#[derive(Default)]
struct MenuTargets {
    windows: Recency<AnyWindowHandle, FocusHandle>,
    observing_closes: bool,
}

impl Global for MenuTargets {}

/// Let desktop menu-bar commands reach this window's content even when the
/// window has lost keyboard focus to the top bar, or nothing in it is
/// focused. `focus` must be the handle tracked by the element that handles
/// the app's menu actions.
pub fn register_menu_target<V: 'static>(
    window: &mut Window,
    focus: &FocusHandle,
    cx: &mut Context<V>,
) {
    let handle = window.window_handle();
    let first_window = {
        let targets = cx.default_global::<MenuTargets>();
        targets.windows.remember(handle, focus.clone());
        !std::mem::replace(&mut targets.observing_closes, true)
    };
    if first_window {
        cx.on_window_closed(|cx, closed| {
            cx.default_global::<MenuTargets>()
                .windows
                .forget_where(|window| window.window_id() == closed);
        })
        .detach();
    }
    cx.observe_window_activation(window, |_, window, cx| {
        if window.is_window_active() {
            let handle = window.window_handle();
            cx.default_global::<MenuTargets>().windows.promote(handle);
        }
    })
    .detach();
}

/// Send a menu-bar command to the app's key window, focusing its registered
/// content first if the command would otherwise not reach a handler.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn dispatch_menu_action(action: Box<dyn Action>, cx: &mut App) {
    let active = cx.active_window();
    let target = match cx.try_global::<MenuTargets>() {
        Some(targets) => targets.windows.target(active),
        None => active.map(|window| (window, None)),
    };
    let Some((window_handle, focus)) = target else {
        cx.dispatch_action(action.as_ref());
        return;
    };
    let delivered = window_handle.update(cx, |_, window, cx| {
        if let Some(focus) = focus {
            if !window.is_action_available(action.as_ref(), cx)
                && window.is_action_available_in(action.as_ref(), &focus)
            {
                window.focus(&focus, cx);
            }
        }
        window.dispatch_action(action.boxed_clone(), cx);
    });
    if delivered.is_err() {
        // The window closed between the click and now.
        cx.default_global::<MenuTargets>()
            .windows
            .forget_where(|window| *window == window_handle);
        cx.dispatch_action(action.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::Recency;

    #[test]
    fn commands_prefer_the_active_window_then_the_last_active_one() {
        let mut windows = Recency::default();
        assert_eq!(windows.target(None), None::<(u32, Option<&str>)>);
        windows.remember(1, "first");
        windows.remember(2, "second");
        assert_eq!(windows.target(None), Some((2, Some("second"))));
        windows.promote(1);
        assert_eq!(windows.target(None), Some((1, Some("first"))));
        assert_eq!(windows.target(Some(2)), Some((2, Some("second"))));
        assert_eq!(windows.target(Some(3)), Some((3, None)));
    }

    #[test]
    fn closed_windows_are_forgotten_and_re_registering_replaces() {
        let mut windows = Recency::default();
        windows.remember(1, "a");
        windows.remember(2, "b");
        windows.remember(1, "c");
        assert_eq!(windows.target(None), Some((1, Some("c"))));
        windows.forget_where(|window| *window == 1);
        assert_eq!(windows.target(None), Some((2, Some("b"))));
        windows.promote(7);
        assert_eq!(windows.target(None), Some((2, Some("b"))));
        windows.forget_where(|_| true);
        assert_eq!(windows.target(None), None);
    }
}
