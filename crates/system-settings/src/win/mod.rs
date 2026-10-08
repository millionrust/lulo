//! System Settings on Windows (ADR 0023 phase 2e).
//!
//! Lulo OS's Settings (`crate::controller`) drives Linux services pane by
//! pane, so it is `cfg(unix)`. On Windows Settings shows the six panes it
//! can really drive there, in the same window and with the same
//! measurements and style tokens:
//!
//! - **Appearance:** Lulo's light/dark/auto and accent colour, saved in the
//!   shared theme store every Lulo app reads (`rmac-theme`), which applies
//!   them at once (`rmac-ui` watches the store on Windows too).
//! - **Wallpaper:** Lulo's wallpaper for the Lulo shell
//!   (`rmac-shell-settings`), and, only when the user asks, the same
//!   picture as the Windows desktop background.
//! - **Sound:** Lulo's alert sound (`rmac-sound`) and a read-out of the
//!   Windows output volume.
//! - **Displays:** each display's resolution, refresh rate and scale.
//! - **General ▸ About This PC:** processor, memory, graphics, Windows
//!   version and drives.
//! - **Keyboard:** the shortcuts Lulo apps use on Windows, to read.
//!
//! Everything Windows-specific goes through [`host::Host`]; the panes
//! never call Win32 themselves. Nothing here polls: each pane reads what it
//! shows when it opens, and the read-outs that Windows can change behind
//! Settings' back (volume, displays) are read again when the window becomes
//! active.

mod about;
mod appearance_pane;
mod displays_pane;
mod form;
mod host;
mod keyboard_pane;
mod sound_pane;
mod wallpaper_pane;
// The measured Lulo OS Settings geometry and colours, shared as is; the
// Windows panes use only part of it.
#[allow(dead_code)]
#[path = "../controller/settings_style.rs"]
mod style;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    actions, div, prelude::FluentBuilder as _, px, AppContext as _, AssetSource, Context, Div,
    Entity, FocusHandle, Focusable as _, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, ParentElement as _, Render, RenderImage, Result, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_ui::{InputEvent, InputState, ListRow, SearchField, StyledExt as _};

use crate::navigation::Category;
use form::{glyph, label, secondary, tile};

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct SettingsIcons;

/// Settings' own icons, read straight from the binary.
struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(SettingsIcons::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(SettingsIcons::iter()
            .filter(|file| file.starts_with(path))
            .map(|file| SharedString::from(file.to_string()))
            .collect())
    }
}

// The names match Lulo OS Settings' own actions, so the shared menu table
// (`rmac-app-menu`) lists exactly the commands this binary registers: View
// shows these six panes and nothing Windows cannot open.
actions!(
    system_settings,
    [
        GoBack,
        GoForward,
        FocusSearch,
        ShowAbout,
        ShowAppearance,
        ShowDisplays,
        ShowKeyboard,
        ShowSound,
        ShowWallpaper,
        CloseAll,
        ArrangeInFront,
        Minimize,
    ]
);

/// A second `rmac-system-settings --pane <id>` launch's request, handed to
/// the running window (SET-57).
#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = system_settings, no_json)]
struct NavigateToPane {
    pane: String,
}

/// The panes Settings shows on Windows, in sidebar order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Pane {
    General,
    Appearance,
    Displays,
    Wallpaper,
    Sound,
    Keyboard,
}

impl Pane {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Appearance,
        Self::Displays,
        Self::Wallpaper,
        Self::Sound,
        Self::Keyboard,
    ];

    /// The sidebar name, which is also the `navigation::categories` name.
    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Displays => "Displays",
            Self::Wallpaper => "Wallpaper",
            Self::Sound => "Sound",
            Self::Keyboard => "Keyboard",
        }
    }

    /// The toolbar title; General shows its hero instead.
    fn title(self) -> Option<&'static str> {
        match self {
            Self::General => None,
            other => Some(other.name()),
        }
    }

    /// `--pane <id>`: the Lulo OS pane ids these panes answer to.
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "general" | "about" | "storage" => Some(Self::General),
            "appearance" => Some(Self::Appearance),
            "displays" => Some(Self::Displays),
            "wallpaper" => Some(Self::Wallpaper),
            "sound" => Some(Self::Sound),
            "keyboard" => Some(Self::Keyboard),
            _ => None,
        }
    }

    /// Extra words each pane is found by in the sidebar search, beyond its
    /// name and description: only things the pane really shows here.
    fn search_terms(self) -> &'static [&'static str] {
        match self {
            Self::General => &[
                "about this pc",
                "processor",
                "cpu",
                "memory",
                "ram",
                "graphics",
                "gpu",
                "windows version",
                "storage",
                "disk",
                "computer name",
            ],
            Self::Appearance => &[
                "dark mode",
                "light mode",
                "accent",
                "colour",
                "color",
                "theme",
            ],
            Self::Displays => &["resolution", "scale", "refresh rate", "monitor", "screen"],
            Self::Wallpaper => &["background", "desktop picture", "photo"],
            Self::Sound => &["alert", "volume", "output", "speakers", "beep"],
            Self::Keyboard => &["shortcuts", "keys", "ctrl", "hotkeys"],
        }
    }
}

/// The sidebar row for `pane`, from the shared pane inventory.
fn category(pane: Pane, categories: &[Category]) -> Option<&Category> {
    categories
        .iter()
        .find(|category| category.name.as_ref() == pane.name())
}

/// The panes a sidebar search for `query` finds, best first: a name match,
/// then a description match, then a search term.
fn search(query: &str, categories: &[Category]) -> Vec<Pane> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Pane::ALL.to_vec();
    }
    let mut ranked: Vec<(u8, usize, Pane)> = Pane::ALL
        .iter()
        .enumerate()
        .filter_map(|(order, &pane)| {
            let category = category(pane, categories)?;
            let rank = if category.name.to_lowercase().contains(&query) {
                0
            } else if category.desc.to_lowercase().contains(&query) {
                1
            } else if pane.search_terms().iter().any(|term| term.contains(&query)) {
                2
            } else {
                return None;
            };
            Some((rank, order, pane))
        })
        .collect();
    ranked.sort();
    ranked.into_iter().map(|(_, _, pane)| pane).collect()
}

/// A background read in flight or finished: `None` until it answers.
type Loaded<T> = Option<std::result::Result<T, SharedString>>;

pub(crate) struct WinSettings {
    focus: FocusHandle,
    host: Arc<dyn host::Host>,
    categories: Vec<Category>,
    current: Pane,
    back: Vec<Pane>,
    forward: Vec<Pane>,
    search: Entity<InputState>,
    search_selection: usize,
    native_window_title: String,
    /// Images replaced since the last frame, released in `render`.
    garbage: Vec<Arc<RenderImage>>,

    // Appearance
    theme: Loaded<crate::appearance::ThemeLoad>,
    theme_busy: bool,
    theme_error: Option<SharedString>,

    // Wallpaper
    shell: Loaded<rmac_shell_settings::Snapshot>,
    wallpaper_busy: bool,
    wallpaper_error: Option<SharedString>,
    wallpaper_preview: Option<(wallpaper_pane::PreviewKey, Arc<RenderImage>)>,
    wallpaper_preview_pending: Option<wallpaper_pane::PreviewKey>,
    desktop_busy: bool,
    desktop_status: Option<std::result::Result<SharedString, SharedString>>,

    // Sound
    sound: Loaded<rmac_sound::Settings>,
    sound_error: Option<SharedString>,
    volume: Loaded<host::VolumeReading>,

    // Displays and About
    displays: Loaded<Vec<host::DisplayFacts>>,
    about: Option<host::AboutFacts>,
}

impl WinSettings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rmac_system_settings::accessibility::SEARCH_NAME)
        });
        cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_selection = 0;
                cx.notify();
            }
        })
        .detach();
        // Volume and displays can change behind Settings' back; read them
        // again when the window comes forward rather than polling.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_live_read_outs(cx);
            }
        })
        .detach();
        let categories = crate::navigation::categories()
            .into_iter()
            .flatten()
            .collect();
        let initial = crate::navigation::requested_pane(&std::env::args().collect::<Vec<_>>())
            .and_then(|id| Pane::from_id(&id))
            .unwrap_or(Pane::Appearance);
        let mut settings = Self {
            focus: cx.focus_handle(),
            host: host::current(),
            categories,
            current: initial,
            back: Vec::new(),
            forward: Vec::new(),
            search,
            search_selection: 0,
            native_window_title: String::new(),
            garbage: Vec::new(),
            theme: None,
            theme_busy: false,
            theme_error: None,
            shell: None,
            wallpaper_busy: false,
            wallpaper_error: None,
            wallpaper_preview: None,
            wallpaper_preview_pending: None,
            desktop_busy: false,
            desktop_status: None,
            sound: None,
            sound_error: None,
            volume: None,
            displays: None,
            about: None,
        };
        settings.load_pane(cx);
        settings
    }

    fn category(&self, pane: Pane) -> Option<&Category> {
        category(pane, &self.categories)
    }

    // ---- navigation ------------------------------------------------------

    fn show(&mut self, pane: Pane, cx: &mut Context<Self>) {
        if pane == self.current {
            return;
        }
        self.back.push(self.current);
        self.forward.clear();
        self.current = pane;
        self.load_pane(cx);
        cx.notify();
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = self.back.pop() {
            self.forward.push(self.current);
            self.current = pane;
            self.load_pane(cx);
            cx.notify();
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = self.forward.pop() {
            self.back.push(self.current);
            self.current = pane;
            self.load_pane(cx);
            cx.notify();
        }
    }

    fn navigate_to_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(pane) = Pane::from_id(id) {
            self.show(pane, cx);
        }
    }

    /// Read what the current pane shows, the first time it opens.
    fn load_pane(&mut self, cx: &mut Context<Self>) {
        match self.current {
            Pane::Appearance if self.theme.is_none() => self.load_theme(cx),
            Pane::Wallpaper if self.shell.is_none() => self.load_wallpaper(cx),
            Pane::Sound => {
                if self.sound.is_none() {
                    self.load_sound(cx);
                }
                if self.volume.is_none() {
                    self.load_volume(cx);
                }
            }
            Pane::Displays if self.displays.is_none() => self.load_displays(cx),
            Pane::General if self.about.is_none() => self.load_about(cx),
            _ => {}
        }
    }

    fn refresh_live_read_outs(&mut self, cx: &mut Context<Self>) {
        match self.current {
            Pane::Sound => self.load_volume(cx),
            Pane::Displays => self.load_displays(cx),
            _ => {}
        }
    }

    fn load_displays(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let task = cx
            .background_executor()
            .spawn(async move { host.displays() });
        cx.spawn(async move |this, cx| {
            let displays = task.await;
            let _ = this.update(cx, |this, cx| {
                this.displays = Some(displays.map_err(SharedString::from));
                cx.notify();
            });
        })
        .detach();
    }

    fn load_about(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let task = cx.background_executor().spawn(async move { host.about() });
        cx.spawn(async move |this, cx| {
            let about = task.await;
            let _ = this.update(cx, |this, cx| {
                this.about = Some(about);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- search ----------------------------------------------------------

    fn search_results(&self, cx: &Context<Self>) -> Vec<Pane> {
        search(self.search.read(cx).value().as_ref(), &self.categories)
    }

    fn searching(&self, cx: &Context<Self>) -> bool {
        !self.search.read(cx).value().trim().is_empty()
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        self.search_selection = 0;
    }

    // ---- chrome ----------------------------------------------------------

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let searching = self.searching(cx);
        let panes = self.search_results(cx);
        let active = window.is_window_active();
        let highlight = if active {
            style::sidebar_selection_focused()
        } else {
            style::sidebar_selection()
        };
        let highlight_text = if active {
            gpui::white()
        } else {
            style::sidebar_text()
        };
        let selection = self.search_selection.min(panes.len().saturating_sub(1));

        let search = div()
            .id(rmac_system_settings::accessibility::SEARCH_ID)
            .role(Role::SearchInput)
            .aria_label("Search")
            .mx(px(style::SIDEBAR_ROW_INSET))
            .h(px(style::SEARCH_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.0))
            .pl(px(8.0))
            .pr(px(6.0))
            .rounded(px(style::SEARCH_HEIGHT / 2.0))
            .bg(style::search_fill())
            .child(glyph(
                "icons/search.svg",
                style::SEARCH_GLYPH,
                style::search_glyph(),
            ))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
            );

        let mut list = div()
            .id(rmac_system_settings::accessibility::SIDEBAR_ID)
            .role(Role::List)
            .aria_label("Settings Categories")
            .flex_1()
            .min_h(px(0.0))
            .v_flex()
            .pt(px(style::LIST_TOP_GAP + style::SIDEBAR_SECTION_GAP))
            .px(px(style::SIDEBAR_ROW_INSET))
            .pb(px(style::SIDEBAR_ROW_INSET))
            .overflow_y_scroll();
        if searching && panes.is_empty() {
            list = list.child(
                div()
                    .mt_6()
                    .v_flex()
                    .items_center()
                    .gap_1()
                    .text_center()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(style::sidebar_text())
                            .child("No Settings Found"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(secondary())
                            .child("Try a different search."),
                    ),
            );
        }
        for (index, pane) in panes.iter().copied().enumerate() {
            let Some(category) = self.category(pane) else {
                continue;
            };
            let selected = if searching {
                index == selection
            } else {
                pane == self.current
            };
            let text = if selected {
                highlight_text
            } else {
                style::sidebar_text()
            };
            list = list.child(
                ListRow::new(
                    SharedString::from(format!("cat-{}", pane.name())),
                    div()
                        .flex()
                        .items_center()
                        .w_full()
                        .gap(px(style::SIDEBAR_LABEL_X
                            - style::SIDEBAR_ICON_X
                            - style::SIDEBAR_ICON))
                        .child(tile(category.icon, category.color, style::SIDEBAR_ICON))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(rmac_ui::text_px(13.0))
                                .text_color(text)
                                .truncate()
                                .child(category.name.clone()),
                        ),
                )
                .aria_label(category.name.clone())
                .tab_stop(false)
                .active_descendant(selected)
                .selected(selected)
                .bg(if selected {
                    highlight
                } else {
                    gpui::transparent_black()
                })
                .rounded(px(style::SIDEBAR_ROW_RADIUS))
                .pl(px(style::SIDEBAR_ICON_X))
                .pr(px(6.0))
                .flex_none()
                .h(px(style::SIDEBAR_ROW_HEIGHT))
                .on_activate(cx.listener(move |this, _, window, cx| {
                    this.show(pane, cx);
                    if this.searching(cx) {
                        this.clear_search(window, cx);
                    }
                })),
            );
        }

        let lights_origin = rmac_ui::traffic_lights_origin(true) - style::SIDEBAR_INSET;
        let panel = div()
            .relative()
            .size_full()
            .v_flex()
            .rounded(px(style::SIDEBAR_RADIUS))
            .bg(style::sidebar_panel())
            .border_1()
            .border_color(style::sidebar_panel_edge())
            .overflow_hidden()
            .child(
                rmac_ui::title_bar_drag_region("sidebar-header")
                    .h(px(style::SEARCH_TOP))
                    .flex_none()
                    .w_full(),
            )
            .child(search)
            .child(list)
            .child(
                div()
                    .absolute()
                    .left(px(lights_origin))
                    .top(px(lights_origin))
                    .child(rmac_ui::traffic_lights()),
            );
        div()
            .h_full()
            .flex_shrink_0()
            .w(px(style::SIDEBAR_COLUMN_WIDTH))
            .pl(px(style::SIDEBAR_INSET))
            .py(px(style::SIDEBAR_INSET))
            .child(panel)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_back = !self.back.is_empty();
        let can_forward = !self.forward.is_empty();
        let segment = |id: &'static str, path: &'static str, name: &'static str, enabled: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(name)
                .w(px(style::CAPSULE_SEGMENT))
                .h(px(style::CAPSULE_SEGMENT))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |segment| segment.cursor_pointer())
                .child(glyph(
                    path,
                    style::CAPSULE_GLYPH,
                    style::toolbar_glyph(enabled),
                ))
        };
        let capsule = div()
            .h(px(style::CAPSULE_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(style::CAPSULE_HEIGHT / 2.0))
            .bg(style::capsule_fill())
            .border_1()
            .border_color(style::capsule_edge())
            .child(
                segment(
                    rmac_system_settings::accessibility::BACK_ID,
                    "icons/chevron-left.svg",
                    "Back",
                    can_back,
                )
                .when(can_back, |back| {
                    back.on_click(cx.listener(|this, _, _, cx| this.go_back(cx)))
                }),
            )
            .child(
                div()
                    .w(px(1.0))
                    .h(px(style::CAPSULE_DIVIDER_HEIGHT))
                    .bg(style::capsule_divider()),
            )
            .child(
                segment(
                    "nav-forward",
                    "icons/chevron-right.svg",
                    "Forward",
                    can_forward,
                )
                .when(can_forward, |forward| {
                    forward.on_click(cx.listener(|this, _, _, cx| this.go_forward(cx)))
                }),
            );
        rmac_ui::title_bar_drag_region("topbar")
            .role(Role::Toolbar)
            .aria_label("Toolbar")
            .h(px(style::TOOLBAR_HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .pl(px(style::CAPSULE_LEADING))
            .child(capsule)
            .when_some(self.current.title(), |bar, title| {
                bar.child(
                    div()
                        .ml(px(style::TITLE_GAP))
                        .min_w_0()
                        .truncate()
                        .text_size(rmac_ui::text_px(style::TITLE_SIZE))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(style::title_text())
                        .child(title),
                )
            })
    }

    /// General's hero: the big tile, the pane's name and what it is for.
    fn render_hero(&self, pane: Pane) -> Option<Div> {
        let category = self.category(pane)?;
        Some(
            div()
                .v_flex()
                .items_center()
                .min_h(px(style::HERO_HEIGHT))
                .mb(px(style::GROUP_GAP))
                .pt(px(style::HERO_ICON_TOP))
                .pb(px(style::DETAIL_INSET))
                .px(px(style::ROW_PADDING))
                .rounded(px(style::GROUP_RADIUS))
                .bg(style::group_fill())
                .child(tile(category.icon, category.color, style::HERO_ICON))
                .child(
                    div()
                        .mt(px(10.0))
                        .text_size(rmac_ui::text_px(style::HERO_TITLE))
                        .line_height(px(26.0))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(label())
                        .child(category.name.clone()),
                )
                .child(
                    div()
                        .max_w(px(style::HERO_TEXT_WIDTH))
                        .text_center()
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(16.0))
                        .text_color(secondary())
                        .child(category.desc.clone()),
                ),
        )
    }

    fn render_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.current == Pane::Wallpaper {
            return self.render_wallpaper(window, cx).into_any_element();
        }
        let content = match self.current {
            Pane::General => self.render_about(cx),
            Pane::Appearance => self.render_appearance(cx),
            Pane::Displays => self.render_displays(cx),
            Pane::Sound => self.render_sound(cx),
            Pane::Keyboard => self.render_keyboard(),
            Pane::Wallpaper => unreachable!("Wallpaper lays out its own column"),
        };
        let content = match self.current {
            Pane::General => div()
                .v_flex()
                .children(self.render_hero(Pane::General))
                .child(content),
            _ => div()
                .v_flex()
                .pt(px(style::FIRST_SECTION_TOP))
                .child(content),
        };
        div()
            .id(rmac_system_settings::accessibility::DETAIL_ID)
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(style::DETAIL_CONTENT_WIDTH + 2.0 * style::DETAIL_INSET))
                    .mx_auto()
                    .px(px(style::DETAIL_INSET))
                    .pb(px(style::DETAIL_INSET))
                    .child(content),
            )
            .into_any_element()
    }
}

/// The accent swatch colour from `0xRRGGBB`.
fn hex(value: u32) -> Hsla {
    gpui::rgb(value).into()
}

impl Render for WinSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.garbage.drain(..) {
            cx.drop_image(image, Some(window));
        }
        rmac_ui::set_menu_enabled("system_settings::GoBack", !self.back.is_empty(), cx);
        rmac_ui::set_menu_enabled("system_settings::GoForward", !self.forward.is_empty(), cx);
        let subject = match self.current {
            Pane::General => "About This PC",
            other => other.name(),
        };
        let title = rmac_ui::native_window_title(subject, "Settings");
        if self.native_window_title != title {
            window.set_window_title(&title);
            self.native_window_title = title;
        }
        let sidebar = self.render_sidebar(window, cx);
        let toolbar = self.render_toolbar(cx);
        let detail = self.render_detail(window, cx);
        div()
            .id(rmac_system_settings::accessibility::ROOT_ID)
            .size_full()
            .relative()
            .flex()
            .font_features(rmac_ui::mac::tabular_font_features())
            .track_focus(&self.focus)
            .key_context("SystemSettings")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.searching(cx) {
                    return;
                }
                let results = this.search_results(cx);
                let handled = match event.keystroke.key.as_str() {
                    "down" => {
                        this.search_selection =
                            (this.search_selection + 1).min(results.len().saturating_sub(1));
                        true
                    }
                    "up" => {
                        this.search_selection = this.search_selection.saturating_sub(1);
                        true
                    }
                    "enter" => {
                        if let Some(&pane) = results.get(this.search_selection) {
                            this.show(pane, cx);
                            this.clear_search(window, cx);
                        }
                        true
                    }
                    "escape" => {
                        this.clear_search(window, cx);
                        true
                    }
                    _ => false,
                };
                if handled {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, action: &NavigateToPane, _, cx| {
                this.navigate_to_id(&action.pane, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAbout, _, cx| this.show(Pane::General, cx)))
            .on_action(
                cx.listener(|this, _: &ShowAppearance, _, cx| this.show(Pane::Appearance, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowDisplays, _, cx| this.show(Pane::Displays, cx)))
            .on_action(cx.listener(|this, _: &ShowKeyboard, _, cx| this.show(Pane::Keyboard, cx)))
            .on_action(cx.listener(|this, _: &ShowSound, _, cx| this.show(Pane::Sound, cx)))
            .on_action(cx.listener(|this, _: &ShowWallpaper, _, cx| this.show(Pane::Wallpaper, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                let focus = this.search.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }))
            .on_action(cx.listener(|_, _: &CloseAll, window, cx| {
                window.dispatch_action(Box::new(rmac_ui::RequestClose), cx);
            }))
            .on_action(cx.listener(|_, _: &ArrangeInFront, window, _| {
                window.activate_window();
            }))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &Minimize, _, cx| {
                rmac_ui::minimize_focused_window(cx);
            }))
            .bg(style::window_fill())
            .text_color(label())
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .v_flex()
                    .child(toolbar)
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .w_full()
                            .v_flex()
                            .child(detail),
                    ),
            )
    }
}

/// Open System Settings on Windows.
pub(crate) fn run() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    rmac_ui::boot_unified_single_window_app_with_assets(
        rmac_ui::app_id::SYSTEM_SETTINGS,
        rmac_ui::layered_assets(Icons),
        style::WINDOW_WIDTH,
        style::WINDOW_HEIGHT,
        arguments,
        |window, cx| {
            let context = Some("SystemSettings");
            // `rmac_ui::bind_keys` also binds each ⌘ chord's Ctrl twin,
            // which is what Windows keyboards press (ADR 0023).
            rmac_ui::bind_keys(
                cx,
                [
                    KeyBinding::new(rmac_ui::shortcuts::BACK.keystroke, GoBack, context),
                    KeyBinding::new("cmd-]", GoForward, context),
                    KeyBinding::new("cmd-f", FocusSearch, context),
                    KeyBinding::new(
                        rmac_ui::shortcuts::CLOSE.keystroke,
                        rmac_ui::RequestClose,
                        context,
                    ),
                    KeyBinding::new("cmd-q", rmac_ui::RequestClose, context),
                    KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, context),
                    KeyBinding::new("cmd-m", Minimize, context),
                ],
            );
            let settings = WinSettings::new(window, cx);
            let search_focus = settings.search.read(cx).focus_handle(cx);
            window.focus(&search_focus, cx);
            rmac_ui::register_menu_target(window, &settings.focus, cx);
            // `cx.activate(true)` does nothing on Windows (ADR 0023,
            // WIN-OS-13): bring the new window forward itself.
            window.activate_window();
            settings
        },
        |arguments, cx| {
            if let Some(pane) = crate::navigation::requested_pane(&arguments) {
                rmac_ui::dispatch_to_app_window(Box::new(NavigateToPane { pane }), cx);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn categories() -> Vec<Category> {
        crate::navigation::categories()
            .into_iter()
            .flatten()
            .collect()
    }

    #[test]
    fn every_windows_pane_has_a_sidebar_row() {
        let categories = categories();
        assert_eq!(categories.len(), Pane::ALL.len());
        for pane in Pane::ALL {
            assert!(category(pane, &categories).is_some(), "{pane:?}");
        }
    }

    #[test]
    fn lulo_os_pane_ids_route_to_the_windows_panes() {
        assert_eq!(Pane::from_id("about"), Some(Pane::General));
        assert_eq!(Pane::from_id("appearance"), Some(Pane::Appearance));
        assert_eq!(Pane::from_id("keyboard"), Some(Pane::Keyboard));
        assert_eq!(Pane::from_id("wifi"), None);
        assert_eq!(Pane::from_id("bluetooth"), None);
    }

    #[test]
    fn search_ranks_names_first_and_finds_only_what_the_panes_show() {
        let categories = categories();
        assert_eq!(search("", &categories), Pane::ALL.to_vec());
        assert_eq!(search("sound", &categories)[0], Pane::Sound);
        assert_eq!(search("processor", &categories), vec![Pane::General]);
        assert_eq!(search("resolution", &categories)[0], Pane::Displays);
        assert!(search("bluetooth", &categories).is_empty());
        assert!(search("microphone", &categories).is_empty());
    }
}
