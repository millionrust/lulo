//! Shared typed row, control, icon, and visualization builders.

mod appearance_input;
mod bluetooth;
mod displays;
mod focus;
mod locale;
mod network;
mod power;
mod shell;
mod sound;
mod storage;

use super::*;
pub(super) use appearance_input::*;
pub(super) use bluetooth::*;
pub(super) use displays::*;
pub(super) use focus::*;
pub(super) use locale::*;
pub(super) use network::*;
pub(super) use power::*;
pub(super) use shell::*;
pub(super) use sound::*;
pub(super) use storage::*;
// ---- row / control builders ----------------------------------------------

pub(super) fn row_base() -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(44.0))
        .px_3()
        .py_2()
}

pub(super) fn text_block(title: SharedString, sub: Option<SharedString>) -> Div {
    let mut b = div().v_flex().flex_1().child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(s) = sub {
        b = b.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .text_color(secondary())
                .child(s),
        );
    }
    b
}

/// A plain card-section label row (no control).
pub(super) fn label_row(title: &'static str, value: Option<SharedString>) -> Div {
    let mut r = row_base().child(
        div()
            .flex_1()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(label())
            .child(title),
    );
    if let Some(v) = value {
        r = r.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
    }
    r
}

/// A read-only row with a right-aligned value.
pub(super) fn value_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: SharedString,
) -> AnyElement {
    row_base()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

/// rather than a reflection of (or control over) real system hardware.
/// An informational note card, e.g. to flag a pane as simulated/demo state
pub(super) fn note_card(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mb_3()
        .px_3()
        .py_2p5()
        .rounded(px(rmac_ui::mac::radius_card()))
        .bg(rmac_ui::mac::warning_background())
        .border_1()
        .border_color(rmac_ui::mac::warning_border())
        .child(glyph("icons/info.svg", 15.0, rmac_ui::mac::warning_text()))
        .child(
            div()
                .flex_1()
                .text_size(rmac_ui::text_px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
        )
}

/// A section header above a card (gray small caps-ish title).
pub(super) fn section_header(title: impl Into<SharedString>) -> Div {
    div()
        .px_1()
        .pt_2()
        .pb_1()
        .text_size(rmac_ui::text_px(12.0))
        .font_weight(rmac_ui::mac::SEMIBOLD)
        .text_color(secondary())
        .child(title.into())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn notification_toggle_row(
    view: &Entity<Settings>,
    app_id: &str,
    id: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    disabled: bool,
    change: fn(bool) -> NotificationPolicyChange,
) -> AnyElement {
    let app = app_id.to_owned();
    let control_view = view.clone();
    let toggle = Toggle::new(ElementId::from(SharedString::from(format!(
        "notification-{id}-{app_id}"
    ))))
    .checked(checked)
    .disabled(disabled)
    .on_click(move |value, _, cx| {
        control_view.update(cx, |settings, cx| {
            settings.apply_notification_policy(app.clone(), change(*value), cx);
        });
    });
    row_base()
        .child(text_block(title.into(), subtitle.map(Into::into)))
        .child(toggle)
        .into_any_element()
}

pub(super) fn application_icon(
    icon: Option<&PathBuf>,
    fallback_icon: &'static str,
    fallback_color: Hsla,
) -> AnyElement {
    match icon {
        Some(icon) => img(icon.clone())
            .w(px(22.0))
            .h(px(22.0))
            .flex_none()
            .into_any_element(),
        None => tile(fallback_icon, fallback_color, 22.0).into_any_element(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn application_nav_row(
    view: &Entity<Settings>,
    app_id: &str,
    display_name: &str,
    icon: Option<&PathBuf>,
    status: &'static str,
    enabled: bool,
    target: SubPage,
) -> AnyElement {
    let target_view = view.clone();
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(application_icon(
            icon,
            "icons/bell.svg",
            if enabled { accent() } else { secondary() },
        ))
        .child(text_block(display_name.to_owned().into(), None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(status),
        )
        .child(glyph(
            "icons/chevron-right.svg",
            14.0,
            rmac_ui::mac::text_tertiary(),
        ));
    ListRow::new(
        SharedString::from(format!("notification-app-{app_id}")),
        content,
    )
    .h(px(52.0))
    .px_3()
    .on_activate(move |_, _, cx| {
        let target = target.clone();
        target_view.update(cx, |settings, cx| settings.push(target, cx));
    })
    .into_any_element()
}

/// A clickable navigation row that pushes a subpage onto the back stack.
pub(super) fn nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
    target: SubPage,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("nav-{title}")));
    let mut content = div()
        .w_full()
        .flex()
        .items_center()
        .gap_3()
        .child(tile(icon, color, 22.0))
        .child(text_block(title, None));
    if let Some(v) = value {
        content = content.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
    }
    content = content.child(glyph(
        "icons/chevron-right.svg",
        14.0,
        rmac_ui::mac::text_tertiary(),
    ));
    ListRow::new(id, content)
        .h(px(52.0))
        .px_3()
        .on_activate(move |_, _, cx| {
            let target = target.clone();
            view.update(cx, |s, cx| s.push(target, cx));
        })
        .into_any_element()
}

/// Build a rounded white card from rows, inserting inset separators.
pub(super) fn card(rows: Vec<AnyElement>) -> Div {
    let mut c = div()
        .v_flex()
        .mb_3()
        .rounded(px(rmac_ui::mac::radius_card()))
        .bg(card_bg())
        .border_1()
        .border_color(sep());
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        c = c.child(r);
        if i + 1 < n {
            c = c.child(div().h(px(1.0)).bg(sep()).mx_3());
        }
    }
    c
}
