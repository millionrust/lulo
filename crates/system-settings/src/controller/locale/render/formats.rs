//! Locale format-example projection.

use super::*;

impl Settings {
    /// The Mac's centred examples at the top of the Region group: the full
    /// date and time on one 11 pt line, then the short number and currency
    /// forms. None when the locale service reported no preview.
    pub(super) fn locale_format_examples(&self, snapshot: &rmac_locale::Snapshot) -> Option<Div> {
        let preview = snapshot.format_preview.as_ref()?;
        let line = |text: String| {
            div()
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(16.0))
                .text_color(secondary())
                .child(text)
        };
        Some(
            div()
                .v_flex()
                .items_center()
                .pt(px(9.0))
                .pb(px(12.0))
                .child(line(preview.date_time.clone()))
                .child(line(format!("{}    {}", preview.currency, preview.number))),
        )
    }
}
