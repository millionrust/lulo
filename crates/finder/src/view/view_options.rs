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
    pub(super) browse_in_view: bool,
    pub(super) group_by: GroupBy,
    pub(super) sort_by: SortKey,
    pub(super) icon_size: f32,
    pub(super) grid_spacing: f32,
    pub(super) text_size: u8,
    pub(super) label_right: bool,
    pub(super) show_item_info: bool,
    pub(super) show_icon_preview: bool,
    pub(super) background: Background,
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
            browse_in_view: false,
            group_by: GroupBy::None,
            sort_by: SortKey::Name,
            icon_size: 64.0,
            grid_spacing: 54.0,
            text_size: 13,
            label_right: false,
            show_item_info: false,
            show_icon_preview: true,
            background: Background::Default,
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
    }
}

#[derive(Clone, Copy)]
enum Field {
    Preferred,
    Browse,
    ItemInfo,
    Preview,
    LargeIcons,
    RelativeDates,
    CalculateSizes,
    Column(usize),
}
impl FinderView {
    pub(super) fn current_options(&self) -> FolderOptions {
        self.folder_options
            .get(&self.cwd)
            .cloned()
            .unwrap_or_else(|| self.default_options.clone())
    }
    pub(super) fn change_options(
        &mut self,
        change: impl FnOnce(&mut FolderOptions),
        cx: &mut Context<Self>,
    ) {
        let mut options = self.current_options();
        change(&mut options);
        if !options.valid() {
            return;
        }
        self.icon_size = options.icon_size;
        self.sort_key = options.sort_by;
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        self.folder_options.insert(self.cwd.clone(), options);
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
                Field::LargeIcons => o.list_large_icons = value,
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
        self.view_options_open = !self.view_options_open;
        cx.notify();
    }
    pub(super) fn restore_folder_options(&mut self) {
        if self.options_path.as_ref() == Some(&self.cwd) {
            return;
        }
        self.options_path = Some(self.cwd.clone());
        let o = self.current_options();
        if let Some(view) = o.preferred_view {
            self.view = view;
        }
        self.icon_size = o.icon_size;
        self.sort_key = o.sort_by;
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
                entity.update(cx, |this, cx| this.change_options(choose, cx));
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
                        entity.update(cx, |this, cx| this.change_options(change, cx));
                    }),
            )
    }
    pub(super) fn render_view_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.current_options();
        let mode = self.view;
        let name = self.title();
        let mut panel = div()
            .id("finder-view-options")
            .role(Role::Dialog)
            .aria_label(format!("{name} View Options"))
            .absolute()
            .top(px(34.0))
            .right(px(16.0))
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
                .h(px(26.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
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
                            SortKey::Date => "Date Modified",
                            SortKey::Size => "Size",
                            SortKey::Kind => "Kind",
                        }
                        .into(),
                        |o| {
                            o.sort_by = match o.sort_by {
                                SortKey::Name => SortKey::Date,
                                SortKey::Date => SortKey::Size,
                                SortKey::Size => SortKey::Kind,
                                SortKey::Kind => SortKey::Name,
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
                    .child(self.cycle(
                        "vo-grid",
                        "Grid spacing:",
                        format!("{}", o.grid_spacing as u32),
                        |o| {
                            o.grid_spacing = if o.grid_spacing >= 100.0 {
                                0.0
                            } else {
                                o.grid_spacing + 10.0
                            }
                        },
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
            for kind in [Background::Default, Background::Colour, Background::Picture] {
                background = background.child(self.choice(
                    format!("vo-background-{}", kind.label()),
                    kind.label(),
                    o.background == kind,
                    move |o| o.background = kind,
                    cx,
                ));
            }
            panel = panel.child(background);
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
                        o.text_size.to_string(),
                        |o| {
                            o.text_size = if o.text_size >= 20 {
                                10
                            } else {
                                o.text_size + 1
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
