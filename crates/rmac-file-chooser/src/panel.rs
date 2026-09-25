//! The Open/Save panel window: state and behaviour. Rendering is in
//! `panel_view.rs`; every measured number comes from `metrics`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_channel::{Receiver, Sender};
use gpui::{
    actions, px, size, AppContext as _, Context, Entity, FocusHandle, Focusable as _, KeyBinding,
    SharedString, Subscription, Window,
};
use rmac_file_chooser::browser::{Browser, Location, Policy, ViewMode};
use rmac_file_chooser::filter::{CompiledFilter, MimeDatabase};
use rmac_file_chooser::goto;
use rmac_file_chooser::metrics;
use rmac_file_chooser::outcome::{
    confirm_open, save_files_targets, save_target, Outcome, Selection,
};
use rmac_file_chooser::request::{Choice, Mode, Request};
use rmac_finder::listing::{read_directory, Item, SortKey};
use rmac_ui::{InputEvent, InputState};

actions!(
    file_chooser,
    [
        Accept,
        Cancel,
        GoToFolder,
        GoDesktop,
        GoHome,
        GoEnclosing,
        GoBack,
        GoForward,
        ViewAsIcons,
        ViewAsList,
        FocusSearch,
        NewFolder,
        SelectAllItems,
        ToggleHidden,
    ]
);

pub const CONTEXT: &str = "FileChooser";

pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        KeyBinding::new("enter", Accept, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("cmd-.", Cancel, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-g", GoToFolder, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-d", GoDesktop, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-h", GoHome, Some(CONTEXT)),
        KeyBinding::new("cmd-up", GoEnclosing, Some(CONTEXT)),
        KeyBinding::new("cmd-[", GoBack, Some(CONTEXT)),
        KeyBinding::new("cmd-]", GoForward, Some(CONTEXT)),
        KeyBinding::new("cmd-1", ViewAsIcons, Some(CONTEXT)),
        KeyBinding::new("cmd-2", ViewAsList, Some(CONTEXT)),
        KeyBinding::new("cmd-f", FocusSearch, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-n", NewFolder, Some(CONTEXT)),
        KeyBinding::new("cmd-a", SelectAllItems, Some(CONTEXT)),
        KeyBinding::new("cmd-shift-.", ToggleHidden, Some(CONTEXT)),
    ]);
}

#[derive(Clone, Debug)]
pub struct SidebarPlace {
    pub name: SharedString,
    pub location: Location,
    pub icon: &'static str,
}

#[derive(Clone, Debug)]
pub struct SidebarSection {
    pub title: SharedString,
    pub places: Vec<SidebarPlace>,
}

/// Which in-window pop-up menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Where,
    View,
    Sort,
    Filter,
    Choice(usize),
}

#[derive(Clone, Debug)]
pub enum MenuAction {
    Go(PathBuf),
    View(ViewMode),
    Sort(SortKey),
    ToggleHidden,
    Filter(usize),
    Choice(usize, String),
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub checked: bool,
    /// `None` renders a separator.
    pub action: Option<MenuAction>,
}

pub struct GoToSheet {
    pub input: Entity<InputState>,
    pub suggestions: Vec<PathBuf>,
    pub highlighted: Option<usize>,
    pub error: bool,
}

/// The Mac's New Folder sheet: a name field defaulted to "untitled folder",
/// asked before anything is created (OTHER-10).
pub struct NewFolderSheet {
    pub input: Entity<InputState>,
    pub folder: PathBuf,
    pub error: bool,
}

pub struct Panel {
    pub(crate) request: Request,
    reply: Option<Sender<Outcome>>,
    pub(crate) home: PathBuf,
    pub(crate) browser: Browser,
    compiled: Vec<CompiledFilter>,
    pub(crate) filter_index: Option<usize>,
    pub(crate) choices: Vec<Choice>,
    pub(crate) expanded: bool,
    pub(crate) name: Option<Entity<InputState>>,
    pub(crate) search: Entity<InputState>,
    pub(crate) goto: Option<GoToSheet>,
    pub(crate) new_folder_sheet: Option<NewFolderSheet>,
    pub(crate) menu: Option<MenuKind>,
    pub(crate) replace: Option<PathBuf>,
    pub(crate) notice: Option<SharedString>,
    pub(crate) focus: FocusHandle,
    pub(crate) sections: Vec<SidebarSection>,
    pending_select: Option<PathBuf>,
    search_cancel: Option<Arc<AtomicBool>>,
    /// The folder a search started from, so clearing the field returns there.
    search_origin: Option<PathBuf>,
    /// Quick Look opened with Space from the file list.
    quick_look: Option<rmac_quick_look::Handle>,
    _subscriptions: Vec<Subscription>,
}

/// The byte range of `name`'s base name, excluding a trailing extension —
/// Finder's Save As selection. A leading dot (`dot == 0`, a dotfile) has no
/// extension to protect, so the whole name is selected instead.
fn base_name_selection(name: &str) -> std::ops::Range<usize> {
    match name.rfind('.') {
        Some(dot) if dot > 0 => 0..dot,
        _ => 0..name.len(),
    }
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn sidebar_sections(home: &Path) -> Vec<SidebarSection> {
    let words = rmac_locale::FileVocabulary::from_environment();
    let place = |spec: rmac_finder::places::PlaceSpec| SidebarPlace {
        name: spec.name.into(),
        location: Location::Folder(spec.path),
        icon: spec.icon,
    };
    let mut first = vec![SidebarPlace {
        name: "Recents".into(),
        location: Location::Recents,
        icon: "icons/clock.svg",
    }];
    first.extend(rmac_finder::places::shared_folder(home).map(place));
    // The Mac's Favourites opens with Applications, then the user's folders.
    let favourites = rmac_finder::places::applications_folder()
        .into_iter()
        .chain(
            rmac_finder::places::favourite_folders(home)
                .into_iter()
                .filter(|spec| spec.path.is_dir()),
        )
        .map(place)
        .collect();
    let mut locations: Vec<SidebarPlace> = rmac_finder::places::standard_locations(home)
        .into_iter()
        .map(place)
        .collect();
    if let Ok(mounts) = rmac_mounts::discover() {
        locations.extend(mounts.into_iter().map(|mount| SidebarPlace {
            name: mount.name.into(),
            location: Location::Folder(mount.path),
            icon: "icons/hard-drive.svg",
        }));
    }
    let media = rmac_finder::places::media_folders(home)
        .into_iter()
        .map(place)
        .collect();
    vec![
        SidebarSection {
            title: SharedString::default(),
            places: first,
        },
        SidebarSection {
            title: words.favourites().to_string().into(),
            places: favourites,
        },
        SidebarSection {
            title: "Locations".into(),
            places: locations,
        },
        // Tags is omitted here: Linux has no tag store rmac can write
        // (docs/decisions/0012-file-chooser-portal.md).
        SidebarSection {
            title: "Media".into(),
            places: media,
        },
    ]
}

impl Panel {
    pub fn new(
        request: Request,
        reply: Sender<Outcome>,
        closed: Receiver<()>,
        database: Arc<MimeDatabase>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = home_directory();
        let start = request
            .initial_folder()
            .filter(|folder| folder.is_dir())
            .or_else(|| {
                let downloads = home.join("Downloads");
                (request.mode == Mode::Open && downloads.is_dir()).then_some(downloads)
            })
            .or_else(|| {
                let documents = home.join("Documents");
                documents.is_dir().then_some(documents)
            })
            .unwrap_or_else(|| home.clone());
        let mut browser = Browser::new(start);
        if request.mode != Mode::Open {
            browser.view = ViewMode::List;
        }
        let compiled = request
            .filters
            .iter()
            .map(|filter| CompiledFilter::compile(filter, &database))
            .collect();

        let mut subscriptions = Vec::new();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, input, event: &InputEvent, _window, cx| match event {
                InputEvent::Change | InputEvent::PressEnter { .. } => {
                    let query = input.read(cx).value().trim().to_owned();
                    this.search_changed(query, cx);
                }
                _ => {}
            },
        ));

        let name = matches!(request.mode, Mode::Save).then(|| {
            let initial = request.initial_name();
            cx.new(|cx| InputState::new(window, cx).default_value(initial))
        });
        if let Some(name) = &name {
            subscriptions.push(cx.subscribe_in(
                name,
                window,
                |this: &mut Self, _input, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.accept(window, cx);
                    }
                },
            ));
        }

        let focus = cx.focus_handle();
        match &name {
            Some(name) => {
                let name_focus = name.read(cx).focus_handle(cx);
                window.focus(&name_focus, cx);
                let name = name.clone();
                window.on_next_frame(move |window, cx| {
                    window.focus(&name_focus, cx);
                    // Finder's Save As convention: select only the base
                    // name ("Untitled", not "Untitled.txt"), so typing
                    // replaces it and leaves the extension alone (OTHER-09).
                    name.update(cx, |state, cx| {
                        let selection = base_name_selection(&state.value());
                        state.set_selected_range(selection, cx);
                    });
                });
            }
            None => window.focus(&focus, cx),
        }

        // Request.Close() from the portal frontend closes the panel without
        // a reply (the adapter answers the call itself).
        cx.spawn_in(window, async move |this, cx| {
            if closed.recv().await.is_ok() {
                let _ = this.update_in(cx, |this, window, _cx| {
                    this.reply = None;
                    window.remove_window();
                });
            }
        })
        .detach();

        let pending_select = request.current_file.clone();
        let mut panel = Self {
            sections: sidebar_sections(&home),
            filter_index: request.current_filter,
            choices: request.choices.clone(),
            request,
            reply: Some(reply),
            home,
            browser,
            compiled,
            expanded: false,
            name,
            search,
            goto: None,
            new_folder_sheet: None,
            menu: None,
            replace: None,
            quick_look: None,
            notice: None,
            focus,
            pending_select,
            search_cancel: None,
            search_origin: None,
            _subscriptions: subscriptions,
        };
        panel.load(cx);
        panel
    }

    pub fn mode(&self) -> Mode {
        self.request.mode
    }

    /// Save shows the compact sheet until the disclosure button expands it.
    pub fn is_compact(&self) -> bool {
        self.request.mode == Mode::Save && !self.expanded
    }

    /// Save header rows above the browser: Save As, File Format (when the app
    /// offers more than one), then one row per application choice — where
    /// the Mac puts an app's accessory view. Tags is omitted (no tag store).
    pub fn header_rows(&self) -> usize {
        match self.request.mode {
            Mode::Save => 1 + usize::from(self.shows_format_row()) + self.choices.len(),
            _ => 0,
        }
    }

    /// The File Format row appears only when the app offers a choice.
    pub fn shows_format_row(&self) -> bool {
        self.request.filters.len() > 1
    }

    pub fn window_size(&self) -> (f32, f32) {
        if self.is_compact() {
            (
                metrics::COMPACT_WIDTH,
                metrics::compact_height(self.header_rows() + 1),
            )
        } else if self.request.mode == Mode::Save {
            (
                metrics::PANEL_WIDTH,
                metrics::expanded_height(self.header_rows()),
            )
        } else {
            (metrics::PANEL_WIDTH, metrics::PANEL_HEIGHT)
        }
    }

    pub fn policy(&self) -> Policy {
        Policy {
            mode: self.request.mode,
            directory: self.request.directory || self.request.mode == Mode::SaveFiles,
            multiple: self.request.multiple,
            filter: self
                .filter_index
                .and_then(|index| self.compiled.get(index).cloned())
                .unwrap_or_else(CompiledFilter::everything),
        }
    }

    pub fn filter_name(&self) -> SharedString {
        self.filter_index
            .and_then(|index| self.request.filters.get(index))
            .map(|filter| SharedString::from(filter.name.clone()))
            .unwrap_or_else(|| "All Files".into())
    }

    // ---- listing -------------------------------------------------------

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let generation = self.browser.generation();
        let location = self.browser.location().clone();
        let home = self.home.clone();
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match location {
                        Location::Folder(folder) => read_directory(&folder, true).map_err(|_| {
                            format!(
                                "The folder “{}” can’t be opened.",
                                rmac_file_chooser::browser::display_name(&folder)
                            )
                        }),
                        Location::Recents => {
                            let options = rmac_search::Options::new(&cancel);
                            rmac_search::recents(options)
                                .map(|paths| {
                                    let mut items: Vec<(Item, std::time::SystemTime)> = paths
                                        .iter()
                                        .filter_map(|path| Item::from_path(path))
                                        .map(|item| {
                                            let when = item.mtime;
                                            (item, when)
                                        })
                                        .collect();
                                    items.sort_by_key(|(_, when)| std::cmp::Reverse(*when));
                                    items.truncate(200);
                                    items.into_iter().map(|(item, _)| item).collect()
                                })
                                .map_err(|_| "Recent files are unavailable.".to_owned())
                        }
                        Location::Search(query) => {
                            let options = rmac_search::Options::new(&cancel);
                            rmac_search::filenames(&home, &query, options)
                                .map(|paths| {
                                    paths
                                        .iter()
                                        .take(500)
                                        .filter_map(|path| Item::from_path(path))
                                        .collect()
                                })
                                .map_err(|_| "Search is unavailable.".to_owned())
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |this: &mut Panel, cx| {
                let policy = this.policy();
                let select = this.pending_select.take();
                match result {
                    Ok(items) => {
                        this.browser
                            .set_items(generation, items, &policy, select.as_deref());
                    }
                    Err(message) => {
                        if this.browser.generation() == generation {
                            this.notice = Some(message.into());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn navigate(&mut self, location: Location, cx: &mut Context<Self>) {
        self.menu = None;
        if !matches!(location, Location::Search(_)) {
            self.search_origin = None;
        }
        if self.browser.navigate(location) {
            self.load(cx);
        }
    }

    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.browser.go_back() {
            self.load(cx);
        }
    }

    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.browser.go_forward() {
            self.load(cx);
        }
    }

    pub fn go_enclosing(&mut self, cx: &mut Context<Self>) {
        let current = self.browser.location().folder().map(Path::to_path_buf);
        if self.browser.go_enclosing() {
            // Like Finder, the folder we came from stays selected.
            self.pending_select = current;
            self.load(cx);
        }
    }

    fn search_changed(&mut self, query: String, cx: &mut Context<Self>) {
        if query.is_empty() {
            if matches!(self.browser.location(), Location::Search(_)) {
                let origin = self
                    .search_origin
                    .take()
                    .unwrap_or_else(|| self.home.clone());
                self.navigate(Location::Folder(origin), cx);
            }
            return;
        }
        if self.search_origin.is_none() {
            self.search_origin = self.browser.location().folder().map(Path::to_path_buf);
        }
        let origin = self.search_origin.clone();
        self.browser.navigate(Location::Search(query));
        self.search_origin = origin;
        self.load(cx);
    }

    // ---- selection and activation ---------------------------------------

    pub fn click_row(
        &mut self,
        index: usize,
        toggle: bool,
        extend: bool,
        double: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        window.focus(&self.focus, cx);
        self.browser
            .click(index, toggle, extend, self.request.multiple);
        if double {
            self.open_row(index, window, cx);
        }
        cx.notify();
    }

    fn open_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.browser.rows().get(index).cloned() else {
            return;
        };
        if !row.enabled {
            return;
        }
        if row.item.is_dir {
            self.navigate(Location::Folder(row.item.path), cx);
        } else if self.policy().choosable(&row.item) {
            self.browser.select_only(index);
            self.accept(window, cx);
        }
    }

    /// Whether the default button is enabled.
    pub fn can_accept(&self, cx: &gpui::App) -> bool {
        match self.request.mode {
            Mode::Open if self.request.directory => {
                self.browser.selected_folder().is_some()
                    || self.browser.location().folder().is_some()
            }
            Mode::Open => {
                let policy = self.policy();
                self.browser.selected_folder().is_some()
                    || self
                        .browser
                        .selected_rows()
                        .any(|row| policy.choosable(&row.item))
            }
            Mode::Save => {
                self.save_folder().is_some()
                    && self
                        .name
                        .as_ref()
                        .is_some_and(|name| !name.read(cx).value().trim().is_empty())
            }
            Mode::SaveFiles => self.save_folder().is_some(),
        }
    }

    fn save_folder(&self) -> Option<PathBuf> {
        if self.request.mode == Mode::SaveFiles {
            if let Some(folder) = self.browser.selected_folder() {
                return Some(folder);
            }
        }
        self.browser.location().folder().map(Path::to_path_buf)
    }

    fn selection(&self, paths: Vec<PathBuf>) -> Selection {
        Selection {
            paths,
            choices: self
                .choices
                .iter()
                .map(|choice| (choice.id.clone(), choice.selected.clone()))
                .collect(),
            current_filter: self.filter_index,
        }
    }

    pub fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if self.new_folder_sheet.is_some() {
            self.new_folder_commit(window, cx);
            return;
        }
        if self.goto.is_some() {
            self.goto_commit(window, cx);
            return;
        }
        match self.request.mode {
            Mode::Open => {
                let policy = self.policy();
                let chosen: Vec<PathBuf> = self
                    .browser
                    .selected_rows()
                    .filter(|row| policy.choosable(&row.item))
                    .map(|row| row.item.path.clone())
                    .collect();
                if !self.request.directory && chosen.is_empty() {
                    // Open on a selected folder browses into it.
                    if let Some(folder) = self.browser.selected_folder() {
                        self.navigate(Location::Folder(folder), cx);
                    }
                    return;
                }
                let chosen = if chosen.is_empty() {
                    match self.browser.location().folder() {
                        Some(folder) => vec![folder.to_path_buf()],
                        None => return,
                    }
                } else {
                    chosen
                };
                match confirm_open(&chosen, self.request.directory, self.request.multiple) {
                    Ok(paths) => self.finish(self.selection(paths), window),
                    Err(rejection) => {
                        self.notice = Some(rejection.to_string().into());
                        cx.notify();
                    }
                }
            }
            Mode::Save => {
                let Some(folder) = self.save_folder() else {
                    return;
                };
                let name = self
                    .name
                    .as_ref()
                    .map(|name| name.read(cx).value().trim().to_owned())
                    .unwrap_or_default();
                match save_target(&folder, &name) {
                    Ok(target) if target.symlink_metadata().is_ok() && self.replace.is_none() => {
                        self.replace = Some(target);
                        cx.notify();
                    }
                    Ok(target) => {
                        self.replace = None;
                        self.finish(self.selection(vec![target]), window);
                    }
                    Err(rejection) => {
                        self.notice = Some(rejection.to_string().into());
                        cx.notify();
                    }
                }
            }
            Mode::SaveFiles => {
                let Some(folder) = self.save_folder() else {
                    return;
                };
                match save_files_targets(&folder, &self.request.files) {
                    Ok(targets) => self.finish(self.selection(targets), window),
                    Err(rejection) => {
                        self.notice = Some(rejection.to_string().into());
                        cx.notify();
                    }
                }
            }
        }
    }

    pub fn confirm_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.replace.take() {
            if target.parent().is_some_and(Path::is_dir) {
                self.finish(self.selection(vec![target]), window);
            } else {
                cx.notify();
            }
        }
    }

    pub fn dismiss_replace(&mut self, cx: &mut Context<Self>) {
        self.replace = None;
        cx.notify();
    }

    fn finish(&mut self, selection: Selection, window: &mut Window) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.try_send(Outcome::Chosen(selection));
        }
        window.remove_window();
    }

    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
            return;
        }
        if self.new_folder_sheet.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.goto.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        if self.replace.take().is_some() {
            cx.notify();
            return;
        }
        if let Some(reply) = self.reply.take() {
            let _ = reply.try_send(Outcome::Cancelled);
        }
        window.remove_window();
    }

    // ---- keyboard ---------------------------------------------------------

    pub fn icon_columns(&self, content_width: f32) -> usize {
        let first = metrics::ICON_FIRST_X - (metrics::ICON_LABEL_WIDTH - metrics::ICON_SIZE) / 2.0;
        let usable = content_width - 2.0 * first - metrics::ICON_LABEL_WIDTH;
        ((usable / metrics::ICON_PITCH_X).floor().max(0.0) as usize) + 1
    }

    /// Arrow keys and type-to-select while the file list has focus.
    pub fn list_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        content_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.focus.is_focused(window)
            || self.goto.is_some()
            || self.replace.is_some()
            || self.new_folder_sheet.is_some()
        {
            return false;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if keystroke.key == "space" && !modifiers.platform && !modifiers.control && !modifiers.alt {
            self.toggle_quick_look(cx);
            return true;
        }
        let columns = match self.browser.view {
            ViewMode::Icons => self.icon_columns(content_width) as isize,
            ViewMode::List => 1,
        };
        let step = match keystroke.key.as_str() {
            "down" if !modifiers.platform => Some(columns),
            "up" if !modifiers.platform => Some(-columns),
            "right" if self.browser.view == ViewMode::Icons => Some(1),
            "left" if self.browser.view == ViewMode::Icons => Some(-1),
            "down" if modifiers.platform => {
                // ⌘↓ opens the selection, as in Finder.
                if let Some(index) = self.browser.anchor() {
                    self.open_row(index, window, cx);
                }
                return true;
            }
            _ => None,
        };
        if let Some(step) = step {
            self.browser.move_selection(step);
            cx.notify();
            return true;
        }
        if modifiers.platform || modifiers.control || modifiers.alt {
            return false;
        }
        if let Some(text) = keystroke.key_char.as_ref() {
            if !text.is_empty() && text.chars().all(|c| !c.is_control()) && text != " " {
                self.browser.type_select(text, Instant::now());
                cx.notify();
                return true;
            }
        }
        false
    }

    /// Space: Quick Look on the selection, as in Finder (Space again closes).
    pub fn toggle_quick_look(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.quick_look.take() {
            if handle.is_open() {
                handle.close(cx);
                return;
            }
        }
        let paths = self.browser.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.quick_look = rmac_quick_look::open(paths, 0, rmac_quick_look::Options::default(), cx)
            .map(|(handle, _)| handle);
    }

    // ---- toolbar ----------------------------------------------------------

    pub fn set_view(&mut self, view: ViewMode, cx: &mut Context<Self>) {
        self.browser.view = view;
        self.menu = None;
        cx.notify();
    }

    pub fn set_sort(&mut self, sort: SortKey, cx: &mut Context<Self>) {
        let ascending = if self.browser.sort == sort {
            !self.browser.ascending
        } else {
            sort == SortKey::Name || sort == SortKey::Kind
        };
        self.browser.resort(sort, ascending);
        self.menu = None;
        cx.notify();
    }

    pub fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.browser.show_hidden = !self.browser.show_hidden;
        self.menu = None;
        self.load(cx);
    }

    pub fn set_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.request.filters.len() {
            return;
        }
        self.filter_index = Some(index);
        let policy = self.policy();
        self.browser.apply_policy(&policy);
        self.menu = None;
        cx.notify();
    }

    pub fn set_choice(&mut self, index: usize, value: String, cx: &mut Context<Self>) {
        if let Some(choice) = self.choices.get_mut(index) {
            if choice.is_checkbox() {
                choice.selected = if choice.checked() { "false" } else { "true" }.to_owned();
            } else if choice.options.iter().any(|(key, _)| *key == value) {
                choice.selected = value;
            }
        }
        self.menu = None;
        cx.notify();
    }

    pub fn toggle_menu(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(kind) {
            None
        } else {
            Some(kind)
        };
        cx.notify();
    }

    pub fn run_menu(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        match action {
            MenuAction::Go(path) => self.navigate(Location::Folder(path), cx),
            MenuAction::View(view) => self.set_view(view, cx),
            MenuAction::Sort(sort) => self.set_sort(sort, cx),
            MenuAction::ToggleHidden => self.toggle_hidden(cx),
            MenuAction::Filter(index) => self.set_filter(index, cx),
            MenuAction::Choice(index, value) => self.set_choice(index, value, cx),
        }
    }

    pub fn menu_items(&self, kind: MenuKind) -> Vec<MenuItem> {
        let item = |label: String, checked: bool, action: MenuAction| MenuItem {
            label: label.into(),
            icon: None,
            checked,
            action: Some(action),
        };
        let separator = || MenuItem {
            label: SharedString::default(),
            icon: None,
            checked: false,
            action: None,
        };
        match kind {
            MenuKind::Where => {
                let mut items = Vec::new();
                if let Some(folder) = self.browser.location().folder() {
                    for ancestor in folder.ancestors() {
                        items.push(MenuItem {
                            label: rmac_file_chooser::browser::display_name(ancestor).into(),
                            icon: Some(if ancestor == Path::new("/") {
                                "icons/hard-drive.svg"
                            } else {
                                "icons/folder-artwork.svg"
                            }),
                            checked: ancestor == folder,
                            action: Some(MenuAction::Go(ancestor.to_path_buf())),
                        });
                    }
                    items.push(separator());
                }
                for section in self.sections.iter().skip(1) {
                    for place in &section.places {
                        if let Location::Folder(path) = &place.location {
                            items.push(MenuItem {
                                label: place.name.clone(),
                                icon: Some(place.icon),
                                checked: false,
                                action: Some(MenuAction::Go(path.clone())),
                            });
                        }
                    }
                }
                items
            }
            MenuKind::View => vec![
                item(
                    "as Icons".into(),
                    self.browser.view == ViewMode::Icons,
                    MenuAction::View(ViewMode::Icons),
                ),
                item(
                    "as List".into(),
                    self.browser.view == ViewMode::List,
                    MenuAction::View(ViewMode::List),
                ),
            ],
            MenuKind::Sort => vec![
                item(
                    "Name".into(),
                    self.browser.sort == SortKey::Name,
                    MenuAction::Sort(SortKey::Name),
                ),
                item(
                    "Kind".into(),
                    self.browser.sort == SortKey::Kind,
                    MenuAction::Sort(SortKey::Kind),
                ),
                item(
                    "Date Modified".into(),
                    self.browser.sort == SortKey::Date,
                    MenuAction::Sort(SortKey::Date),
                ),
                item(
                    "Size".into(),
                    self.browser.sort == SortKey::Size,
                    MenuAction::Sort(SortKey::Size),
                ),
                separator(),
                item(
                    "Show Hidden Files".into(),
                    self.browser.show_hidden,
                    MenuAction::ToggleHidden,
                ),
            ],
            MenuKind::Filter => self
                .request
                .filters
                .iter()
                .enumerate()
                .map(|(index, filter)| {
                    item(
                        filter.name.clone(),
                        self.filter_index == Some(index),
                        MenuAction::Filter(index),
                    )
                })
                .collect(),
            MenuKind::Choice(index) => self
                .choices
                .get(index)
                .map(|choice| {
                    choice
                        .options
                        .iter()
                        .map(|(key, label)| {
                            item(
                                label.clone(),
                                *key == choice.selected,
                                MenuAction::Choice(index, key.clone()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    // ---- Save expansion, New Folder -------------------------------------

    pub fn toggle_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        self.menu = None;
        let (width, height) = self.window_size();
        // `Window::resize` sets the outer platform surface, the same units
        // as `window.viewport_size()` — not the smaller box `Root` hands
        // the panel's own content (`rmac_ui::window_content_size`); see the
        // matching comment in `main.rs::open_panel`.
        let (outer_width, outer_height) = rmac_ui::outer_window_size(width, height);
        window.resize(size(px(outer_width), px(outer_height)));
        cx.notify();
    }

    /// ⇧⌘N / the New Folder button: asks the name first, like the Mac's
    /// "Name of new folder inside “…”:" sheet, instead of silently creating
    /// “untitled folder” (OTHER-10).
    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.mode == Mode::Open || self.new_folder_sheet.is_some() {
            return;
        }
        let Some(folder) = self.browser.location().folder().map(Path::to_path_buf) else {
            return;
        };
        self.menu = None;
        let default_name = "untitled folder";
        let input = cx.new(|cx| InputState::new(window, cx).default_value(default_name));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.new_folder_commit(window, cx);
                }
            },
        );
        self._subscriptions.push(subscription);
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        input.update(cx, |state, cx| {
            state.set_selected_range(0..default_name.len(), cx);
        });
        self.new_folder_sheet = Some(NewFolderSheet {
            input,
            folder,
            error: false,
        });
        cx.notify();
    }

    pub fn new_folder_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.new_folder_sheet.as_ref() else {
            return;
        };
        let folder = sheet.folder.clone();
        let name = sheet.input.read(cx).value().trim().to_owned();
        if !rmac_file_chooser::request::valid_file_name(&name)
            || folder.join(&name).symlink_metadata().is_ok()
            || std::fs::create_dir(folder.join(&name)).is_err()
        {
            if let Some(sheet) = self.new_folder_sheet.as_mut() {
                sheet.error = true;
            }
            cx.notify();
            return;
        }
        self.new_folder_sheet = None;
        window.focus(&self.focus, cx);
        self.navigate(Location::Folder(folder.join(name)), cx);
    }

    // ---- Go to Folder -------------------------------------------------------

    pub fn open_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.goto.is_some() {
            return;
        }
        self.menu = None;
        // The sheet needs the full panel; the compact Save sheet expands first.
        if self.is_compact() {
            self.toggle_expanded(window, cx);
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to Folder"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    let cwd = this
                        .browser
                        .location()
                        .folder()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| this.home.clone());
                    let suggestions =
                        goto::suggestions(&text, &cwd, &this.home, this.browser.show_hidden);
                    if let Some(sheet) = this.goto.as_mut() {
                        sheet.highlighted = (!suggestions.is_empty()).then_some(0);
                        sheet.suggestions = suggestions;
                        sheet.error = false;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.goto_commit(window, cx),
                _ => {}
            },
        );
        self._subscriptions.push(subscription);
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.goto = Some(GoToSheet {
            input,
            suggestions: Vec::new(),
            highlighted: None,
            error: false,
        });
        cx.notify();
    }

    pub fn goto_pick(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.goto = None;
        window.focus(&self.focus, cx);
        self.navigate(Location::Folder(path), cx);
    }

    fn goto_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.goto.as_ref() else {
            return;
        };
        let text = sheet.input.read(cx).value().to_string();
        let cwd = self
            .browser
            .location()
            .folder()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.home.clone());
        // A highlighted suggestion wins when the typed path is incomplete.
        let resolved = goto::resolve(&text, &cwd, &self.home).or_else(|| {
            sheet
                .highlighted
                .and_then(|index| sheet.suggestions.get(index).cloned())
                .map(|folder| goto::GoTo {
                    folder,
                    select: None,
                })
        });
        match resolved {
            Some(target) => {
                self.goto = None;
                window.focus(&self.focus, cx);
                self.pending_select = target.select;
                self.navigate(Location::Folder(target.folder), cx);
            }
            None => {
                if let Some(sheet) = self.goto.as_mut() {
                    sheet.error = true;
                }
                cx.notify();
            }
        }
    }

    pub fn goto_move(&mut self, step: isize, cx: &mut Context<Self>) {
        if let Some(sheet) = self.goto.as_mut() {
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
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        // A window closed by the compositor answers Cancel.
        if let Some(reply) = self.reply.take() {
            let _ = reply.try_send(Outcome::Cancelled);
        }
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod panel_tests {
    use super::base_name_selection;

    #[test]
    fn base_name_selection_stops_before_the_last_extension() {
        assert_eq!(base_name_selection("Untitled.txt"), 0..8);
        // Only the last extension is excluded, as Finder does.
        assert_eq!(base_name_selection("Archive.tar.gz"), 0..11);
        assert_eq!(base_name_selection("Untitled"), 0..8);
        // A dotfile's leading dot is not an extension to protect.
        assert_eq!(base_name_selection(".bashrc"), 0..7);
        assert_eq!(base_name_selection(""), 0..0);
    }
}
