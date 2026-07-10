use gpui::{
    actions, div, prelude::*, px, rgb, size, AccessibleAction, App, Bounds, Context, FocusHandle,
    KeyBinding, Role, SharedString, Toggled, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

actions!(rmac_a11y_lab, [Tab, TabPrevious]);

struct AccessibilityLab {
    count: u32,
    enabled: bool,
    focus: FocusHandle,
}

impl AccessibilityLab {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            count: 0,
            enabled: false,
            focus,
        }
    }
}

impl Render for AccessibilityLab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("accessibility-lab")
            .role(Role::Application)
            .aria_label("rmac accessibility lab")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &Tab, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &TabPrevious, window, cx| window.focus_prev(cx)))
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(0x171923))
            .text_color(rgb(0xf7fafc))
            .child(
                div()
                    .id("heading")
                    .role(Role::Heading)
                    .aria_level(1)
                    .aria_label("Accessibility gate")
                    .text_xl()
                    .child("Accessibility gate"),
            )
            .child("Verify this structure and its actions with Orca on Ubuntu/Wayland.")
            .child(
                div()
                    .id("counter")
                    .focusable()
                    .tab_stop(true)
                    .role(Role::SpinButton)
                    .aria_label(SharedString::from(format!("Counter: {}", self.count)))
                    .aria_numeric_value(self.count as f64)
                    .aria_min_numeric_value(0.0)
                    .on_a11y_action(AccessibleAction::Increment, {
                        let this = cx.entity().downgrade();
                        move |_, _, cx| {
                            this.update(cx, |this, cx| {
                                this.count += 1;
                                cx.notify();
                            })
                            .ok();
                        }
                    })
                    .on_a11y_action(AccessibleAction::Decrement, {
                        let this = cx.entity().downgrade();
                        move |_, _, cx| {
                            this.update(cx, |this, cx| {
                                this.count = this.count.saturating_sub(1);
                                cx.notify();
                            })
                            .ok();
                        }
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.count += 1;
                        cx.notify();
                    }))
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x3182ce))
                    .cursor_pointer()
                    .child(format!("Count: {}", self.count)),
            )
            .child(
                div()
                    .id("feature-switch")
                    .focusable()
                    .tab_stop(true)
                    .role(Role::Switch)
                    .aria_label("Enable experimental feature")
                    .aria_toggled(if self.enabled {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.enabled = !this.enabled;
                        cx.notify();
                    }))
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(self.enabled, |element| element.bg(rgb(0x38a169)))
                    .when(!self.enabled, |element| element.bg(rgb(0x4a5568)))
                    .child(if self.enabled {
                        "Experimental feature: on"
                    } else {
                        "Experimental feature: off"
                    }),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", Tab, None),
            KeyBinding::new("shift-tab", TabPrevious, None),
        ]);

        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("rmac upstream accessibility lab".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AccessibilityLab::new(window, cx)),
        )
        .expect("open the accessibility lab window");
        cx.activate(true);
    });
}
