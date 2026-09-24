//! Text Editor title bar: the Mac's plain 32 pt TextEdit bar.
//!
//! Measured on macOS 26.2 (design-lab/apps.html): no toolbar and no status
//! bar. The lights sit in the shared title bar, the document proxy icon is an
//! 18 pt frame at x 81, the 13 pt bold title (#9F9EAB) starts at x 100 and
//! " — Edited" follows dimmer (#63626F). A ▾ shows while the pointer is over
//! the title; title and ▾ open the document menu, which in rmac holds the
//! encoding and line-ending choices the saver backs.

use rmac_ui::DocumentTitleMenu;

use super::*;

/// Left edge of the proxy icon's 18 pt frame, from the window edge.
const PROXY_ICON_X: f32 = 81.0;
/// The proxy icon's frame; the title follows it directly (x 100).
const PROXY_ICON_FRAME: f32 = 18.0;
/// `rmac_ui::title_bar_content` insets its content 12 from the window edge.
const TITLE_BAR_CONTENT_INSET: f32 = 12.0;

/// A plain-text document page drawn at the Mac's proxy-icon size.
fn document_proxy_icon() -> impl IntoElement {
    let line = || div().h(px(1.0)).w(px(6.0)).bg(mac::text_tertiary());
    div()
        .w(px(PROXY_ICON_FRAME))
        .h(px(PROXY_ICON_FRAME))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(12.0))
                .h(px(15.0))
                .rounded(px(1.5))
                .bg(mac::text().opacity(0.85))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .child(line())
                .child(line())
                .child(line()),
        )
}

impl EditorView {
    pub(super) fn render_title_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.filename();
        let format = self.text_format;
        let format_status = format.status_against(self.saved_format);
        let format_locked = self.file_busy || self.rtf_runs.is_some() || self.file_action_blocked();
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .pl(px(PROXY_ICON_X - TITLE_BAR_CONTENT_INSET))
            .pr_2()
            .child(document_proxy_icon())
            .child(
                div().min_w_0().pl(px(1.0)).child(
                    DocumentTitleMenu::new("document-actions", title)
                        .edited(self.dirty)
                        .accessible_label("Document options")
                        .disabled(format_locked)
                        .menu(move |menu, _, _| {
                            menu.label(format_status.clone())
                                .separator()
                                .menu_with_check(
                                    "Unicode (UTF-8)",
                                    format.encoding == document::TextEncoding::Utf8,
                                    Box::new(SetEncodingUtf8),
                                )
                                .menu_with_check(
                                    "Unicode (UTF-8) with BOM",
                                    format.encoding == document::TextEncoding::Utf8Bom,
                                    Box::new(SetEncodingUtf8Bom),
                                )
                                .menu_with_check(
                                    "Unicode (UTF-16 Little-Endian)",
                                    format.encoding == document::TextEncoding::Utf16Le,
                                    Box::new(SetEncodingUtf16Le),
                                )
                                .menu_with_check(
                                    "Unicode (UTF-16 Big-Endian)",
                                    format.encoding == document::TextEncoding::Utf16Be,
                                    Box::new(SetEncodingUtf16Be),
                                )
                                .separator()
                                .menu_with_check(
                                    "Unix (LF)",
                                    format.save_line_ending == document::LineEnding::Lf,
                                    Box::new(SetLineEndingLf),
                                )
                                .menu_with_check(
                                    "Windows (CRLF)",
                                    format.save_line_ending == document::LineEnding::CrLf,
                                    Box::new(SetLineEndingCrLf),
                                )
                                .menu_with_check(
                                    "Classic Mac (CR)",
                                    format.save_line_ending == document::LineEnding::Cr,
                                    Box::new(SetLineEndingCr),
                                )
                        }),
                ),
            );
        rmac_ui::title_bar_content(row)
    }
}
