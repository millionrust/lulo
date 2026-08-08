//! Locale format-example projection.

use super::*;

impl Settings {
    pub(super) fn append_locale_formats(
        &self,
        snapshot: &rmac_locale::Snapshot,
        cards: &mut Vec<Div>,
    ) {
        cards.push(section_header("Format examples"));
        let preview = snapshot.format_preview.as_ref();
        cards.push(card(vec![
            locale_preview_row(
                "icons/clock.svg",
                "Dates and times",
                locale_format(snapshot, "LC_TIME"),
                preview.map(|preview| preview.date_time.as_str()),
            ),
            locale_preview_row(
                "icons/info.svg",
                "Numbers",
                locale_format(snapshot, "LC_NUMERIC"),
                preview.map(|preview| preview.number.as_str()),
            ),
            locale_preview_row(
                "icons/database.svg",
                "Currency",
                locale_format(snapshot, "LC_MONETARY"),
                preview.map(|preview| preview.currency.as_str()),
            ),
            locale_preview_row(
                "icons/settings.svg",
                "Measurement",
                locale_format(snapshot, "LC_MEASUREMENT"),
                None,
            ),
        ]));
        if let Some(error) = &snapshot.format_preview_error {
            cards.push(note_card(format!(
                "Format examples are unavailable: {error}. Locale assignments remain authoritative."
            )));
        }
    }
}
