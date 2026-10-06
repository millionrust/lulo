//! Folder-scoped Finder View Options. State changes are applied on the UI thread;
//! the existing quiet-period Finder persistence worker writes them off-thread.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum GroupBy {
    None,
    Name,
    Kind,
    Date,
    Size,
}
impl GroupBy {
    fn next(self) -> Self {
        match self {
            Self::None => Self::Name,
            Self::Name => Self::Kind,
            Self::Kind => Self::Date,
            Self::Date => Self::Size,
            Self::Size => Self::None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Name => "Name",
            Self::Kind => "Kind",
            Self::Date => "Date Modified",
            Self::Size => "Size",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Background {
    Default,
    Colour,
    Picture,
}
impl Background {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Colour => "Colour",
            Self::Picture => "Picture",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FolderOptions {
    pub(super) preferred_view: Option<ViewMode>,
    pub(super) view: ViewMode,
    pub(super) browse_in_view: bool,
    pub(super) group_by: GroupBy,
    pub(super) sort_by: SortKey,
    pub(super) icon_size: f32,
    pub(super) grid_spacing: f32,
    pub(super) text_size: u8,
    pub(super) list_text_size: u8,
    pub(super) label_right: bool,
    pub(super) show_item_info: bool,
    pub(super) show_icon_preview: bool,
    pub(super) background: Background,
    pub(super) picture_path: Option<PathBuf>,
    pub(super) list_large_icons: bool,
    /// Date Modified, Date Created, Date Last Opened, Date Added, Size, Kind,
    /// Version, Comments, Tags, in that order.
    pub(super) columns: [bool; 9],
    pub(super) relative_dates: bool,
    pub(super) calculate_sizes: bool,
}
impl Default for FolderOptions {
    fn default() -> Self {
        Self {
            preferred_view: None,
            view: ViewMode::List,
            browse_in_view: false,
            group_by: GroupBy::None,
            sort_by: SortKey::Name,
            icon_size: 64.0,
            grid_spacing: 54.0,
            text_size: ICON_LABEL_SIZE as u8,
            list_text_size: 13,
            label_right: false,
            show_item_info: false,
            show_icon_preview: true,
            background: Background::Default,
            picture_path: None,
            list_large_icons: false,
            columns: [true, false, false, false, true, true, false, false, false],
            relative_dates: true,
            calculate_sizes: false,
        }
    }
}
impl FolderOptions {
    pub(super) fn valid(&self) -> bool {
        self.icon_size.is_finite()
            && (32.0..=128.0).contains(&self.icon_size)
            && self.grid_spacing.is_finite()
            && (0.0..=100.0).contains(&self.grid_spacing)
            && (10..=20).contains(&self.text_size)
            && (10..=20).contains(&self.list_text_size)
            && self
                .picture_path
                .as_ref()
                .is_none_or(|path| path.is_absolute() && path.as_os_str().len() <= 4096)
    }
}

#[derive(Clone, Copy)]
enum Field {
    Preferred,
    Browse,
    ItemInfo,
    Preview,
    RelativeDates,
    CalculateSizes,
    Column(usize),
}

pub(super) fn group_title(entry: &Entry, group: GroupBy) -> Option<String> {
    match group {
        GroupBy::None => None,
        GroupBy::Name => Some(
            entry
                .name
                .chars()
                .next()
                .unwrap_or('#')
                .to_uppercase()
                .to_string(),
        ),
        GroupBy::Kind => Some(entry.kind.to_string()),
        GroupBy::Date => Some(
            entry
                .modified
                .split(" at ")
                .next()
                .unwrap_or("Other")
                .to_string(),
        ),
        GroupBy::Size => Some(
            if entry.is_dir {
                "Folders"
            } else if entry.size_bytes < 1_000_000 {
                "Small files"
            } else if entry.size_bytes < 100_000_000 {
                "Medium files"
            } else {
                "Large files"
            }
            .to_string(),
        ),
    }
}
fn directory_size(root: PathBuf, cancelled: &AtomicBool) -> Option<u64> {
    let mut stack = vec![root];
    let mut total = 0_u64;
    while let Some(directory) = stack.pop() {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(children) = std::fs::read_dir(directory) else {
            continue;
        };
        for child in children.flatten() {
            if cancelled.load(Ordering::Relaxed) {
                return None;
            }
            let Ok(metadata) = std::fs::symlink_metadata(child.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(child.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Some(total)
}

pub(super) fn group_entries(entries: &mut [Entry], group: GroupBy) {
    if group != GroupBy::None {
        // Stable sort keeps the selected Sort By order within each group.
        entries.sort_by_cached_key(|entry| group_title(entry, group));
    }
}

impl FinderView {
    pub(super) fn start_size_scan(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.size_scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.directory_sizes.clear();
        if !self.current_options().calculate_sizes {
            cx.notify();
            return;
        }
        let directories = self
            .entries
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        if directories.is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.size_scan_cancel = Some(cancel.clone());
        let path = self.cwd.clone();
        let generation = self.directory_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let worker_cancel = cancel.clone();
            let sizes = cx
                .background_executor()
                .spawn(async move {
                    directories
                        .into_iter()
                        .filter_map(|directory| {
                            directory_size(directory.clone(), &worker_cancel)
                                .map(|size| (directory, size))
                        })
                        .collect::<std::collections::HashMap<_, _>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.cwd == path
                    && this.directory_generation == generation
                    && !cancel.load(Ordering::Relaxed)
                {
                    this.directory_sizes = sizes;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn current_options(&self) -> FolderOptions {
        self.folder_options
            .get(&self.cwd)
            .cloned()
            .unwrap_or_else(|| self.default_options.clone())
    }
    pub(super) fn change_options(
        &mut self,
        change: impl Fn(&mut FolderOptions),
        cx: &mut Context<Self>,
    ) {
        let mut options = self.current_options();
        let calculate_was_enabled = options.calculate_sizes;
        change(&mut options);
        if !options.valid() {
            return;
        }
        self.icon_size = options.icon_size;
        if self.sort_key != options.sort_by || self.current_options().group_by != options.group_by {
            self.sort_key = options.sort_by;
            if !self.trash_view && !self.applications_view && self.search_summary.is_none() {
                sort_entries(&mut self.root_entries, self.sort_key, self.sort_asc);
                group_entries(&mut self.root_entries, options.group_by);
                for rows in self.child_entries.values_mut() {
                    sort_entries(rows, self.sort_key, self.sort_asc);
                    group_entries(rows, options.group_by);
                }
                self.rebuild_list_entries();
            } else {
                let selected = self.selected_paths().into_iter().collect::<BTreeSet<_>>();
                sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
                group_entries(&mut self.entries, options.group_by);
                self.selected = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| selected.contains(&entry.path).then_some(index))
                    .collect();
                self.anchor = self.selected.iter().next().copied();
            }
        }
        let calculate_is_enabled = options.calculate_sizes;
        if self.folder_options.len() >= 128 && !self.folder_options.contains_key(&self.cwd) {
            if let Some(oldest) = self.folder_options.keys().next().cloned() {
                self.folder_options.remove(&oldest);
            }
        }
        self.folder_options.insert(self.cwd.clone(), options);
        if calculate_was_enabled != calculate_is_enabled {
            self.start_size_scan(cx);
        }
        self.persist_finder_state();
        cx.notify();
    }
    fn set_field(&mut self, field: Field, value: bool, cx: &mut Context<Self>) {
        let view = self.view;
        self.change_options(
            |o| match field {
                Field::Preferred => o.preferred_view = value.then_some(view),
                Field::Browse => o.browse_in_view = value,
                Field::ItemInfo => o.show_item_info = value,
                Field::Preview => o.show_icon_preview = value,
                Field::RelativeDates => o.relative_dates = value,
                Field::CalculateSizes => o.calculate_sizes = value,
                Field::Column(i) => o.columns[i] = value,
            },
            cx,
        );
    }
    pub(super) fn change_icon_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.change_options(|o| o.icon_size = size, cx);
    }
    pub(super) fn toggle_view_options(&mut self, cx: &mut Context<Self>) {
        if self.view_options_open {
            self.close_view_options(cx);
            return;
        }
        self.view_options_open = true;
        let height = if self.view == ViewMode::List {
            646.0
        } else {
            632.0
        };
        let title = format!("{} View Options", self.title());
        let owner = cx.entity().downgrade();
        // Opening a window renders it immediately. Defer until this Finder
        // update ends, since its utility view reads the owner's controls.
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            cx.update(|cx| {
                let (width, height) = rmac_ui::outer_window_size(236.0, height);
                // Not `window_options_for_app_with_title`: View Options
                // would then inherit whatever size the main Files window
                // last saved under the same app_id (the UIA-06/UIA-09
                // window-geometry-key bug).
                let mut options = rmac_ui::window_options_for_panel_with_title(
                    rmac_ui::app_id::FILES,
                    title,
                    width,
                    height,
                    cx,
                );
                options.focus = false;
                options.kind = gpui::WindowKind::Floating;
                let view_owner = owner.clone();
                let opened = cx.open_window(options, move |window, cx| {
                    rmac_ui::prepare_surface_window(window, cx);
                    let view = cx.new(|cx| ViewOptionsWindow {
                        owner: view_owner,
                        focus: cx.focus_handle(),
                    });
                    cx.new(|cx| rmac_ui::shell_surface_root(view, window, cx))
                });
                let _ = owner.update(cx, |this, cx| {
                    match opened {
                        Ok(handle) => this.view_options_window = Some(handle),
                        Err(_) => {
                            this.view_options_open = false;
                            this.operation_error = Some("Files could not open View Options".into());
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn close_view_options(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.view_options_window.take() {
            let _ = cx.update_window(*handle, |_, window, _| window.remove_window());
        }
        self.view_options_open = false;
        cx.notify();
    }
    pub(super) fn restore_folder_options(&mut self, cx: &mut Context<Self>) {
        if self.options_path.as_ref() == Some(&self.cwd) {
            return;
        }
        self.options_path = Some(self.cwd.clone());
        if let Some(cancel) = self.size_scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.directory_sizes.clear();
        let stored = self.folder_options.contains_key(&self.cwd);
        let inherited = self.browse_view.take();
        let o = self.current_options();
        self.view = o.preferred_view.unwrap_or_else(|| {
            if stored {
                o.view
            } else {
                inherited.unwrap_or(o.view)
            }
        });
        self.icon_size = o.icon_size;
        self.sort_key = o.sort_by;
        self.icon_size_slider = cx.new(|_| {
            SliderState::new()
                .min(32.0)
                .max(128.0)
                .step(4.0)
                .default_value(o.icon_size)
        });
        cx.subscribe(
            &self.icon_size_slider,
            |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    this.change_icon_size(value.start().clamp(32.0, 128.0), cx);
                }
            },
        )
        .detach();
        self.grid_spacing_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(100.0)
                .step(2.0)
                .default_value(o.grid_spacing)
        });
        cx.subscribe(
            &self.grid_spacing_slider,
            |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    this.change_options(|o| o.grid_spacing = value.start().clamp(0.0, 100.0), cx);
                }
            },
        )
        .detach();
    }
    fn checkbox(
        &self,
        id: String,
        label: &'static str,
        checked: bool,
        field: Field,
        cx: &mut Context<Self>,
    ) -> rmac_ui::Checkbox {
        let entity = cx.entity();
        rmac_ui::Checkbox::new(SharedString::from(id))
            .label(label)
            .checked(checked)
            .on_change(move |value, _, cx| {
                let value = *value;
                entity.update(cx, |this, cx| this.set_field(field, value, cx));
            })
    }
    fn choice(
        &self,
        id: String,
        label: &'static str,
        selected: bool,
        choose: impl Fn(&mut FolderOptions) + 'static,
        cx: &mut Context<Self>,
    ) -> rmac_ui::Radio {
        let entity = cx.entity();
        rmac_ui::Radio::new(SharedString::from(id))
            .label(label)
            .selected(selected)
            .on_change(move |_, _, cx| {
                entity.update(cx, |this, cx| this.change_options(&choose, cx));
            })
    }
    fn cycle(
        &self,
        id: &'static str,
        label: &'static str,
        value: String,
        change: impl Fn(&mut FolderOptions) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(label)
            .child(
                Button::new(id, format!("{value}  ⌄"))
                    .xsmall()
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| this.change_options(&change, cx));
                    }),
            )
    }
    pub(super) fn render_view_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.current_options();
        let mode = self.view;
        let name = self.title();
        let mut panel = div()
            .id("finder-view-options")
            .role(Role::Group)
            .key_context("Finder")
            .aria_label(format!("{name} View Options"))
            .w(px(236.0))
            .h(px(if mode == ViewMode::List { 646.0 } else { 632.0 }))
            .rounded(px(rmac_ui::mac::radius_large_surface()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .bg(rmac_ui::mac::raised())
            .shadow_lg()
            .overflow_hidden()
            .v_flex()
            .text_size(rmac_ui::text_px(12.0))
            .text_color(rmac_ui::mac::text());
        panel = panel.child(
            div()
                .relative()
                .h(px(26.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div().absolute().left(px(5.0)).top(px(2.0)).child(
                        Button::new("vo-close", "×")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.view_options_open = false;
                                this.view_options_window = None;
                                window.remove_window();
                                cx.notify();
                            })),
                    ),
                )
                .child(name),
        );
        panel = panel.child(
            div()
                .px_3()
                .pb_2()
                .border_b_1()
                .border_color(rmac_ui::mac::separator())
                .child(self.checkbox(
                    "vo-always".into(),
                    match mode {
                        ViewMode::Icon => "Always open in icon view",
                        ViewMode::List => "Always open in list view",
                        ViewMode::Column => "Always open in column view",
                        ViewMode::Gallery => "Always open in gallery view",
                    },
                    o.preferred_view == Some(mode),
                    Field::Preferred,
                    cx,
                ))
                .child(self.checkbox(
                    "vo-browse".into(),
                    match mode {
                        ViewMode::Icon => "Browse in icon view",
                        ViewMode::List => "Browse in list view",
                        ViewMode::Column => "Browse in column view",
                        ViewMode::Gallery => "Browse in gallery view",
                    },
                    o.browse_in_view,
                    Field::Browse,
                    cx,
                )),
        );
        panel = panel.child(
            div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rmac_ui::mac::separator())
                .child(self.cycle(
                    "vo-group",
                    "Group By:",
                    o.group_by.label().into(),
                    |o| o.group_by = o.group_by.next(),
                    cx,
                ))
                .child(
                    self.cycle(
                        "vo-sort",
                        "Sort By:",
                        match o.sort_by {
                            SortKey::Name => "Name",
                            SortKey::Kind => "Kind",
                            SortKey::LastOpened => "Date Last Opened",
                            SortKey::Added => "Date Added",
                            SortKey::Date => "Date Modified",
                            SortKey::Size => "Size",
                            SortKey::Tags => "Tags",
                            SortKey::Created => "Date Created",
                        }
                        .into(),
                        |o| {
                            o.sort_by = match o.sort_by {
                                SortKey::Name => SortKey::Kind,
                                SortKey::Kind => SortKey::LastOpened,
                                SortKey::LastOpened => SortKey::Added,
                                SortKey::Added => SortKey::Date,
                                SortKey::Date => SortKey::Size,
                                SortKey::Size => SortKey::Tags,
                                SortKey::Tags => SortKey::Created,
                                SortKey::Created => SortKey::Name,
                            }
                        },
                        cx,
                    ),
                ),
        );
        if mode == ViewMode::Icon {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .child(format!(
                        "Icon size:  {} × {}",
                        o.icon_size as u32, o.icon_size as u32
                    ))
                    .child(
                        Slider::new(&self.icon_size_slider)
                            .horizontal()
                            .accessible_name("Icon size"),
                    )
                    .child("Grid spacing:")
                    .child(
                        Slider::new(&self.grid_spacing_slider)
                            .horizontal()
                            .accessible_name("Grid spacing"),
                    ),
            );
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .child(self.cycle(
                        "vo-text",
                        "Text size:",
                        o.text_size.to_string(),
                        |o| {
                            o.text_size = if o.text_size >= 20 {
                                10
                            } else {
                                o.text_size + 1
                            }
                        },
                        cx,
                    ))
                    .child("Label position:")
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.choice(
                                "vo-bottom".into(),
                                "Bottom",
                                !o.label_right,
                                |o| o.label_right = false,
                                cx,
                            ))
                            .child(self.choice(
                                "vo-right".into(),
                                "Right",
                                o.label_right,
                                |o| o.label_right = true,
                                cx,
                            )),
                    ),
            );
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .child(self.checkbox(
                        "vo-info".into(),
                        "Show item info",
                        o.show_item_info,
                        Field::ItemInfo,
                        cx,
                    ))
                    .child(self.checkbox(
                        "vo-preview".into(),
                        "Show icon preview",
                        o.show_icon_preview,
                        Field::Preview,
                        cx,
                    )),
            );
            let mut background = div()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(rmac_ui::mac::separator())
                .child("Background:");
            let selected_picture = self.selected_entry().and_then(|entry| {
                entry
                    .path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .filter(|extension| {
                        ["png", "jpg", "jpeg", "gif", "webp", "bmp"]
                            .contains(&extension.to_ascii_lowercase().as_str())
                    })
                    .map(|_| entry.path.clone())
            });
            for kind in [Background::Default, Background::Colour, Background::Picture] {
                let picture = selected_picture.clone();
                background = background.child(self.choice(
                    format!("vo-background-{}", kind.label()),
                    kind.label(),
                    o.background == kind,
                    move |o| {
                        o.background = kind;
                        if kind == Background::Picture {
                            if let Some(path) = picture.clone() {
                                o.picture_path = Some(path);
                            }
                        }
                    },
                    cx,
                ));
            }
            panel = panel.child(
                background.child(
                    div()
                        .text_size(rmac_ui::text_px(10.0))
                        .text_color(rmac_ui::mac::text_secondary())
                        .child("Select an image, then choose Picture"),
                ),
            );
        } else if mode == ViewMode::List {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .child("Icon size:")
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.choice(
                                "vo-small".into(),
                                "Small",
                                !o.list_large_icons,
                                |o| o.list_large_icons = false,
                                cx,
                            ))
                            .child(self.choice(
                                "vo-large".into(),
                                "Large",
                                o.list_large_icons,
                                |o| o.list_large_icons = true,
                                cx,
                            )),
                    )
                    .child(self.cycle(
                        "vo-text",
                        "Text size:",
                        o.list_text_size.to_string(),
                        |o| {
                            o.list_text_size = if o.list_text_size >= 20 {
                                10
                            } else {
                                o.list_text_size + 1
                            }
                        },
                        cx,
                    )),
            );
            let mut columns = div()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(rmac_ui::mac::separator())
                .child("Show Columns:");
            for (i, label) in [
                "Date Modified",
                "Date Created",
                "Date Last Opened",
                "Date Added",
                "Size",
                "Kind",
                "Version",
                "Comments",
                "Tags",
            ]
            .into_iter()
            .enumerate()
            {
                columns = columns.child(self.checkbox(
                    format!("vo-column-{i}"),
                    label,
                    o.columns[i],
                    Field::Column(i),
                    cx,
                ));
            }
            panel = panel.child(columns).child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rmac_ui::mac::separator())
                    .child(self.checkbox(
                        "vo-relative".into(),
                        "Use relative dates",
                        o.relative_dates,
                        Field::RelativeDates,
                        cx,
                    ))
                    .child(self.checkbox(
                        "vo-sizes".into(),
                        "Calculate all sizes",
                        o.calculate_sizes,
                        Field::CalculateSizes,
                        cx,
                    ))
                    .child(self.checkbox(
                        "vo-preview".into(),
                        "Show icon preview",
                        o.show_icon_preview,
                        Field::Preview,
                        cx,
                    )),
            );
        } else {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .child(self.cycle(
                        "vo-text",
                        "Text size:",
                        o.text_size.to_string(),
                        |o| {
                            o.text_size = if o.text_size >= 20 {
                                10
                            } else {
                                o.text_size + 1
                            }
                        },
                        cx,
                    ))
                    .child(self.checkbox(
                        "vo-preview".into(),
                        "Show icon preview",
                        o.show_icon_preview,
                        Field::Preview,
                        cx,
                    )),
            );
        }
        panel.child(div().flex_1()).child(
            div().py_2().flex().justify_center().child(
                Button::new("vo-defaults", "Use as Defaults")
                    .xsmall()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.default_options = this.current_options();
                        this.persist_finder_state();
                        cx.notify();
                    })),
            ),
        )
    }
}

/// Non-modal utility window. Its controls still update the owning Finder
/// view, so selecting files and changing folders remain available behind it.
struct ViewOptionsWindow {
    owner: gpui::WeakEntity<FinderView>,
    focus: FocusHandle,
}

impl ViewOptionsWindow {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |owner, cx| {
            owner.view_options_open = false;
            owner.view_options_window = None;
            cx.notify();
        });
        window.remove_window();
    }
}

impl gpui::Focusable for ViewOptionsWindow {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ViewOptionsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self
            .owner
            .update(cx, |owner, cx| owner.render_view_options(cx))
            .ok();
        div()
            .size_full()
            .key_context("Finder")
            .on_action(cx.listener(|this, _: &ShowViewOptions, window, cx| this.close(window, cx)))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.close(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    this.close(window, cx);
                }
            }))
            .when_some(panel, |container, panel| container.child(panel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calculate_sizes_skips_symlink_cycles() {
        let root =
            std::env::temp_dir().join(format!("rmac-view-options-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/file"), b"12345").unwrap();
        std::os::unix::fs::symlink(&root, root.join("nested/cycle")).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(directory_size(root.clone(), &cancel), Some(5));
        std::fs::remove_dir_all(root).unwrap();
    }
}
