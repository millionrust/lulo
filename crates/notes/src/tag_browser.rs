//! Tags (NOTES-11): `#tags` typed in a note's body join its tags, as on the
//! Mac (Edit ▸ Substitutions ▸ Smart Tags), and the sidebar's Tags browser
//! lists every tag in the library. Clicking tags narrows the note list to
//! notes carrying all of them; All Tags lists every tagged note.

use std::collections::BTreeSet;

use super::*;

/// The Tags browser's selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TagSelection {
    /// Every note with at least one tag.
    All,
    /// Notes carrying every one of these tags (lower-cased).
    Tags(BTreeSet<String>),
}

impl TagSelection {
    pub(super) fn matches(&self, tags: &[String]) -> bool {
        match self {
            Self::All => !tags.is_empty(),
            Self::Tags(wanted) => wanted
                .iter()
                .all(|tag| tags.iter().any(|note_tag| note_tag.to_lowercase() == *tag)),
        }
    }

    /// The list's title: "All Tags", or the chosen tags.
    pub(super) fn title(&self, library: &[String]) -> String {
        match self {
            Self::All => "All Tags".to_owned(),
            Self::Tags(wanted) => library
                .iter()
                .filter(|tag| wanted.contains(&tag.to_lowercase()))
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// Whether `character` can be part of a tag's name.
fn tag_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '-'
}

/// The `#tags` in a note's text, in order, each once (case-insensitively):
/// a `#` that starts a word, then letters, digits, `_` or `-`, with at
/// least one letter (so `#1` and `C#` are not tags).
pub(super) fn inline_tags(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut seen = BTreeSet::new();
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((at, character)) = chars.next() {
        // A tag starts a word: after a space or opening punctuation, never
        // inside an address (`example.com/#top`) or a word (`C#`).
        let starts_word = previous.is_none_or(|before| {
            before.is_whitespace() || "([{\"'\u{201c}\u{2018},;!".contains(before)
        });
        previous = Some(character);
        if character != '#' || !starts_word {
            continue;
        }
        let start = at + 1;
        let mut end = start;
        while let Some(&(next_at, next)) = chars.peek() {
            if !tag_character(next) {
                break;
            }
            end = next_at + next.len_utf8();
            previous = Some(next);
            chars.next();
        }
        let tag = text[start..end].trim_end_matches('-');
        if tag.chars().any(char::is_alphabetic)
            && tag.len() <= rmac_notes_store::MAX_TAG_BYTES
            && seen.insert(tag.to_lowercase())
        {
            tags.push(tag.to_owned());
        }
    }
    tags
}

/// A note's stored tags: the Tags field's, then the body's own `#tags`,
/// each once, at most as many as a note may hold.
pub(super) fn merged_tags(field: Vec<String>, body: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut tags = Vec::with_capacity(field.len() + body.len());
    for tag in field.into_iter().chain(body) {
        if tags.len() == rmac_notes_store::MAX_TAGS_PER_NOTE {
            break;
        }
        if seen.insert(tag.to_lowercase()) {
            tags.push(tag);
        }
    }
    tags
}

impl NotesView {
    /// Every tag of every visible, unlocked note, sorted, each once.
    pub(super) fn library_tags(&self) -> Vec<String> {
        let Some(snapshot) = self.session.snapshot() else {
            return Vec::new();
        };
        let mut seen = BTreeSet::new();
        let mut tags: Vec<String> = snapshot
            .notes
            .iter()
            .filter(|note| !note.deleted && note.lock.is_none())
            .flat_map(|note| note.tags.iter())
            .filter(|tag| seen.insert(tag.to_lowercase()))
            .cloned()
            .collect();
        tags.sort_by_key(|tag| tag.to_lowercase());
        tags
    }

    /// Whether the note list shows `note` under the current Smart Folder
    /// and Tags browser selection.
    pub(super) fn note_listed(&self, note: &NoteRecord) -> bool {
        let smart_tag = self
            .smart_folder_filter
            .and_then(|id| {
                self.session
                    .smart_folders()
                    .into_iter()
                    .find(|folder| folder.id == id)
            })
            .map(|folder| folder.tag.to_lowercase());
        let in_smart_folder = smart_tag.as_deref().is_none_or(|tag| {
            note.lock.is_none() && note.tags.iter().any(|note_tag| note_tag.to_lowercase() == tag)
        });
        let in_tags = self
            .tag_selection
            .as_ref()
            .is_none_or(|selection| note.lock.is_none() && selection.matches(&note.tags));
        in_smart_folder && in_tags
    }

    /// A click on a tag (`None`: All Tags). Clicking a chosen tag drops
    /// it; tags combine. Any tag choice shows notes from every folder.
    pub(super) fn toggle_tag(
        &mut self,
        tag: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_interactive_ready() {
            return;
        }
        let next = match (self.tag_selection.take(), tag) {
            (Some(TagSelection::All), None) => None,
            (_, None) => Some(TagSelection::All),
            (Some(TagSelection::Tags(mut chosen)), Some(tag)) => {
                let key = tag.to_lowercase();
                if !chosen.remove(&key) {
                    chosen.insert(key);
                }
                (!chosen.is_empty()).then_some(TagSelection::Tags(chosen))
            }
            (_, Some(tag)) => Some(TagSelection::Tags(BTreeSet::from([tag.to_lowercase()]))),
        };
        if next.is_some() {
            self.select_folder(rmac_notes_runtime::FolderSelection::All, window, cx);
        }
        self.tag_selection = next;
        cx.notify();
    }

    pub(super) fn render_tag_browser(&self, cx: &mut Context<Self>) -> AnyElement {
        let tags = self.library_tags();
        let selection = self.tag_selection.clone();
        let pill = |id: gpui::ElementId,
                    label: SharedString,
                    selected: bool,
                    tag: Option<String>,
                    cx: &mut Context<Self>| {
            div()
                .id(id)
                .role(Role::ListBoxOption)
                .aria_label(label.clone())
                .aria_selected(selected)
                .px(px(8.0))
                .h(px(22.0))
                .flex()
                .items_center()
                .rounded(px(mac::radius_pill()))
                .text_size(rmac_ui::text_px(12.0))
                .when(selected, |pill| {
                    pill.bg(sidebar_selection()).text_color(sidebar_selected_text())
                })
                .when(!selected, |pill| {
                    pill.bg(mac::control_fill())
                        .text_color(sidebar_text())
                        .hover(|hover| hover.bg(mac::hover()))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.toggle_tag(tag.clone(), window, cx)
                }))
                .into_any_element()
        };
        let mut pills = Vec::with_capacity(tags.len() + 1);
        if !tags.is_empty() {
            pills.push(pill(
                "tag-all".into(),
                "All Tags".into(),
                selection == Some(TagSelection::All),
                None,
                cx,
            ));
        }
        for (index, tag) in tags.into_iter().enumerate() {
            let selected = matches!(
                &selection,
                Some(TagSelection::Tags(chosen)) if chosen.contains(&tag.to_lowercase())
            );
            pills.push(pill(
                ("tag", index).into(),
                format!("#{tag}").into(),
                selected,
                Some(tag),
                cx,
            ));
        }
        div()
            .v_flex()
            .child(
                div()
                    .h(px(SIDEBAR_SECTION_HEIGHT))
                    .mt(px(12.0))
                    .flex()
                    .items_center()
                    .pl(px(SIDEBAR_SECTION_TEXT_X - SIDEBAR_ROW_INSET))
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::BOLD)
                    .text_color(sidebar_section_text())
                    .child("Tags"),
            )
            .child(
                div()
                    .id("notes-tags-browser")
                    .role(Role::ListBox)
                    .aria_label("Tags")
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .pt(px(4.0))
                    .pl(px(SIDEBAR_SECTION_TEXT_X - SIDEBAR_ROW_INSET))
                    .children(pills),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_words_in_the_body_are_tags() {
        assert_eq!(
            inline_tags("Plan #work and #Home-office, not C# or #1 or a#b. #work again"),
            ["work", "Home-office"]
        );
        assert_eq!(inline_tags("#café_2026 trailing-#"), ["café_2026"]);
        assert!(inline_tags("no tags here").is_empty());
    }

    #[test]
    fn field_and_body_tags_merge_once_each() {
        let merged = merged_tags(
            vec!["Work".into(), "home".into()],
            vec!["work".into(), "garden".into()],
        );
        assert_eq!(merged, ["Work", "home", "garden"]);
        let many = (0..40).map(|index| format!("t{index}")).collect();
        assert_eq!(merged_tags(Vec::new(), many).len(), 32);
    }

    #[test]
    fn a_tag_selection_matches_notes_with_every_chosen_tag() {
        let tags = vec!["Work".to_string(), "Urgent".to_string()];
        assert!(TagSelection::All.matches(&tags));
        assert!(!TagSelection::All.matches(&[]));
        let work = TagSelection::Tags(BTreeSet::from(["work".to_string()]));
        assert!(work.matches(&tags));
        let both = TagSelection::Tags(BTreeSet::from(["work".into(), "home".into()]));
        assert!(!both.matches(&tags));
        assert_eq!(
            both.title(&["Home".into(), "Work".into(), "Other".into()]),
            "#Home #Work"
        );
    }
}
