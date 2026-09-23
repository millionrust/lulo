//! Notification cards, stacked groups, and their hover controls, measured
//! against macOS 26 (design-lab/notifications.html).

use super::*;

/// Card geometry and colours from design-lab/notifications.html.
pub(super) mod card {
    pub const WIDTH: f32 = 344.0;
    pub const RADIUS: f32 = 22.0;
    pub const PAD_V: f32 = 14.0;
    pub const PAD_LEFT: f32 = 14.0;
    pub const PAD_RIGHT: f32 = 16.0;
    pub const MIN_HEIGHT: f32 = 64.0;
    pub const ICON: f32 = 32.0;
    pub const ICON_GAP: f32 = 10.0;
    pub const LINE: f32 = 16.0;
    pub const TITLE_SIZE: f32 = 13.0;
    pub const TIME_SIZE: f32 = 12.0;
    pub const BODY_LINES: usize = 4;
    pub const GAP: f32 = 8.0;
    /// Collapsed stacks show up to two cards peeking below the newest one.
    pub const PEEK: f32 = 7.0;
    pub const PEEK_INSET: f32 = 9.0;
    pub const PEEK_HEIGHT: f32 = 40.0;
    pub const CLOSE: f32 = 20.0;
    pub const CLOSE_OFFSET: f32 = -4.0;
    pub const HEADER_HEIGHT: f32 = 30.0;
    pub const HEADER_NAME_SIZE: f32 = 15.0;
    pub const HEADER_BUTTON: f32 = 24.0;

    /// Tahoe's dark glass is rgba(40,40,44,.72) over a blur; a shared layer
    /// surface cannot blur per card, so the same tone is drawn nearly opaque.
    pub const FILL: u32 = 0x242428F0;
    pub const BORDER: u32 = 0xFFFFFF1C;
    pub const PEEK_NEAR: u32 = 0x323236E6;
    pub const PEEK_FAR: u32 = 0x2C2C30CC;
    pub const TITLE: u32 = 0xFFFFFFEB;
    pub const BODY: u32 = 0xFFFFFFDB;
    pub const TIME: u32 = 0xFFFFFF8C;
    pub const CLOSE_FILL: u32 = 0x404044EB;
    pub const CLOSE_BORDER: u32 = 0xFFFFFF33;
    pub const CLOSE_GLYPH: u32 = 0xFFFFFFD9;
    pub const CONTROL_FILL: u32 = 0x3A3A3EE6;
}

/// The current wall-clock time for relative timestamps.
#[derive(Clone, Copy)]
pub(super) struct Clock {
    pub(super) now_ms: u64,
    pub(super) offset_seconds: i32,
}

impl Clock {
    pub(super) fn now() -> Self {
        Self {
            now_ms: rmac_notifications_linux::origin::unix_ms_now(),
            offset_seconds: chrono::Local::now().offset().local_minus_utc(),
        }
    }
}

/// What a card's × button clears.
#[derive(Clone)]
enum CloseTarget {
    Record(NotificationId),
    Group { key: String, app_ids: Vec<String> },
}

fn glass(element: gpui::Div, radius: f32, fill: u32) -> gpui::Div {
    element
        .rounded(px(radius))
        .bg(rgba(fill))
        .border_1()
        .border_color(rgba(card::BORDER))
}

fn close_glyph(size: f32) -> impl IntoElement {
    gpui_component::Icon::new(IconName::Close)
        .size(px(size))
        .text_color(rgba(card::CLOSE_GLYPH))
}

impl NotificationCenterView {
    fn app_icon(identity: &ApplicationIdentity) -> AnyElement {
        let size = card::ICON;
        if let Some(icon) = &identity.icon {
            return img(icon.clone())
                .size(px(size))
                .flex_none()
                .into_any_element();
        }
        // Unresolved senders get a plain plate, like a generic app icon.
        let initial = identity
            .name
            .chars()
            .next()
            .map(|character| character.to_uppercase().to_string())
            .unwrap_or_else(|| "•".into());
        div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.225))
            .bg(mac::control_fill())
            .text_color(mac::text_secondary())
            .font_weight(mac::SEMIBOLD)
            .text_size(rmac_ui::text_px(size * 0.45))
            .child(initial)
            .into_any_element()
    }

    fn close_button(
        &self,
        id: SharedString,
        target: CloseTarget,
        cx: &Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        div()
            .id(id)
            .absolute()
            .left(px(card::CLOSE_OFFSET))
            .top(px(card::CLOSE_OFFSET))
            .size(px(card::CLOSE))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rgba(card::CLOSE_FILL))
            .border_1()
            .border_color(rgba(card::CLOSE_BORDER))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let target = target.clone();
                view.update(cx, |this, cx| match target {
                    CloseTarget::Record(id) => this.remove(id, cx),
                    CloseTarget::Group { key, app_ids } => this.clear_group(key, app_ids, cx),
                });
            })
            .child(close_glyph(8.0))
            .into_any_element()
    }

    /// One notification card: app icon, title with its relative time, and
    /// the body. Clicking runs the default action when the sender offered one.
    #[allow(clippy::too_many_arguments)]
    fn card(
        &self,
        record: &HistoryRecord,
        identity: &ApplicationIdentity,
        hover_key: String,
        close: CloseTarget,
        on_click_expand: Option<String>,
        clock: Clock,
        cx: &Context<Self>,
    ) -> AnyElement {
        let hovered = self.hovered.as_deref() == Some(hover_key.as_str());
        let time = record
            .origin
            .posted_unix_ms
            .map(|posted| relative_time(posted, clock.now_ms, clock.offset_seconds));
        let title = if record.content.title().is_empty() {
            identity.name.to_string()
        } else {
            record.content.title().to_owned()
        };
        let default_action = record
            .actions
            .iter()
            .any(|action| action.selection == ActionSelection::Default);
        let buttons = record
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| action.selection != ActionSelection::Default)
            .map(|(action_index, action)| {
                let id = record.id;
                let selection = action.selection;
                let view = cx.entity();
                let action_busy = matches!(
                    &self.busy,
                    Some(Busy::Invoke(candidate, candidate_selection))
                        if *candidate == id && *candidate_selection == selection
                );
                Button::new(
                    SharedString::from(format!(
                        "notification-{}-action-{}",
                        record.id.get(),
                        action_index
                    )),
                    action.label.clone(),
                )
                .xsmall()
                .role(ButtonRole::Secondary)
                .disabled(self.busy.is_some())
                .busy(action_busy)
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    view.update(cx, |this, cx| this.invoke_action(id, selection, cx));
                })
            })
            .collect::<Vec<_>>();
        let view = cx.entity();
        let hover_view = view.clone();
        let hover_target = hover_key.clone();
        let record_id = record.id;
        let close_id = SharedString::from(format!("close-{hover_key}"));

        div()
            .id(SharedString::from(format!("card-{hover_key}")))
            .relative()
            .w(px(card::WIDTH))
            .min_h(px(card::MIN_HEIGHT))
            .flex()
            .items_center()
            .gap(px(card::ICON_GAP))
            .pt(px(card::PAD_V))
            .pb(px(card::PAD_V))
            .pl(px(card::PAD_LEFT))
            .pr(px(card::PAD_RIGHT))
            .rounded(px(card::RADIUS))
            .bg(rgba(card::FILL))
            .border_1()
            .border_color(rgba(card::BORDER))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_hover(move |hovered: &bool, _, cx| {
                let hovered = *hovered;
                let target = hover_target.clone();
                hover_view.update(cx, |this, cx| this.set_hovered(&target, hovered, cx));
            })
            .on_click(move |_, _, cx| {
                if let Some(key) = on_click_expand.clone() {
                    view.update(cx, |this, cx| this.toggle_expanded(&key, cx));
                } else if default_action {
                    view.update(cx, |this, cx| {
                        this.invoke_action(record_id, ActionSelection::Default, cx)
                    });
                }
            })
            .child(Self::app_icon(identity))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .v_flex()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rmac_ui::text_px(card::TITLE_SIZE))
                                    .line_height(px(card::LINE))
                                    .font_weight(mac::SEMIBOLD)
                                    .text_color(rgba(card::TITLE))
                                    .child(title),
                            )
                            .when_some(time, |row, time| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_size(rmac_ui::text_px(card::TIME_SIZE))
                                        .line_height(px(card::LINE))
                                        .text_color(rgba(card::TIME))
                                        .child(time),
                                )
                            }),
                    )
                    .when(!record.content.body().is_empty(), |text| {
                        text.child(
                            div()
                                .whitespace_normal()
                                .line_clamp(card::BODY_LINES)
                                .text_size(rmac_ui::text_px(card::TITLE_SIZE))
                                .line_height(px(card::LINE))
                                .text_color(rgba(card::BODY))
                                .child(record.content.body().to_owned()),
                        )
                    })
                    .when(hovered && !buttons.is_empty(), |text| {
                        text.child(div().flex().flex_wrap().gap_1().pt_1p5().children(buttons))
                    }),
            )
            .when(hovered, |card_element| {
                card_element.child(self.close_button(close_id, close, cx))
            })
            .into_any_element()
    }

    /// A group is a single card, a collapsed stack (newest card plus up to
    /// two peeking behind it), or an expanded list under an app header.
    pub(super) fn group(
        &self,
        group: &RecordGroup<'_>,
        clock: Clock,
        cx: &Context<Self>,
    ) -> AnyElement {
        let group_close = CloseTarget::Group {
            key: group.key.clone(),
            app_ids: group.app_ids.clone(),
        };
        let Some(newest) = group.records.first().copied() else {
            return div().into_any_element();
        };
        if group.records.len() == 1 {
            return self.card(
                newest,
                &group.identity,
                format!("record-{}", newest.id.get()),
                CloseTarget::Record(newest.id),
                None,
                clock,
                cx,
            );
        }

        if !self.expanded.contains(&group.key) {
            let peeks = (group.records.len() - 1).min(2);
            let top = self.card(
                newest,
                &group.identity,
                format!("stack-{}", group.key),
                group_close,
                Some(group.key.clone()),
                clock,
                cx,
            );
            return div()
                .relative()
                .w(px(card::WIDTH))
                .pb(px(card::PEEK * peeks as f32))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when(peeks >= 2, |stack| {
                    stack.child(
                        glass(div(), card::RADIUS, card::PEEK_FAR)
                            .absolute()
                            .bottom_0()
                            .left(px(card::PEEK_INSET * 2.0))
                            .right(px(card::PEEK_INSET * 2.0))
                            .h(px(card::PEEK_HEIGHT)),
                    )
                })
                .child(
                    glass(div(), card::RADIUS, card::PEEK_NEAR)
                        .absolute()
                        .bottom(px(card::PEEK * (peeks as f32 - 1.0)))
                        .left(px(card::PEEK_INSET))
                        .right(px(card::PEEK_INSET))
                        .h(px(card::PEEK_HEIGHT)),
                )
                .child(top)
                .into_any_element();
        }

        let collapse_view = cx.entity();
        let collapse_key = group.key.clone();
        let clear_view = cx.entity();
        let clear_key = group.key.clone();
        let clear_app_ids = group.app_ids.clone();
        let busy = self.busy.is_some();
        let cards = group
            .records
            .iter()
            .map(|record| {
                self.card(
                    record,
                    &group.identity,
                    format!("record-{}", record.id.get()),
                    CloseTarget::Record(record.id),
                    None,
                    clock,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        div()
            .w(px(card::WIDTH))
            .v_flex()
            .gap(px(card::GAP))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(card::HEADER_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pl(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(rmac_ui::text_px(card::HEADER_NAME_SIZE))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(rgba(card::TITLE))
                            .child(group.identity.name.clone()),
                    )
                    .child(
                        glass(div(), card::HEADER_BUTTON / 2.0, card::CONTROL_FILL)
                            .id(SharedString::from(format!("show-less-{}", group.key)))
                            .h(px(card::HEADER_BUTTON))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .text_size(rmac_ui::text_px(12.0))
                            .font_weight(mac::MEDIUM)
                            .text_color(rgba(card::TITLE))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                collapse_view
                                    .update(cx, |this, cx| this.toggle_expanded(&collapse_key, cx));
                            })
                            .child(SHOW_LESS_LABEL),
                    )
                    .child(
                        glass(div(), card::HEADER_BUTTON / 2.0, card::CONTROL_FILL)
                            .id(SharedString::from(format!("clear-{}", group.key)))
                            .size(px(card::HEADER_BUTTON))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(busy, |button| button.opacity(0.5))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| {
                                let key = clear_key.clone();
                                let app_ids = clear_app_ids.clone();
                                clear_view
                                    .update(cx, |this, cx| this.clear_group(key, app_ids, cx));
                            })
                            .child(close_glyph(9.0)),
                    ),
            )
            .children(cards)
            .into_any_element()
    }
}
