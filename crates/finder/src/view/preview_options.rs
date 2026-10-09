//! View ▸ Show Preview Options (FIL-MENU-006): which extra fields the
//! Column view's preview pane shows under "Information"
//! (`content_presentation::render_column_preview`), from
//! `settings::PREVIEW_INFO_FIELDS`. A small floating popover, the same
//! shape as View Options (`view_options.rs`) but independent of it — the
//! Mac keeps these as two separate commands too.
use super::*;

impl FinderView {
    pub(super) fn toggle_preview_options(&mut self, cx: &mut Context<Self>) {
        if self.preview_options_open {
            self.close_preview_options(cx);
            return;
        }
        self.preview_options_open = true;
        let owner = cx.entity().downgrade();
        // Opening a window renders it immediately. Defer until this Finder
        // update ends, since its utility view reads the owner's controls,
        // the same reason `toggle_view_options` defers.
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            cx.update(|cx| {
                let (width, height) = rmac_ui::outer_window_size(220.0, 148.0);
                let mut options = rmac_ui::window_options_for_panel_with_title(
                    rmac_ui::app_id::FILES,
                    "Preview Options".to_string(),
                    width,
                    height,
                    cx,
                );
                options.focus = false;
                options.kind = gpui::WindowKind::Floating;
                let view_owner = owner.clone();
                let opened = cx.open_window(options, move |window, cx| {
                    rmac_ui::prepare_surface_window(window, cx);
                    let view = cx.new(|cx| PreviewOptionsWindow {
                        owner: view_owner,
                        focus: cx.focus_handle(),
                    });
                    cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
                });
                let _ = owner.update(cx, |this, cx| {
                    match opened {
                        Ok(handle) => this.preview_options_window = Some(handle),
                        Err(_) => {
                            this.preview_options_open = false;
                            this.operation_error =
                                Some("Files could not open Preview Options".into());
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_preview_options(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.preview_options_window.take() {
            let _ = cx.update_window(*handle, |_, window, _| window.remove_window());
        }
        self.preview_options_open = false;
        cx.notify();
    }

    fn render_preview_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = settings::current().preview_info_fields;
        let entity = cx.entity();
        div()
            .id("finder-preview-options")
            .role(Role::Group)
            .key_context("Finder")
            .aria_label("Preview Options")
            .w(px(220.0))
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
                            Button::new("po-close", "×")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    window.remove_window();
                                    this.preview_options_open = false;
                                    this.preview_options_window = None;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child("Preview Options"),
            )
            .children(settings::PREVIEW_INFO_FIELDS.into_iter().map(|field| {
                let checked = selected.iter().any(|existing| existing == field);
                let entity = entity.clone();
                div().px_3().py_1().child(
                    rmac_ui::Checkbox::new(SharedString::from(format!("po-{field}")))
                        .label(field)
                        .checked(checked)
                        .on_change(move |value, _, cx| {
                            let value = *value;
                            entity.update(cx, |_, cx| {
                                let _ = settings::update(
                                    |settings| {
                                        if value {
                                            if !settings
                                                .preview_info_fields
                                                .iter()
                                                .any(|existing| existing == field)
                                            {
                                                settings
                                                    .preview_info_fields
                                                    .push(field.to_string());
                                            }
                                        } else {
                                            settings
                                                .preview_info_fields
                                                .retain(|existing| existing != field);
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

struct PreviewOptionsWindow {
    owner: gpui::WeakEntity<FinderView>,
    focus: FocusHandle,
}

impl PreviewOptionsWindow {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |owner, cx| {
            owner.preview_options_open = false;
            owner.preview_options_window = None;
            cx.notify();
        });
        window.remove_window();
    }
}

impl gpui::Focusable for PreviewOptionsWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PreviewOptionsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self
            .owner
            .update(cx, |owner, cx| owner.render_preview_options(cx))
            .ok();
        div()
            .size_full()
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &ShowPreviewOptions, window, cx| {
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
