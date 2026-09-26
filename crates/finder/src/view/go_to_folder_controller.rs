//! Go ▸ Go to Folder… (⇧⌘G) and File ▸ New Finder Window (⌘N).

use super::*;
use rmac_finder::goto;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PendingSelectionAction {
    Select(usize),
    Wait,
    Discard,
}

pub(super) fn pending_selection_action<'a>(
    target: &Path,
    entry_paths: impl Iterator<Item = &'a PathBuf>,
    transfer_active: bool,
) -> PendingSelectionAction {
    if let Some(index) = entry_paths.position(|path| path == target) {
        PendingSelectionAction::Select(index)
    } else if transfer_active {
        PendingSelectionAction::Wait
    } else {
        PendingSelectionAction::Discard
    }
}

/// The Go to Folder sheet: a path field with folder suggestions under it.
pub(super) struct GoToSheet {
    pub(super) input: gpui::Entity<InputState>,
    pub(super) suggestions: Vec<PathBuf>,
    pub(super) highlighted: Option<usize>,
    /// The typed path names nothing; the field says so until it changes.
    pub(super) error: bool,
}

impl FinderView {
    /// File ▸ New Finder Window: another window in this process, opened on
    /// the home folder as Finder's are by default.
    pub(super) fn new_window(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.home.to_str().map(str::to_owned) else {
            self.operation_error = Some("The home folder path cannot open a new window".into());
            cx.notify();
            return;
        };
        if !rmac_ui::open_another_window(vec!["--path".to_owned(), home], cx) {
            self.operation_error = Some("Files could not open another window".into());
            cx.notify();
        }
    }

    pub(super) fn open_go_to_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.go_to.is_some() {
            return;
        }
        self.menu_at = None;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to Folder"));
        cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    let suggestions =
                        goto::suggestions(&text, &this.cwd, &this.home, this.show_hidden);
                    if let Some(sheet) = this.go_to.as_mut() {
                        sheet.highlighted = (!suggestions.is_empty()).then_some(0);
                        sheet.suggestions = suggestions;
                        sheet.error = false;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.commit_go_to_folder(window, cx),
                _ => {}
            },
        )
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.go_to = Some(GoToSheet {
            input,
            suggestions: Vec::new(),
            highlighted: None,
            error: false,
        });
        cx.notify();
    }

    pub(super) fn close_go_to_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn move_go_to_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(sheet) = self.go_to.as_mut() else {
            return;
        };
        if sheet.suggestions.is_empty() {
            return;
        }
        let last = sheet.suggestions.len() as isize - 1;
        let next = sheet
            .highlighted
            .map_or(0, |index| (index as isize + step).clamp(0, last));
        sheet.highlighted = Some(next as usize);
        cx.notify();
    }

    fn go_to_target(&mut self, target: goto::GoTo, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to = None;
        window.focus(&self.focus, cx);
        self.pending_select = target.select;
        if target.folder == self.cwd && !self.trash_view && !self.applications_view {
            // Already here: only the named item needs selecting.
            self.select_pending(cx);
        } else {
            self.navigate(target.folder, cx);
        }
        cx.notify();
    }

    pub(super) fn pick_go_to_suggestion(
        &mut self,
        folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_target(
            goto::GoTo {
                folder,
                select: None,
            },
            window,
            cx,
        );
    }

    fn commit_go_to_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.go_to.as_ref() else {
            return;
        };
        let text = sheet.input.read(cx).value().to_string();
        // A highlighted suggestion wins when the typed path is incomplete.
        let resolved = goto::resolve(&text, &self.cwd, &self.home).or_else(|| {
            sheet
                .highlighted
                .and_then(|index| sheet.suggestions.get(index).cloned())
                .map(|folder| goto::GoTo {
                    folder,
                    select: None,
                })
        });
        match resolved {
            Some(target) => self.go_to_target(target, window, cx),
            None => {
                if let Some(sheet) = self.go_to.as_mut() {
                    sheet.error = true;
                }
                cx.notify();
            }
        }
    }

    /// Selects a pending destination once its folder has loaded. A watcher
    /// reload can run before an active transfer has created its destination,
    /// so keep that pending selection for the transfer's reload.
    pub(super) fn select_pending(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.pending_select.as_ref() else {
            return;
        };
        match pending_selection_action(
            path,
            self.entries.iter().map(|entry| &entry.path),
            self.transfer.is_some(),
        ) {
            PendingSelectionAction::Select(index) => {
                self.pending_select = None;
                self.select_single(index);
                cx.notify();
            }
            PendingSelectionAction::Wait => {}
            PendingSelectionAction::Discard => self.pending_select = None,
        }
    }

    /// The sheet hangs from the toolbar like Finder's: a 460 pt panel with
    /// the path field and up to twelve folder suggestions.
    pub(super) fn render_go_to_folder(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let sheet = self.go_to.as_ref()?;
        let suggestions = sheet
            .suggestions
            .iter()
            .enumerate()
            .map(|(index, folder)| {
                let highlighted = sheet.highlighted == Some(index);
                let target = folder.clone();
                div()
                    .id(("go-to-suggestion", index))
                    .role(Role::ListBoxOption)
                    .aria_label(goto::display(folder, &self.home))
                    .h(px(GO_TO_ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .rounded(px(ROW_RADIUS))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(if highlighted {
                        selected_text(true)
                    } else {
                        primary_text()
                    })
                    .when(highlighted, |row| row.bg(selection(true)))
                    .cursor_pointer()
                    .child(item_artwork(true, "", LIST_ICON))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .child(goto::display(folder, &self.home)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_go_to_suggestion(target.clone(), window, cx)
                    }))
            })
            .collect::<Vec<_>>();
        let panel = div()
            .id("go-to-folder")
            .role(Role::Dialog)
            .aria_label("Go to Folder")
            .relative()
            .w(px(GO_TO_WIDTH))
            .v_flex()
            .gap(px(6.0))
            .p(px(10.0))
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .child(
                div()
                    .id("go-to-close")
                    .role(Role::Button)
                    .aria_label("Close")
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|h| h.bg(rmac_ui::mac::hover()))
                    .child(Icon::new(IconName::Close).text_color(rmac_ui::mac::text_secondary()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_go_to_folder(window, cx);
                    })),
            )
            .child(
                div()
                    .id("go-to-field")
                    .role(Role::TextInput)
                    .aria_label("Go to Folder")
                    .accessible_text_input(&sheet.input, cx)
                    .child(TextField::new(&sheet.input)),
            )
            .when(sheet.error, |panel| {
                panel.child(
                    div()
                        .px(px(4.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child("The folder can’t be found."),
                )
            })
            .when(!suggestions.is_empty(), |panel| {
                panel.child(
                    div()
                        .id("go-to-suggestions")
                        .role(Role::ListBox)
                        .aria_label("Suggestions")
                        .v_flex()
                        .children(suggestions),
                )
            });
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(TOOLBAR_HEIGHT))
                .child(panel)
                .into_any_element(),
        )
    }
}
