//! Keyboard commands on the selected query result: ⌘↓/⌘↑ between
//! sections, ⌘C to copy it and ⌘Y to show a file in Quick Look, as in
//! macOS 26 Spotlight. (⌘Return, "Show in Files", is the alternate action.)

use std::path::PathBuf;

use gpui::{ClipboardItem, Context, Window};
use rmac_launcher::{Category, MoveSelection};
use rmac_launcher_runtime::Row;

use super::LauncherView;

/// The text ⌘C copies for a row: the answer, the time, the definition, a
/// file's path, otherwise the name. `None` for the "Search in" row.
pub(crate) fn copy_text(row: &Row) -> Option<String> {
    match row.category {
        Category::Calculator => Some(row.title.clone()),
        Category::Clock => row.detail.clone(),
        Category::Dictionary => row.subtitle.clone(),
        Category::Files => Some(row.id.local.clone()),
        Category::SearchIn => None,
        Category::Applications | Category::Settings | Category::Other => Some(row.title.clone()),
    }
    .filter(|text| !text.is_empty())
}

impl LauncherView {
    pub(crate) fn selected_row(&self) -> Option<Row> {
        self.visible_rows().into_iter().find(|row| row.selected)
    }

    pub(crate) fn move_by_section(&mut self, forward: bool, cx: &mut Context<Self>) {
        let direction = if forward {
            MoveSelection::Next
        } else {
            MoveSelection::Previous
        };
        self.keyboard_selection = true;
        self.coordinator.move_selection_by_section(direction);
        cx.notify();
    }

    /// ⌘C: copy the selected result unless the field has a text selection,
    /// which the field copies itself.
    pub(crate) fn copy_selected(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.query.read(cx).selected_range().is_empty() {
            return false;
        }
        let Some(text) = self.selected_row().as_ref().and_then(copy_text) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// ⌘Y: Quick Look on the selected file. Spotlight is a keyboard-owning
    /// overlay that cannot step aside the way the Mac's hides, so it closes
    /// and the panel opens from the launcher service.
    pub(crate) fn quick_look_selected(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(row) = self.selected_row() else {
            return false;
        };
        if row.category != Category::Files {
            return false;
        }
        let path = PathBuf::from(&row.id.local);
        if !path.is_absolute() {
            return false;
        }
        self.dismiss(window, cx);
        cx.defer(move |cx| {
            let _ = rmac_quick_look::open(vec![path], 0, rmac_quick_look::Options::default(), cx);
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::copy_text;
    use rmac_launcher::{Category, ResultId};
    use rmac_launcher_runtime::Row;

    fn row(category: Category, title: &str) -> Row {
        Row {
            id: ResultId {
                provider: rmac_shell_settings::ProviderId("test".into()),
                local: "/home/alex/Report.txt".into(),
            },
            category,
            application_group: None,
            category_label: category.label(),
            title: title.into(),
            subtitle: Some("the definition".into()),
            detail: Some("12:46 AM".into()),
            icon: None,
            selected: true,
            primary_label: "Open",
            has_alternate: false,
            alternate_label: None,
        }
    }

    #[test]
    fn command_c_copies_what_the_row_answers() {
        assert_eq!(
            copy_text(&row(Category::Calculator, "84")).as_deref(),
            Some("84")
        );
        assert_eq!(
            copy_text(&row(Category::Clock, "Tokyo, Japan")).as_deref(),
            Some("12:46 AM")
        );
        assert_eq!(
            copy_text(&row(Category::Dictionary, "serendipity")).as_deref(),
            Some("the definition")
        );
        assert_eq!(
            copy_text(&row(Category::Files, "Report.txt")).as_deref(),
            Some("/home/alex/Report.txt")
        );
        assert_eq!(
            copy_text(&row(Category::Applications, "Terminal")).as_deref(),
            Some("Terminal")
        );
        assert_eq!(copy_text(&row(Category::SearchIn, "Search in Files")), None);
    }
}
