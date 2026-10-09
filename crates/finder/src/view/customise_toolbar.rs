//! View ▸ Customise Toolbar… (FIL-MENU-005): named toolbar groups the
//! owner can hide, shared by every open Files window
//! (`settings::FinderSettings::hidden_toolbar_items`). A small floating
//! popover, the same shape as Show Preview Options (`preview_options.rs`).
use super::*;

/// `View ▸ Customise Toolbar…`'s offered groups, in the popover's order.
pub(super) const TOOLBAR_ITEMS: [(&str, &str); 3] = [
    ("navigation", "Back/Forward"),
    ("view-control", "View/Sort controls"),
    ("search", "Search"),
];

impl FinderView {
    /// The toolbar's own hidden-items list, read once per render
    /// (`render_toolbar` checks it up to three times) rather than cloning
    /// the whole settings document from each check.
    pub(super) fn hidden_toolbar_items() -> Vec<String> {
        settings::current().hidden_toolbar_items
    }

    /// Whether `key` (one of `TOOLBAR_ITEMS`' first elements) is shown,
    /// given the hidden-items list `hidden_toolbar_items` already read.
    pub(super) fn toolbar_item_shown(hidden: &[String], key: &str) -> bool {
        !hidden.iter().any(|existing| existing == key)
    }

    pub(super) fn toggle_customise_toolbar(&mut self, cx: &mut Context<Self>) {
        if self.customise_toolbar_open {
            self.close_customise_toolbar(cx);
            return;
        }
        self.customise_toolbar_open = true;
        let owner = cx.entity().downgrade();
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            cx.update(|cx| {
                let (width, height) = rmac_ui::outer_window_size(240.0, 148.0);
                let mut options = rmac_ui::window_options_for_panel_with_title(
                    rmac_ui::app_id::FILES,
                    "Customise Toolbar".to_string(),
                    width,
                    height,
                    cx,
                );
                options.focus = false;
                options.kind = gpui::WindowKind::Floating;
                let view_owner = owner.clone();
                let opened = cx.open_window(options, move |window, cx| {
                    rmac_ui::prepare_surface_window(window, cx);
                    let view = cx.new(|cx| CustomiseToolbarWindow {
                        owner: view_owner,
                        focus: cx.focus_handle(),
                    });
                    cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
                });
                let _ = owner.update(cx, |this, cx| {
                    match opened {
                        Ok(handle) => this.customise_toolbar_window = Some(handle),
                        Err(_) => {
                            this.customise_toolbar_open = false;
                            this.operation_error =
                                Some("Files could not open Customise Toolbar".into());
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_customise_toolbar(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.customise_toolbar_window.take() {
            let _ = cx.update_window(*handle, |_, window, _| window.remove_window());
        }
        self.customise_toolbar_open = false;
        cx.notify();
    }

    fn render_customise_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hidden = settings::current().hidden_toolbar_items;
        let entity = cx.entity();
        div()
            .id("finder-customise-toolbar")
            .role(Role::Group)
            .key_context("Finder")
            .aria_label("Customise Toolbar")
            .w(px(240.0))
            .rounded(px(rmac_ui::mac::radius_large_surface()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .bg(rmac_ui::mac::raised())
            .shadow_lg()
            .overflow_hidden()
            .v_flex()
            .text_size(rmac_ui::text_px(12.0))
            .text_color(rmac_ui::mac::text())
            .child(
                div()
                    .relative()
                    .h(px(26.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div().absolute().left(px(5.0)).top(px(2.0)).child(
                            Button::new("ct-close", "×")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    window.remove_window();
                                    this.customise_toolbar_open = false;
                                    this.customise_toolbar_window = None;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child("Customise Toolbar"),
            )
            .children(TOOLBAR_ITEMS.into_iter().map(|(key, label)| {
                let checked = !hidden.iter().any(|existing| existing == key);
                let entity = entity.clone();
                div().px_3().py_1().child(
                    rmac_ui::Checkbox::new(SharedString::from(format!("ct-{key}")))
                        .label(label)
                        .checked(checked)
                        .on_change(move |value, _, cx| {
                            let value = *value;
                            entity.update(cx, |_, cx| {
                                let _ = settings::update(
                                    |settings| {
                                        if value {
                                            settings
                                                .hidden_toolbar_items
                                                .retain(|existing| existing != key);
                                        } else if !settings
                                            .hidden_toolbar_items
                                            .iter()
                                            .any(|existing| existing == key)
                                        {
                                            settings.hidden_toolbar_items.push(key.to_string());
                                        }
                                    },
                                    cx,
                                );
                                cx.notify();
                            });
                        }),
                )
            }))
    }
}

struct CustomiseToolbarWindow {
    owner: gpui::WeakEntity<FinderView>,
    focus: FocusHandle,
}

impl CustomiseToolbarWindow {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |owner, cx| {
            owner.customise_toolbar_open = false;
            owner.customise_toolbar_window = None;
            cx.notify();
        });
        window.remove_window();
    }
}

impl gpui::Focusable for CustomiseToolbarWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CustomiseToolbarWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self
            .owner
            .update(cx, |owner, cx| owner.render_customise_toolbar(cx))
            .ok();
        div()
            .size_full()
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &CustomiseToolbar, window, cx| {
                this.close(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.close(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    this.close(window, cx);
                }
            }))
            .when_some(panel, |container, panel| container.child(panel))
    }
}
