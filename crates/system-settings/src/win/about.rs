//! General ▸ About This PC: the Mac's About, from what Windows reports.
//! Any fact Windows does not report is left out, never guessed.

use gpui::{div, Context, Div, ParentElement as _, Styled as _};
use rmac_ui::StyledExt as _;

use super::form::{card, note_card, section_header, value_row};
use super::host::{capacity_label, cores_label, memory_label, os_label, AboutFacts};
use super::WinSettings;

/// The About rows, as (label, value), in the Mac's order: the machine, then
/// its chip and memory, then graphics and the operating system.
pub(super) fn about_rows(facts: &AboutFacts) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(name) = &facts.computer_name {
        rows.push(("Name", name.clone()));
    }
    if let Some(model) = &facts.model {
        rows.push(("Model", model.clone()));
    }
    if let Some(processor) = &facts.processor {
        rows.push(("Processor", processor.clone()));
    }
    if let Some(cores) = facts.cores {
        rows.push(("Cores", cores_label(cores)));
    }
    if let Some(memory) = facts.memory {
        rows.push(("Memory", memory_label(memory)));
    }
    for (index, graphics) in facts.graphics.iter().enumerate() {
        rows.push((if index == 0 { "Graphics" } else { "" }, graphics.clone()));
    }
    if let Some(os) = &facts.os {
        rows.push(("Windows", os_label(os)));
    }
    rows
}

impl WinSettings {
    pub(super) fn render_about(&self, _cx: &mut Context<Self>) -> Div {
        let mut cards = vec![section_header("About This PC").pt(gpui::px(0.0))];
        let Some(facts) = &self.about else {
            cards.push(note_card("Reading this PC's details…"));
            return div().v_flex().children(cards);
        };
        cards.push(card(
            about_rows(facts)
                .into_iter()
                .map(|(title, value)| value_row(title, value))
                .collect(),
        ));
        if !facts.drives.is_empty() {
            cards.push(section_header("Storage"));
            cards.push(card(
                facts
                    .drives
                    .iter()
                    .map(|drive| {
                        value_row(
                            drive.name.clone(),
                            format!(
                                "{} available of {}",
                                capacity_label(drive.available),
                                capacity_label(drive.total)
                            ),
                        )
                    })
                    .collect(),
            ));
        }
        div().v_flex().children(cards)
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{tests::FakeHost, Host as _};
    use super::*;

    #[test]
    fn about_lists_the_reported_facts_in_the_macs_order() {
        let rows = about_rows(&FakeHost.about());
        let titles: Vec<_> = rows.iter().map(|(title, _)| *title).collect();
        assert_eq!(
            titles,
            [
                "Name",
                "Model",
                "Processor",
                "Cores",
                "Memory",
                "Graphics",
                "Windows"
            ]
        );
        assert_eq!(rows[4].1, "8 GB");
        assert_eq!(rows[6].1, "Windows 11 Home 25H2 (build 26200.6584)");
    }

    #[test]
    fn missing_facts_are_left_out() {
        let rows = about_rows(&AboutFacts {
            processor: Some("A CPU".into()),
            ..AboutFacts::default()
        });
        assert_eq!(rows, vec![("Processor", "A CPU".to_owned())]);
    }
}
