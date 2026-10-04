//! Form sheets shared by Users & Groups, Login Password and Printers &
//! Scanners (design-lab/settings-users-printers.html): a 460 pt card,
//! radius 24, 20 in; a 15 pt bold title, an optional 12 pt explanation, a
//! grouped form of label-and-field rows, an error line and right-aligned
//! buttons.

use super::*;

pub(in crate::controller) const FORM_SHEET_WIDTH: f32 = 460.0;
const FIELD_WIDTH: f32 = 240.0;

/// One form row: the label on the left, the text field on the right.
/// Secret fields are not exposed as accessible text (their value would be
/// readable by assistive technology).
pub(in crate::controller) fn sheet_field_row(
    id: &'static str,
    title: &'static str,
    editor: &Entity<InputState>,
    secret: bool,
    enabled: bool,
    cx: &App,
) -> AnyElement {
    let field = div()
        .id(id)
        .role(Role::TextInput)
        .aria_label(title)
        .w(px(FIELD_WIDTH))
        .flex_none()
        .child(TextField::new(editor).disabled(!enabled).w_full());
    let field = if secret {
        field
    } else {
        field.accessible_text_input(editor, cx)
    };
    row_base()
        .child(text_block(title.into(), None))
        .child(field)
        .into_any_element()
}

/// A row holding arbitrary trailing content (a pop-up, a value).
pub(in crate::controller) fn sheet_value_row(title: &'static str, value: AnyElement) -> AnyElement {
    row_base()
        .child(text_block(title.into(), None))
        .child(value)
        .into_any_element()
}

/// The sheet card. `header` sits above the form (an avatar, a note).
#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn form_sheet(
    id: &'static str,
    title: impl Into<SharedString>,
    note: Option<SharedString>,
    header: Option<AnyElement>,
    rows: Vec<AnyElement>,
    error: Option<SharedString>,
    leading: Option<AnyElement>,
    buttons: Vec<AnyElement>,
) -> rmac_ui::Dialog {
    let title: SharedString = title.into();
    let body = div()
        .w(px(FORM_SHEET_WIDTH))
        .v_flex()
        .gap_3()
        .p_5()
        .rounded(px(rmac_ui::mac::radius_large_surface()))
        .bg(rmac_ui::mac::raised())
        .border_1()
        .border_color(rmac_ui::mac::separator())
        .shadow_xl()
        .child(
            div()
                .text_size(rmac_ui::text_px(15.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(label())
                .child(title.clone()),
        )
        .when_some(note, |body, note| {
            body.child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .line_height(px(16.0))
                    .text_color(secondary())
                    .child(note),
            )
        })
        .when_some(header, |body, header| body.child(header))
        .when(!rows.is_empty(), |body| body.child(card(rows).mb(px(0.0))))
        .when_some(error, |body, error| {
            body.child(
                div()
                    .id(SharedString::from(format!("{id}-error")))
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::danger())
                    .child(error),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .when_some(leading, |row, leading| row.child(leading))
                .child(div().flex_1())
                .children(buttons),
        );
    rmac_ui::dialog(id, body).attached().aria_label(title)
}

/// A dialog button that runs `action` with the window.
pub(in crate::controller) fn sheet_button(
    id: &'static str,
    title: &'static str,
    kind: rmac_ui::DialogButtonKind,
    enabled: bool,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    rmac_ui::dialog_button(id, title, kind)
        .disabled(!enabled)
        .on_click(move |_, window, cx| action(window, cx))
        .into_any_element()
}

/// A round account picture, or the initials on a neutral disc.
pub(in crate::controller) fn account_picture(
    picture: Option<&PathBuf>,
    initials: &str,
    size: f32,
) -> AnyElement {
    match picture {
        Some(picture) => img(picture.clone())
            .size(px(size))
            .flex_none()
            .rounded_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rmac_ui::mac::control_fill())
            .text_size(rmac_ui::text_px((size * 0.4).round()))
            .font_weight(rmac_ui::mac::BOLD)
            .text_color(secondary())
            .child(initials.to_owned())
            .into_any_element(),
    }
}
