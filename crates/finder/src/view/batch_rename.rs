//! Finder's "Rename N Items…" sheet (FILES-26): Replace Text, Add Text or
//! Format, with a live example, renaming every selected item as one
//! journaled transfer so a single ⌘Z puts all the names back.

use super::*;
use rmac_ui::{PopUpButton, PopupMenuItem};

const SHEET_WIDTH: f32 = 440.0;
const LABEL_WIDTH: f32 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RenameMode {
    Replace,
    Add,
    Format,
}

impl RenameMode {
    const ALL: [RenameMode; 3] = [Self::Replace, Self::Add, Self::Format];

    fn label(self) -> &'static str {
        match self {
            Self::Replace => "Replace Text",
            Self::Add => "Add Text",
            Self::Format => "Format",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Placement {
    BeforeName,
    AfterName,
}

impl Placement {
    const ALL: [Placement; 2] = [Self::AfterName, Self::BeforeName];

    fn label(self) -> &'static str {
        match self {
            Self::BeforeName => "before name",
            Self::AfterName => "after name",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NameFormat {
    Index,
    Counter,
    Date,
}

impl NameFormat {
    const ALL: [NameFormat; 3] = [Self::Index, Self::Counter, Self::Date];

    fn label(self) -> &'static str {
        match self {
            Self::Index => "Name and Index",
            Self::Counter => "Name and Counter",
            Self::Date => "Name and Date",
        }
    }
}

/// Everything the sheet's fields say, as plain values.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RenamePlan {
    pub(super) mode: RenameMode,
    pub(super) find: String,
    pub(super) replace: String,
    pub(super) add: String,
    pub(super) add_at: Placement,
    pub(super) format: NameFormat,
    pub(super) format_at: Placement,
    pub(super) custom: String,
    pub(super) start: u64,
    /// "2026-10-20 at 18.37.07", fixed when the sheet renames.
    pub(super) date: String,
}

/// A name split as Finder edits it: the base and its last extension (a
/// leading dot belongs to the base; folders have no extension).
fn split_name(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// The new name of every item, in order. Formats add " 2", " 3"… to keep
/// names that would otherwise repeat (the same date) apart.
pub(super) fn planned_names(items: &[(String, bool)], plan: &RenamePlan) -> Vec<String> {
    let mut names = Vec::with_capacity(items.len());
    for (index, (name, is_dir)) in items.iter().enumerate() {
        let (base, extension) = split_name(name, *is_dir);
        let number = plan.start.saturating_add(index as u64);
        let base = match plan.mode {
            RenameMode::Replace if plan.find.is_empty() => base.to_owned(),
            RenameMode::Replace => base.replace(&plan.find, &plan.replace),
            RenameMode::Add => match plan.add_at {
                Placement::BeforeName => format!("{}{base}", plan.add),
                Placement::AfterName => format!("{base}{}", plan.add),
            },
            RenameMode::Format => {
                let stamp = match plan.format {
                    NameFormat::Index => number.to_string(),
                    NameFormat::Counter => format!("{number:05}"),
                    NameFormat::Date => plan.date.clone(),
                };
                let custom = plan.custom.trim();
                match (plan.format, plan.format_at) {
                    (NameFormat::Date, Placement::AfterName) => format!("{custom} {stamp}"),
                    (NameFormat::Date, Placement::BeforeName) => format!("{stamp} {custom}"),
                    (_, Placement::AfterName) => format!("{custom}{stamp}"),
                    (_, Placement::BeforeName) => format!("{stamp}{custom}"),
                }
                .trim()
                .to_owned()
            }
        };
        names.push(format!("{base}{extension}"));
    }
    if plan.mode == RenameMode::Format && plan.format == NameFormat::Date {
        let mut seen = BTreeMap::<String, usize>::new();
        for (name, (_, is_dir)) in names.iter_mut().zip(items) {
            let count = seen.entry(name.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                let (base, extension) = split_name(name, *is_dir);
                *name = format!("{base} {count}{extension}");
            }
        }
    }
    names
}

/// Why the planned names cannot be used, if they cannot: an empty or
/// slash-containing name, two items given one name, or a name already
/// taken by something that is not itself being renamed.
pub(super) fn plan_problem(
    sources: &[PathBuf],
    names: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Option<String> {
    let mut destinations = BTreeSet::new();
    for (source, name) in sources.iter().zip(names) {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            return Some("An item can’t have an empty name.".to_owned());
        }
        if trimmed.contains('/') || (cfg!(windows) && trimmed.contains(['\\', ':'])) {
            return Some(format!("The name “{trimmed}” can’t be used."));
        }
        let destination = source.parent().unwrap_or(Path::new("")).join(trimmed);
        if !destinations.insert(destination.clone()) {
            return Some(format!("More than one item would be named “{trimmed}”."));
        }
        if destination != *source && (sources.contains(&destination) || exists(&destination)) {
            return Some(format!(
                "The name “{trimmed}” is already taken. Please choose a different name."
            ));
        }
    }
    None
}

pub(super) struct BatchRenameSheet {
    paths: Vec<PathBuf>,
    items: Vec<(String, bool)>,
    mode: RenameMode,
    find: Entity<InputState>,
    replace: Entity<InputState>,
    add: Entity<InputState>,
    add_at: Placement,
    format: NameFormat,
    format_at: Placement,
    custom: Entity<InputState>,
    start: Entity<InputState>,
    error: Option<SharedString>,
}

fn date_stamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d at %H.%M.%S")
        .to_string()
}

impl BatchRenameSheet {
    fn plan(&self, cx: &gpui::App) -> RenamePlan {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        RenamePlan {
            mode: self.mode,
            find: text(&self.find),
            replace: text(&self.replace),
            add: text(&self.add),
            add_at: self.add_at,
            format: self.format,
            format_at: self.format_at,
            custom: text(&self.custom),
            start: text(&self.start).trim().parse().unwrap_or(1),
            date: date_stamp(),
        }
    }
}

impl FinderView {
    /// File ▸ Rename N Items… and the multi-selection item menu.
    pub(super) fn open_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trash_view || self.applications_view || self.block_mutation_during_transfer(cx) {
            return;
        }
        // In the order the window shows them.
        let entries = self
            .selected
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect::<Vec<_>>();
        if entries.len() < 2 {
            return;
        }
        self.menu_at = None;
        let paths = entries.iter().map(|entry| entry.path.clone()).collect();
        let items = entries
            .iter()
            .map(|entry| (entry.name.to_string(), entry.is_dir))
            .collect();
        let field = |cx: &mut Context<Self>, window: &mut Window, value: &str| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()));
            cx.observe(&input, |_, _, cx| cx.notify()).detach();
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.commit_batch_rename(window, cx);
                }
            })
            .detach();
            input
        };
        let find = field(cx, window, "");
        let sheet = BatchRenameSheet {
            paths,
            items,
            mode: RenameMode::Replace,
            replace: field(cx, window, ""),
            add: field(cx, window, ""),
            add_at: Placement::AfterName,
            format: NameFormat::Index,
            format_at: Placement::AfterName,
            custom: field(cx, window, ""),
            start: field(cx, window, "1"),
            find: find.clone(),
            error: None,
        };
        self.batch_rename = Some(sheet);
        find.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn close_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.batch_rename.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(super) fn commit_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.batch_rename.as_ref() else {
            return;
        };
        let plan = sheet.plan(cx);
        let names = planned_names(&sheet.items, &plan);
        let sources = sheet.paths.clone();
        if let Some(problem) = plan_problem(&sources, &names, |path| {
            std::fs::symlink_metadata(path).is_ok()
        }) {
            if let Some(sheet) = self.batch_rename.as_mut() {
                sheet.error = Some(problem.into());
            }
            cx.notify();
            return;
        }
        let tasks = sources
            .iter()
            .zip(&names)
            .filter_map(|(source, name)| {
                let destination = source.parent()?.join(name.trim());
                (destination != *source).then(|| file_ops::TransferTask {
                    kind: file_ops::TransferKind::Move,
                    source: source.clone(),
                    destination,
                })
            })
            .collect::<Vec<_>>();
        self.batch_rename = None;
        window.focus(&self.focus, cx);
        if tasks.is_empty() {
            cx.notify();
            return;
        }
        if self.operation_journal.is_none() {
            self.operation_error =
                Some("File-operation recovery is unavailable; Rename is disabled".into());
            cx.notify();
            return;
        }
        self.pending_select = tasks.first().map(|task| task.destination.clone());
        self.start_transfer_with_retained(
            "Renaming",
            tasks,
            TransferStartOptions {
                keep_unfinished_in_clipboard: false,
                retained_clipboard: Vec::new(),
                play_drop_sound: false,
                completion: None,
            },
            cx,
        );
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn batch_popup<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        name: &'static str,
        current: T,
        choices: &'static [T],
        label: fn(T) -> &'static str,
        set: fn(&mut BatchRenameSheet, T),
        width: f32,
        cx: &mut Context<Self>,
    ) -> PopUpButton {
        let entity = cx.entity().downgrade();
        PopUpButton::new(id, label(current))
            .accessible_name(name)
            .w(px(width))
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, &choice| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(label(choice))
                            .checked(choice == current)
                            .on_click(move |_, _, cx| {
                                let _ = entity.update(cx, |this: &mut FinderView, cx| {
                                    if let Some(sheet) = this.batch_rename.as_mut() {
                                        set(sheet, choice);
                                        sheet.error = None;
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                })
            })
    }

    pub(super) fn render_batch_rename(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let sheet = self.batch_rename.as_ref()?;
        let row = |label_text: &'static str, control: gpui::AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(LABEL_WIDTH))
                        .flex_none()
                        .text_right()
                        .text_color(primary_text())
                        .child(label_text),
                )
                .child(div().flex_1().min_w(px(0.0)).child(control))
        };
        let text = |id: &'static str, name: &'static str, input: &Entity<InputState>| {
            div()
                .id(id)
                .role(Role::TextInput)
                .aria_label(name)
                .accessible_text_input(input, cx)
                .child(TextField::new(input).small())
                .into_any_element()
        };
        let mut fields = div().v_flex().gap(px(8.0));
        match sheet.mode {
            RenameMode::Replace => {
                fields = fields
                    .child(row("Find:", text("batch-find", "Find", &sheet.find)))
                    .child(row(
                        "Replace with:",
                        text("batch-replace", "Replace with", &sheet.replace),
                    ));
            }
            RenameMode::Add => {
                fields = fields.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(div().flex_1().child(text("batch-add", "Text", &sheet.add)))
                        .child(self.batch_popup(
                            "batch-add-at",
                            "Where",
                            sheet.add_at,
                            &Placement::ALL,
                            Placement::label,
                            |sheet, at| sheet.add_at = at,
                            130.0,
                            cx,
                        )),
                );
            }
            RenameMode::Format => {
                fields = fields
                    .child(row(
                        "Name Format:",
                        self.batch_popup(
                            "batch-format",
                            "Name Format",
                            sheet.format,
                            &NameFormat::ALL,
                            NameFormat::label,
                            |sheet, format| sheet.format = format,
                            170.0,
                            cx,
                        )
                        .into_any_element(),
                    ))
                    .child(row(
                        "Where:",
                        self.batch_popup(
                            "batch-format-at",
                            "Where",
                            sheet.format_at,
                            &Placement::ALL,
                            Placement::label,
                            |sheet, at| sheet.format_at = at,
                            130.0,
                            cx,
                        )
                        .into_any_element(),
                    ))
                    .child(row(
                        "Custom Format:",
                        text("batch-custom", "Custom Format", &sheet.custom),
                    ))
                    .when(sheet.format != NameFormat::Date, |fields| {
                        fields.child(row(
                            "Start numbers at:",
                            text("batch-start", "Start numbers at", &sheet.start),
                        ))
                    });
            }
        }
        let plan = sheet.plan(cx);
        let example = planned_names(&sheet.items[..1], &plan)
            .into_iter()
            .next()
            .unwrap_or_default();
        let panel = div()
            .id("batch-rename")
            .role(Role::Dialog)
            .aria_label(format!("Rename {} Items", sheet.paths.len()))
            .occlude()
            .w(px(SHEET_WIDTH))
            .v_flex()
            .gap(px(12.0))
            .p(px(18.0))
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(rmac_ui::mac::raised())
            .border_1()
            .border_color(sep())
            .shadow_lg()
            .text_size(rmac_ui::text_px(13.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(primary_text())
                            .child("Rename Finder Items:"),
                    )
                    .child(self.batch_popup(
                        "batch-mode",
                        "Rename Finder Items",
                        sheet.mode,
                        &RenameMode::ALL,
                        RenameMode::label,
                        |sheet, mode| sheet.mode = mode,
                        150.0,
                        cx,
                    )),
            )
            .child(fields)
            .child(
                div()
                    .id("batch-example")
                            .text_size(rmac_ui::text_px(12.0))
                    .text_color(secondary_text())
                    .truncate()
                    .child(format!("Example: {example}")),
            )
            .when_some(sheet.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(rmac_ui::mac::danger())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("batch-cancel", "Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_batch_rename(window, cx)
                            })),
                    )
                    .child(
                        Button::new("batch-rename-button", "Rename")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.commit_batch_rename(window, cx)
                            })),
                    ),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    cx.stop_propagation();
                    this.close_batch_rename(window, cx);
                }
            }));
        Some(
            div()
                .id("batch-rename-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(TOOLBAR_HEIGHT))
                .child(panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(mode: RenameMode) -> RenamePlan {
        RenamePlan {
            mode,
            find: String::new(),
            replace: String::new(),
            add: String::new(),
            add_at: Placement::AfterName,
            format: NameFormat::Index,
            format_at: Placement::AfterName,
            custom: String::new(),
            start: 1,
            date: "2026-10-20 at 18.37.07".to_owned(),
        }
    }

    fn items(names: &[&str]) -> Vec<(String, bool)> {
        names
            .iter()
            .map(|name| ((*name).to_owned(), !name.contains('.')))
            .collect()
    }

    #[test]
    fn replace_text_edits_every_base_name_but_not_extensions() {
        let mut plan = plan(RenameMode::Replace);
        plan.find = "IMG".to_owned();
        plan.replace = "Holiday".to_owned();
        assert_eq!(
            planned_names(&items(&["IMG_1.jpg", "IMG_2.IMG", "Notes"]), &plan),
            ["Holiday_1.jpg", "Holiday_2.IMG", "Notes"]
        );
        plan.find.clear();
        assert_eq!(planned_names(&items(&["a.txt"]), &plan), ["a.txt"]);
    }

    #[test]
    fn add_text_goes_before_or_after_the_name() {
        let mut plan = plan(RenameMode::Add);
        plan.add = " copy".to_owned();
        assert_eq!(
            planned_names(&items(&["a.txt", "Folder"]), &plan),
            ["a copy.txt", "Folder copy"]
        );
        plan.add = "2026 ".to_owned();
        plan.add_at = Placement::BeforeName;
        assert_eq!(planned_names(&items(&["a.txt"]), &plan), ["2026 a.txt"]);
    }

    #[test]
    fn formats_number_or_date_every_item() {
        let mut plan = plan(RenameMode::Format);
        plan.custom = "Photo".to_owned();
        plan.start = 7;
        assert_eq!(
            planned_names(&items(&["x.jpg", "y.png"]), &plan),
            ["Photo7.jpg", "Photo8.png"]
        );
        plan.format = NameFormat::Counter;
        plan.format_at = Placement::BeforeName;
        assert_eq!(planned_names(&items(&["x.jpg"]), &plan), ["00007Photo.jpg"]);
        plan.format = NameFormat::Date;
        plan.format_at = Placement::AfterName;
        assert_eq!(
            planned_names(&items(&["x.jpg", "y.jpg"]), &plan),
            [
                "Photo 2026-10-20 at 18.37.07.jpg",
                "Photo 2026-10-20 at 18.37.07 2.jpg"
            ]
        );
    }

    #[test]
    fn problems_are_found_before_anything_is_renamed() {
        let a = PathBuf::from("/d/a.txt");
        let b = PathBuf::from("/d/b.txt");
        let sources = [a.clone(), b.clone()];
        let none = |_: &Path| false;
        assert_eq!(
            plan_problem(&sources, &["x.txt".into(), "y.txt".into()], none),
            None
        );
        assert!(plan_problem(&sources, &["x.txt".into(), "x.txt".into()], none).is_some());
        assert!(plan_problem(&sources, &["".into(), "y.txt".into()], none).is_some());
        assert!(plan_problem(&sources, &["x/y".into(), "y.txt".into()], none).is_some());
        // Swapping two names would clobber one of them mid-way.
        assert!(plan_problem(&sources, &["b.txt".into(), "a.txt".into()], none).is_some());
        // An unchanged name is fine; a name taken by another file is not.
        assert_eq!(
            plan_problem(&sources, &["a.txt".into(), "c.txt".into()], none),
            None
        );
        assert!(plan_problem(&sources, &["a.txt".into(), "c.txt".into()], |path: &Path| {
            path == Path::new("/d/c.txt")
        })
        .is_some());
    }
}
