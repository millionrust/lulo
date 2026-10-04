//! Finder ▸ Settings… (⌘,): General, Tags, Sidebar and Advanced, laid out
//! in `design-lab/finder-settings.html` from the written spec (the Mac was
//! locked when this shipped, so that mock is not yet measured against a
//! screenshot — see the note at its top). One global window (not tied to
//! any particular Files window), matching `rmac-terminal`'s own Settings…
//! singleton (`crates/terminal/src/settings_window.rs`).
//!
//! Every edit here calls [`settings::update`], which saves it and asks
//! every open Files window to rebuild its sidebar immediately — there is
//! no separate "Apply"/"OK" step.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle,
};
use rmac_ui::{Button, Checkbox, Root, StyledExt as _};

use super::settings::{
    self, AdvancedSettings, FinderSettings, GeneralSettings, NewWindowTarget, SearchScope,
    SidebarSettings,
};

const WIDTH: f32 = 420.0;
const HEIGHT: f32 = 460.0;

thread_local! {
    static OPEN: std::cell::Cell<Option<WindowHandle<Root>>> = const { std::cell::Cell::new(None) };
}

/// Finder ▸ Settings… (⌘,). Opens the one Settings window, or brings the
/// already-open one to the front.
pub(super) fn show(cx: &mut App) {
    if let Some(handle) = OPEN.with(std::cell::Cell::get) {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::FILES, WIDTH, HEIGHT, cx);
    let opened = cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        let view = cx.new(|cx| SettingsView::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => OPEN.with(|open| open.set(Some(handle))),
        Err(error) => eprintln!("rmac-files: could not open Settings: {error}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Tags,
    Sidebar,
    Advanced,
}

impl Tab {
    const ALL: [Self; 4] = [Self::General, Self::Tags, Self::Sidebar, Self::Advanced];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Tags => "Tags",
            Self::Sidebar => "Sidebar",
            Self::Advanced => "Advanced",
        }
    }
}

struct SettingsView {
    focus: FocusHandle,
    tab: Tab,
    file_words: rmac_locale::FileVocabulary,
    settings: FinderSettings,
    save_error: Option<SharedString>,
}

impl SettingsView {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            tab: Tab::General,
            file_words: rmac_locale::FileVocabulary::from_environment(),
            settings: settings::current(),
            save_error: None,
        }
    }

    fn apply(&mut self, edit: impl FnOnce(&mut FinderSettings), cx: &mut Context<Self>) {
        self.save_error = settings::update(edit, cx)
            .err()
            .map(|error| SharedString::from(error.to_string()));
        self.settings = settings::current();
        cx.notify();
    }
}

fn section_label(text: &'static str) -> impl IntoElement {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rmac_ui::mac::text_secondary())
        .child(text)
}

fn checkbox_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div().px_3().pb_2().child(
        Checkbox::new(id)
            .label(label)
            .checked(checked)
            .on_change(move |value, window, cx| on_change(*value, window, cx)),
    )
}

/// One row of an exclusive-choice popup list (the General tab's "New Finder
/// windows show:" and the Advanced tab's "When performing a search:").
fn choice_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    first: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(28.0))
        .px_2()
        .text_size(px(12.0))
        .text_color(rmac_ui::mac::text())
        .when(!first, |row| {
            row.border_t_1().border_color(rmac_ui::mac::separator())
        })
        .hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
        .child(div().flex_1().child(label.into()))
        .when(selected, |row| {
            row.child(div().text_color(rmac_ui::mac::accent()).child("✓"))
        })
        .on_click(on_click)
}

impl SettingsView {
    fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .flex()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rmac_ui::mac::separator())
            .children(Tab::ALL.into_iter().enumerate().map(|(index, tab)| {
                let selected = tab == self.tab;
                // See `crates/mail/src/settings_view.rs::tabs` (same fix,
                // same shared bug): `Button::selected` is a visual-only
                // highlight, so the strip itself needs the real tab
                // semantics, the way Finder's own window-tab strip
                // (`view/chrome_presentation/menus_tabs.rs`) already has.
                div()
                    .id(("settings-tab", index))
                    .role(Role::Tab)
                    .aria_label(tab.label())
                    .aria_selected(selected)
                    .child(
                        Button::new(("settings-tab-button", index), tab.label())
                            .small()
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tab = tab;
                                cx.notify();
                            })),
                    )
            }))
    }

    fn render_general(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let general: GeneralSettings = self.settings.general;
        div()
            .child(section_label("Show these items on the desktop:"))
            .child(checkbox_row(
                "settings-desktop-hard-disks",
                "Hard disks",
                general.show_hard_disks_on_desktop,
                general_listener(cx, |general, value| {
                    general.show_hard_disks_on_desktop = value
                }),
            ))
            .child(checkbox_row(
                "settings-desktop-external-disks",
                "External disks",
                general.show_external_disks_on_desktop,
                general_listener(cx, |general, value| {
                    general.show_external_disks_on_desktop = value
                }),
            ))
            .child(checkbox_row(
                "settings-desktop-cds-dvds",
                "CDs, DVDs, and iPods",
                general.show_cds_dvds_on_desktop,
                general_listener(cx, |general, value| {
                    general.show_cds_dvds_on_desktop = value
                }),
            ))
            .child(checkbox_row(
                "settings-desktop-servers",
                "Connected servers",
                general.show_connected_servers_on_desktop,
                general_listener(cx, |general, value| {
                    general.show_connected_servers_on_desktop = value
                }),
            ))
            .child(section_label("New Finder windows show:"))
            .child(
                div().px_3().v_flex().child(
                    div()
                        .rounded(px(rmac_ui::mac::radius_control()))
                        .border_1()
                        .border_color(rmac_ui::mac::separator())
                        .overflow_hidden()
                        .children(NewWindowTarget::ALL.into_iter().enumerate().map(
                            |(index, target)| {
                                choice_row(
                                    ("settings-new-window", index),
                                    target.label(),
                                    target == general.new_window_target,
                                    index == 0,
                                    cx.listener(move |this, _, _, cx| {
                                        this.apply(|s| s.general.new_window_target = target, cx);
                                    }),
                                )
                            },
                        )),
                ),
            )
            .child(checkbox_row(
                "settings-open-folders-in-tabs",
                "Open folders in tabs instead of new windows",
                general.open_folders_in_tabs,
                general_listener(cx, |general, value| general.open_folders_in_tabs = value),
            ))
    }

    fn render_tags(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().child(section_label("Tags")).child(
            div().px_3().v_flex().child(
                div()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .border_1()
                    .border_color(rmac_ui::mac::separator())
                    .overflow_hidden()
                    .children(self.settings.tags.iter().enumerate().map(|(index, tag)| {
                        div()
                            .id(("settings-tag", index))
                            .flex()
                            .items_center()
                            .gap_2()
                            .h(px(28.0))
                            .px_2()
                            .when(index > 0, |row| {
                                row.border_t_1().border_color(rmac_ui::mac::separator())
                            })
                            .child(
                                div()
                                    .w(px(12.0))
                                    .h(px(12.0))
                                    .rounded_full()
                                    .bg(gpui::rgb(tag.color)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.0))
                                    .text_color(rmac_ui::mac::text())
                                    .child(tag.display_name()),
                            )
                            .child(
                                Checkbox::new(("settings-tag-sidebar", index))
                                    .label("Show in sidebar")
                                    .checked(tag.show_in_sidebar)
                                    .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                        let value = *value;
                                        this.apply(
                                            |s| {
                                                if let Some(tag) = s.tags.get_mut(index) {
                                                    tag.show_in_sidebar = value;
                                                }
                                            },
                                            cx,
                                        );
                                    })),
                            )
                    })),
            ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar: SidebarSettings = self.settings.sidebar;
        div()
            .child(section_label("Show these items in the sidebar:"))
            .child(checkbox_row(
                "settings-sidebar-recents",
                "Recents",
                sidebar.show_recents,
                sidebar_listener(cx, |sidebar, value| sidebar.show_recents = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-applications",
                "Applications",
                sidebar.show_applications,
                sidebar_listener(cx, |sidebar, value| sidebar.show_applications = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-desktop",
                "Desktop",
                sidebar.show_desktop,
                sidebar_listener(cx, |sidebar, value| sidebar.show_desktop = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-documents",
                "Documents",
                sidebar.show_documents,
                sidebar_listener(cx, |sidebar, value| sidebar.show_documents = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-downloads",
                "Downloads",
                sidebar.show_downloads,
                sidebar_listener(cx, |sidebar, value| sidebar.show_downloads = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-home",
                "Home",
                sidebar.show_home,
                sidebar_listener(cx, |sidebar, value| sidebar.show_home = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-bin",
                self.file_words.bin(),
                sidebar.show_bin,
                sidebar_listener(cx, |sidebar, value| sidebar.show_bin = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-hard-disks",
                "Hard disks",
                sidebar.show_hard_disks,
                sidebar_listener(cx, |sidebar, value| sidebar.show_hard_disks = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-external-disks",
                "External disks",
                sidebar.show_external_disks,
                sidebar_listener(cx, |sidebar, value| sidebar.show_external_disks = value),
            ))
            .child(checkbox_row(
                "settings-sidebar-tags",
                "Tags…",
                sidebar.show_tags,
                sidebar_listener(cx, |sidebar, value| sidebar.show_tags = value),
            ))
    }

    fn render_advanced(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let advanced: AdvancedSettings = self.settings.advanced;
        div()
            .child(checkbox_row(
                "settings-show-extensions",
                "Show all filename extensions",
                advanced.show_all_filename_extensions,
                advanced_listener(cx, |advanced, value| {
                    advanced.show_all_filename_extensions = value
                }),
            ))
            .child(checkbox_row(
                "settings-warn-extension",
                "Show warning before changing an extension",
                advanced.warn_before_changing_extension,
                advanced_listener(cx, |advanced, value| {
                    advanced.warn_before_changing_extension = value
                }),
            ))
            .child(checkbox_row(
                "settings-warn-empty-bin",
                format!("Show warning before emptying the {}", self.file_words.bin()),
                advanced.warn_before_emptying_bin,
                advanced_listener(cx, |advanced, value| {
                    advanced.warn_before_emptying_bin = value
                }),
            ))
            .child(checkbox_row(
                "settings-remove-after-30-days",
                format!(
                    "Remove items from the {} after 30 days",
                    self.file_words.bin()
                ),
                advanced.remove_items_from_bin_after_30_days,
                advanced_listener(cx, |advanced, value| {
                    advanced.remove_items_from_bin_after_30_days = value
                }),
            ))
            .child(section_label("Keep folders on top:"))
            .child(checkbox_row(
                "settings-folders-on-top-windows",
                "in windows, when sorting by name",
                advanced.keep_folders_on_top_in_windows,
                advanced_listener(cx, |advanced, value| {
                    advanced.keep_folders_on_top_in_windows = value
                }),
            ))
            .child(checkbox_row(
                "settings-folders-on-top-desktop",
                "on Desktop",
                advanced.keep_folders_on_top_on_desktop,
                advanced_listener(cx, |advanced, value| {
                    advanced.keep_folders_on_top_on_desktop = value
                }),
            ))
            .child(section_label("When performing a search:"))
            .child(
                div().px_3().v_flex().child(
                    div()
                        .rounded(px(rmac_ui::mac::radius_control()))
                        .border_1()
                        .border_color(rmac_ui::mac::separator())
                        .overflow_hidden()
                        .children(SearchScope::ALL.into_iter().enumerate().map(
                            |(index, scope)| {
                                choice_row(
                                    ("settings-search-scope", index),
                                    scope.label(),
                                    scope == advanced.when_performing_search,
                                    index == 0,
                                    cx.listener(move |this, _, _, cx| {
                                        this.apply(
                                            |s| s.advanced.when_performing_search = scope,
                                            cx,
                                        );
                                    }),
                                )
                            },
                        )),
                ),
            )
    }
}

/// Shorthand for the common case of one checkbox editing one field of one
/// of the four settings groups: builds a `Checkbox::on_change` closure that
/// applies `edit` to that group and saves.
fn general_listener(
    cx: &mut Context<SettingsView>,
    edit: impl Fn(&mut GeneralSettings, bool) + Copy + 'static,
) -> impl Fn(bool, &mut Window, &mut App) + 'static {
    let entity = cx.entity();
    move |value, _, cx| {
        entity.update(cx, |this, cx| {
            this.apply(|s| edit(&mut s.general, value), cx);
        });
    }
}

fn sidebar_listener(
    cx: &mut Context<SettingsView>,
    edit: impl Fn(&mut SidebarSettings, bool) + Copy + 'static,
) -> impl Fn(bool, &mut Window, &mut App) + 'static {
    let entity = cx.entity();
    move |value, _, cx| {
        entity.update(cx, |this, cx| {
            this.apply(|s| edit(&mut s.sidebar, value), cx);
        });
    }
}

fn advanced_listener(
    cx: &mut Context<SettingsView>,
    edit: impl Fn(&mut AdvancedSettings, bool) + Copy + 'static,
) -> impl Fn(bool, &mut Window, &mut App) + 'static {
    let entity = cx.entity();
    move |value, _, cx| {
        entity.update(cx, |this, cx| {
            this.apply(|s| edit(&mut s.advanced, value), cx);
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::General => self.render_general(cx).into_any_element(),
            Tab::Tags => self.render_tags(cx).into_any_element(),
            Tab::Sidebar => self.render_sidebar(cx).into_any_element(),
            Tab::Advanced => self.render_advanced(cx).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .bg(rmac_ui::mac::window())
            .child(rmac_ui::title_bar_content(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("Settings"),
            ))
            .child(self.render_tab_strip(cx))
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body)
                    .when_some(self.save_error.clone(), |scroll, error| {
                        scroll.child(
                            div()
                                .px_3()
                                .pb_3()
                                .text_size(px(11.0))
                                .text_color(rmac_ui::mac::danger())
                                .child(error),
                        )
                    }),
            )
    }
}

impl super::FinderView {
    /// Finder ▸ Settings… (⌘,).
    pub(super) fn show_settings(&mut self, cx: &mut Context<Self>) {
        show(cx);
    }
}
