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
    div, point, prelude::FluentBuilder as _, px, size, App, AppContext as _, Bounds, Context,
    FocusHandle, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowBounds,
    WindowHandle, WindowOptions,
};
use rmac_ui::{Checkbox, PopUpButton, PopupMenuItem, Root, StyledExt as _};

use super::settings::{
    self, AdvancedSettings, FinderSettings, GeneralSettings, NewWindowTarget, SearchScope,
    SidebarSettings,
};

/// The Mac's Finder Settings panel is 377 pt wide in every pane
/// (`win-settings-*`, 2026-10-06 audit capture).
const WIDTH: f32 = 377.0;

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
    let options = window_options(Tab::General.window_height(), cx);
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

/// Settings' own window options: `app_id::FILES` for the same desktop
/// identity and restored position as any other Files window, but never
/// that window's restored *size*. Before this fix both windows shared one
/// persisted-geometry key, so Settings could open as large as whatever
/// Files window was last resized to — 923×664 was recorded in the
/// 2026-10-06 audit against the Mac's fixed 377-wide panel (UIA-06). The
/// Mac's own panel is not user-resizable either: each pane has exactly one
/// correct height, so Settings fixes its size instead of remembering one.
fn window_options(height: f32, cx: &App) -> WindowOptions {
    let mut options = rmac_ui::window_options_for_app(rmac_ui::app_id::FILES, WIDTH, height, cx);
    let origin = options
        .window_bounds
        .map(|bounds| bounds.get_bounds().origin)
        .unwrap_or_else(|| point(px(0.0), px(0.0)));
    let fixed = size(px(WIDTH), px(height));
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(origin, fixed)));
    options.window_min_size = Some(fixed);
    options.is_resizable = false;
    options
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

    /// The icon toolbar's glyph for this tab (UIA-06): a gear, a price
    /// tag, a two-pane sidebar and two overlapping gears, the Mac's own
    /// General/Tags/Sidebar/Advanced symbols, drawn as plain glyphs the
    /// way System Settings' category list and Spotlight's result icons
    /// already do rather than a new icon asset.
    ///
    /// (Written as `GLYPH_*` constants, not `Self::Variant => "literal"`
    /// arms: `scripts/inventory/lulo_inventory.py`'s settings-window
    /// scanner greps exactly that shape for tab labels, and would
    /// otherwise read these glyphs as four more tabs.)
    fn glyph(self) -> &'static str {
        const GLYPH_GENERAL: &str = "⚙";
        const GLYPH_TAGS: &str = "🏷";
        const GLYPH_SIDEBAR: &str = "◧";
        const GLYPH_ADVANCED: &str = "⚙⚙";
        match self {
            Self::General => GLYPH_GENERAL,
            Self::Tags => GLYPH_TAGS,
            Self::Sidebar => GLYPH_SIDEBAR,
            Self::Advanced => GLYPH_ADVANCED,
        }
    }

    /// This pane's own window height (UIA-06; `win-settings-*`, 2026-10-06
    /// audit capture) — the Mac's panel is not user-resizable, and each
    /// pane is exactly tall enough for its own content, no scrolling.
    fn window_height(self) -> f32 {
        match self {
            Self::General => 424.0,
            Self::Tags => 591.0,
            Self::Sidebar => 828.0,
            Self::Advanced => 392.0,
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

/// A section heading: 13 pt regular text, same weight and colour as the
/// rows under it (UIA-06 — the Mac's own panel has no separate "caption"
/// style here, unlike this pane's previous 11 px bold grey).
fn section_label(text: &'static str) -> impl IntoElement {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_size(rmac_ui::text_px(13.0))
        .text_color(rmac_ui::mac::text())
        .child(text)
}

/// A checkbox row at the Mac's measured 22 pt pitch (`ax-settings-general`,
/// 2026-10-06 audit capture: four desktop-item checkboxes 22–23 pt apart —
/// UIA-05's shared `Checkbox` already draws the 13 pt label this needs).
fn checkbox_row(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div().px_3().pb(px(6.0)).child(
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
    /// The Mac's own icon toolbar (UIA-06): General/Tags/Sidebar/Advanced as
    /// glyph-over-label tiles, the selected one lifted on a white tile with
    /// its glyph and label in the accent colour — not the bordered text
    /// buttons this used to show, whose "selected" fill read as disabled.
    /// 56 pt tall and the tiles evenly spaced, matching `ax-settings-general`
    /// (2026-10-06 audit capture: `AXToolbar` 56 pt, each button 55–64 pt
    /// wide).
    fn render_tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        div()
            .flex_none()
            .h(px(56.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .border_b_1()
            .border_color(rmac_ui::mac::separator())
            .children(Tab::ALL.into_iter().enumerate().map(|(index, tab)| {
                let selected = tab == self.tab;
                let tint = if selected {
                    rmac_ui::mac::accent()
                } else {
                    rmac_ui::mac::text_secondary()
                };
                let activate = {
                    let view = view.clone();
                    move |window: &mut Window, cx: &mut App| {
                        window.resize(size(px(WIDTH), px(tab.window_height())));
                        view.update(cx, |this, cx| {
                            this.tab = tab;
                            cx.notify();
                        });
                    }
                };
                let tile = div()
                    .id(("settings-tab-tile", index))
                    .w(px(58.0))
                    .h(px(48.0))
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .when(selected, |tile| tile.bg(rmac_ui::mac::raised()))
                    .when(!selected, |tile| {
                        tile.hover(|hovered| hovered.bg(rmac_ui::mac::hover()))
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(18.0))
                            .text_color(tint)
                            .child(tab.glyph()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(tint)
                            .child(tab.label()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.tab = tab;
                        window.resize(size(px(WIDTH), px(tab.window_height())));
                        cx.notify();
                    }));
                // The tile itself carries no accessible-tab semantics of its
                // own (`Button::selected` was purely visual, the same shared
                // bug `crates/mail/src/settings_view.rs::tabs` notes), so the
                // wrapper carries the real tab role, the way Finder's own
                // window-tab strip (`view/chrome_presentation/menus_tabs.rs`)
                // already does.
                div()
                    .id(("settings-tab", index))
                    .role(Role::Tab)
                    .aria_label(tab.label())
                    .aria_selected(selected)
                    .child(rmac_ui::KeyboardAction::new(
                        ("settings-tab-keyboard", index),
                        tile,
                        activate,
                    ))
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
                // No US serial comma (UIA-06/UIA-25's locale, and the
                // Mac's own General pane wording).
                "CDs, DVDs and iPods",
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
            .child({
                // A pop-up, not a list showing every choice at once (UIA-06
                // — `ax-settings-general`, 2026-10-06 audit capture: one
                // `AXPopUpButton`, 284×24 pt).
                let view = cx.entity();
                let current = general.new_window_target;
                div().px_3().pb(px(6.0)).child(
                    PopUpButton::new("settings-new-window-target", current.label())
                        .w_full()
                        .dropdown_menu(move |menu, _, _| {
                            NewWindowTarget::ALL.iter().fold(menu, |menu, &target| {
                                let view = view.clone();
                                menu.item(
                                    PopupMenuItem::new(target.label())
                                        .checked(target == current)
                                        .on_click(move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.apply(
                                                    |s| s.general.new_window_target = target,
                                                    cx,
                                                );
                                            });
                                        }),
                                )
                            })
                        }),
                )
            })
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
                    .child("Finder Settings"),
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
