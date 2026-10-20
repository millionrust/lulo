//! A note shown in a window of its own (NOTES-25): Window ▸ Open Note in
//! New Window and Quick Note. Each pane keeps its own note, title, body
//! and tags editors, caret and undo, as each Mac Notes window does; the
//! main window's selection never moves because of it. Edits are saved
//! through the main view's pipeline (`NotesView::schedule_note_edit`), and
//! a pane with nothing unsaved follows changes made elsewhere.

use gpui::{
    div, px, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_editor::rich::{ListKind, ParagraphRole, RichTextEditor, RichTextEvent};
use rmac_editor::InputState;
use rmac_notes_runtime::EditGeneration;
use rmac_notes_store::NoteId;
use rmac_ui::{AccessibleTextInput as _, InputEvent, StyledExt as _, TextField};

use crate::note_format_controller::{body_document, body_markdown, new_body_editor};
use crate::notes_style::*;
use crate::presentation::full_date_label;
use crate::*;

pub(crate) struct NotePane {
    main: Entity<NotesView>,
    /// The note shown; `None` while a new Quick Note is being created.
    note: Option<NoteId>,
    title: Entity<InputState>,
    tags: Entity<InputState>,
    body: Entity<RichTextEditor>,
    focus: FocusHandle,
    /// The newest save this pane sent that the library has not taken yet.
    pending: Option<EditGeneration>,
    /// The note revision the editors were last loaded from.
    loaded_revision: Option<u64>,
    loading: bool,
}

impl NotePane {
    pub(crate) fn new(
        main: Entity<NotesView>,
        note: Option<NoteId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let tags = cx.new(|cx| InputState::new(window, cx).placeholder("Add Tags"));
        let body = new_body_editor(window, cx);
        cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.save(cx);
            }
        })
        .detach();
        cx.subscribe(&tags, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.save(cx);
            }
        })
        .detach();
        cx.subscribe(&body, |this, _, event: &RichTextEvent, cx| {
            if *event == RichTextEvent::Changed {
                this.save(cx);
            }
        })
        .detach();
        cx.observe_in(&main, window, |this, _, window, cx| {
            this.follow(window, cx);
            cx.notify();
        })
        .detach();
        let focus = cx.focus_handle();
        let mut pane = Self {
            main,
            note,
            title,
            tags,
            body,
            focus,
            pending: None,
            loaded_revision: None,
            loading: false,
        };
        pane.follow(window, cx);
        let focus = gpui::Focusable::focus_handle(pane.body.read(cx), cx);
        window.focus(&focus, cx);
        pane
    }

    pub(crate) fn note(&self) -> Option<NoteId> {
        self.note
    }

    /// Show `note` instead.
    pub(crate) fn show_note(&mut self, note: NoteId, window: &mut Window, cx: &mut Context<Self>) {
        if self.note != Some(note) {
            self.note = Some(note);
            self.pending = None;
            self.loaded_revision = None;
            self.follow(window, cx);
            cx.notify();
        }
    }

    /// Reload from the library when this pane has nothing unsaved and the
    /// note changed elsewhere.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note_id) = self.note else {
            return;
        };
        let (accepted, record, smart_tags) = {
            let main = self.main.read(cx);
            let record = main
                .session
                .snapshot()
                .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                .filter(|note| !main.session.is_note_closed(note))
                .cloned();
            (main.last_accepted_generation, record, main.smart_tags)
        };
        if self
            .pending
            .is_some_and(|pending| accepted.is_some_and(|accepted| accepted >= pending))
        {
            self.pending = None;
        }
        let Some(record) = record else {
            return;
        };
        if self.pending.is_some() || self.loaded_revision == Some(record.revision) {
            return;
        }
        self.loaded_revision = Some(record.revision);
        let document = body_document(&record.body);
        let inline: std::collections::BTreeSet<String> = if smart_tags {
            crate::tag_browser::inline_tags(&document.text())
                .into_iter()
                .map(|tag| tag.to_lowercase())
                .collect()
        } else {
            Default::default()
        };
        let tag_text = record
            .tags
            .iter()
            .filter(|tag| !inline.contains(&tag.to_lowercase()))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        self.loading = true;
        if self.title.read(cx).value() != record.title {
            let title = record.title.clone();
            self.title
                .update(cx, |input, cx| input.set_value(title, window, cx));
        }
        if self.tags.read(cx).value() != tag_text {
            self.tags
                .update(cx, |input, cx| input.set_value(tag_text, window, cx));
        }
        if body_markdown(self.body.read(cx).document()) != record.body {
            self.body
                .update(cx, |editor, cx| editor.set_document(document, cx));
        }
        self.loading = false;
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(note_id) = self.note else {
            return;
        };
        let title = self.title.read(cx).value().to_string();
        let tag_text = self.tags.read(cx).value().to_string();
        let body = body_markdown(self.body.read(cx).document());
        let body_text = self.body.read(cx).text();
        let generation = self.main.update(cx, |main, cx| {
            main.schedule_note_edit(note_id, title, &tag_text, body, &body_text, cx)
        });
        if generation.is_some() {
            self.pending = generation;
        }
    }

    fn format(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut RichTextEditor, &mut Context<RichTextEditor>),
    ) {
        self.body.update(cx, |editor, cx| {
            change(editor, cx);
            editor.focus(window, cx);
        });
    }

    fn toggle_list(&mut self, kind: ListKind, window: &mut Window, cx: &mut Context<Self>) {
        self.format(window, cx, |editor, cx| {
            let current = editor.paragraph_style_at_selection().list;
            editor.set_list_marker((current != Some(kind)).then_some(kind), cx);
        });
    }

    fn role(&mut self, role: ParagraphRole, window: &mut Window, cx: &mut Context<Self>) {
        self.format(window, cx, |editor, cx| editor.set_paragraph_role(role, cx));
    }
}

impl Render for NotePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (record, light, zoom, show_highlights, editable_main) = {
            let main = self.main.read(cx);
            let record = self.note.and_then(|note_id| {
                main.session
                    .snapshot()
                    .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                    .filter(|note| !note.deleted && !main.session.is_note_closed(note))
                    .cloned()
            });
            let light = record.as_ref().is_some_and(|note| {
                main.light_background_default ^ main.light_background_notes.contains(&note.id)
            });
            (
                record,
                light,
                (BODY_SIZE + f32::from(main.note_zoom)) / BODY_SIZE,
                main.show_highlights,
                main.is_interactive_ready(),
            )
        };
        let root = div()
            .key_context("Notes")
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(note_editor_fill(light))
            .text_color(note_editor_text(light))
            .on_action(cx.listener(|this, _: &ToggleBold, window, cx| {
                this.format(window, cx, |editor, cx| editor.toggle_bold(cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleItalic, window, cx| {
                this.format(window, cx, |editor, cx| editor.toggle_italic(cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleUnderline, window, cx| {
                this.format(window, cx, |editor, cx| editor.toggle_underline(cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleStrikethrough, window, cx| {
                this.format(window, cx, |editor, cx| editor.toggle_strikethrough(cx))
            }))
            .on_action(cx.listener(|this, _: &SetStyleTitle, window, cx| {
                this.role(ParagraphRole::Title, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleHeading, window, cx| {
                this.role(ParagraphRole::Heading, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleSubheading, window, cx| {
                this.role(ParagraphRole::Subheading, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleBody, window, cx| {
                this.role(ParagraphRole::Body, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleMonospaced, window, cx| {
                this.role(ParagraphRole::Monostyled, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertChecklist, window, cx| {
                this.toggle_list(ListKind::Checkbox, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertBulletedList, window, cx| {
                this.toggle_list(ListKind::Bullet, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertDashedList, window, cx| {
                this.toggle_list(ListKind::Hyphen, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertNumberedList, window, cx| {
                this.toggle_list(ListKind::Numbered, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleChecklistDone, window, cx| {
                this.format(window, cx, |editor, cx| {
                    if editor.paragraph_style_at_selection().list == Some(ListKind::Checkbox) {
                        editor.toggle_checked(cx);
                    } else {
                        editor.set_list_marker(Some(ListKind::Checkbox), cx);
                    }
                })
            }))
            .on_action(cx.listener(|this, _: &InsertBlockQuote, window, cx| {
                this.format(window, cx, |editor, cx| editor.toggle_quote(cx))
            }));
        let Some(record) = record else {
            let message = if self.note.is_none() {
                "Creating a note…"
            } else {
                "This note is no longer available."
            };
            return root.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(note_editor_date(light))
                    .child(message),
            );
        };
        let editable = editable_main;
        let text_color = note_editor_text(light);
        let paper = note_editor_fill(light);
        self.body.update(cx, |editor, cx| {
            editor.set_editable(editable, cx);
            editor.set_zoom(zoom, cx);
            editor.set_appearance(text_color, mac::text_selection(), mac::text_caret(), cx);
            editor.set_paper_color(paper, cx);
            editor.set_link_color(notes_accent(), cx);
            editor.set_decoration_colors(checkbox_ring(), notes_accent(), checkbox_ring(), cx);
            editor.set_show_highlights(show_highlights, cx);
        });
        let editor = self.body.read(cx);
        let selection = editor.selected_range();
        let head = editor.cursor();
        let anchor = if head == selection.start {
            selection.end
        } else {
            selection.start
        };
        let document = editor.document().clone();
        let body_focus = gpui::Focusable::focus_handle(editor, cx);
        let rich = self.body.clone();
        let title_size = TITLE_SIZE + (zoom - 1.0) * BODY_SIZE;
        root.child(
            div()
                .pt(px(DATE_TOP))
                .h(px(DATE_TOP + DATE_LINE))
                .flex_none()
                .flex()
                .justify_center()
                .text_size(rmac_ui::text_px(DATE_SIZE))
                .text_color(note_editor_date(light))
                .child(full_date_label(record.modified_unix_ms)),
        )
        .child(
            div()
                .id("pane-title")
                .role(Role::TextInput)
                .aria_label("Title")
                .accessible_text_input(&self.title, cx)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if event.keystroke.key != "tab"
                        || modifiers.shift
                        || modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || !this.title.read(cx).focus_handle(cx).is_focused(window)
                    {
                        return;
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    let focus = gpui::Focusable::focus_handle(this.body.read(cx), cx);
                    window.focus(&focus, cx);
                }))
                .flex_none()
                .px(px(EDITOR_INSET))
                .pt(px(TITLE_TOP))
                .font_weight(mac::BOLD)
                .child(
                    TextField::new(&self.title)
                        .appearance(false)
                        .disabled(!editable)
                        .text_size(rmac_ui::text_px(title_size))
                        .h(px(title_size * 1.2))
                        .px_0()
                        .py_0(),
                ),
        )
        .child(
            div()
                .id("pane-body")
                .role(Role::MultilineTextInput)
                .aria_label("Body")
                .accessible_text(
                    move || document.text(),
                    (anchor, head),
                    body_focus,
                    move |range, _window, cx| {
                        rich.update(cx, |editor, cx| editor.select_range(range, cx));
                    },
                )
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if event.keystroke.key != "tab"
                        || modifiers.shift
                        || modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || !this.body.read(cx).is_focused(window)
                        || this.body.read(cx).at_list_item_start()
                    {
                        return;
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    window.focus(&this.tags.read(cx).focus_handle(cx), cx);
                }))
                .flex_1()
                .min_h(px(0.0))
                .pb_2()
                .child(self.body.clone()),
        )
        .child(
            div()
                .id("pane-tags")
                .role(Role::TextInput)
                .aria_label("Tags")
                .accessible_text_input(&self.tags, cx)
                .flex_none()
                .h(px(30.0))
                .mx(px(EDITOR_INSET))
                .flex()
                .items_center()
                .gap_1()
                .border_t_1()
                .border_color(row_rule())
                .text_size(rmac_ui::text_px(12.0))
                .child("#")
                .child(
                    div().flex_1().min_w(px(0.0)).child(
                        TextField::new(&self.tags)
                            .appearance(false)
                            .small()
                            .disabled(!editable),
                    ),
                ),
        )
    }
}
