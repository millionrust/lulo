//! The panel's browsing state: location history, listed rows, selection,
//! and macOS keyboard behaviour (arrow keys, type-to-select). No I/O happens
//! here — the view lists folders off the render thread and hands the items
//! in — so every rule is unit-tested.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rmac_finder::listing::{sort_items, Item, SortKey};

use crate::filter::CompiledFilter;
use crate::request::Mode;

/// macOS resets the type-to-select buffer after about a second.
pub const TYPE_SELECT_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Icons,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Recents,
    Search(String),
}

impl Location {
    pub fn folder(&self) -> Option<&Path> {
        match self {
            Self::Folder(path) => Some(path),
            _ => None,
        }
    }

    pub fn title(&self) -> String {
        match self {
            Self::Folder(path) => display_name(path),
            Self::Recents => "Recents".to_owned(),
            Self::Search(query) => format!("Searching “{query}”"),
        }
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| rmac_finder::places::root_volume_name().to_owned())
}

/// Which rows may be chosen (the rest are drawn dimmed, as on the Mac).
#[derive(Clone, Debug)]
pub struct Policy {
    pub mode: Mode,
    pub directory: bool,
    pub multiple: bool,
    pub filter: CompiledFilter,
}

impl Policy {
    pub fn enabled(&self, item: &Item) -> bool {
        match self.mode {
            Mode::Open if self.directory => item.is_dir,
            Mode::Open => item.is_dir || self.filter.accepts(&item.name),
            Mode::Save | Mode::SaveFiles => item.is_dir,
        }
    }

    /// Items the default button may return (folders are browsed into unless
    /// the request asked for folders).
    pub fn choosable(&self, item: &Item) -> bool {
        match self.mode {
            Mode::Open if self.directory => item.is_dir,
            Mode::Open => !item.is_dir && self.filter.accepts(&item.name),
            Mode::Save | Mode::SaveFiles => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct Browser {
    location: Location,
    back: Vec<Location>,
    forward: Vec<Location>,
    rows: Vec<Row>,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    pub sort: SortKey,
    pub ascending: bool,
    pub view: ViewMode,
    pub show_hidden: bool,
    type_buffer: String,
    type_at: Option<Instant>,
    /// Bumped on every navigation so stale background listings are ignored.
    generation: u64,
}

impl Browser {
    pub fn new(start: PathBuf) -> Self {
        Self {
            location: Location::Folder(start),
            back: Vec::new(),
            forward: Vec::new(),
            rows: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            sort: SortKey::Name,
            ascending: true,
            view: ViewMode::Icons,
            show_hidden: false,
            type_buffer: String::new(),
            type_at: None,
            generation: 0,
        }
    }

    pub fn location(&self) -> &Location {
        &self.location
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    fn enter(&mut self, location: Location) {
        self.location = location;
        self.rows.clear();
        self.selected.clear();
        self.anchor = None;
        self.type_buffer.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Go somewhere new; returns false when already there.
    pub fn navigate(&mut self, location: Location) -> bool {
        if location == self.location {
            return false;
        }
        let previous = self.location.clone();
        // A refined search replaces the previous search in history.
        if !matches!(previous, Location::Search(_)) || !matches!(location, Location::Search(_)) {
            self.back.push(previous);
        }
        self.forward.clear();
        self.enter(location);
        true
    }

    pub fn go_back(&mut self) -> bool {
        let Some(previous) = self.back.pop() else {
            return false;
        };
        let current = self.location.clone();
        self.forward.push(current);
        self.enter(previous);
        true
    }

    pub fn go_forward(&mut self) -> bool {
        let Some(next) = self.forward.pop() else {
            return false;
        };
        let current = self.location.clone();
        self.back.push(current);
        self.enter(next);
        true
    }

    /// ⌘↑: the enclosing folder.
    pub fn go_enclosing(&mut self) -> bool {
        let parent = self
            .location
            .folder()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        match parent {
            Some(parent) => self.navigate(Location::Folder(parent)),
            None => false,
        }
    }

    /// Install a listing for `generation`; stale listings are dropped.
    /// `select` preselects an item by path (Go to Folder on a file, or the
    /// Save panel's current file).
    pub fn set_items(
        &mut self,
        generation: u64,
        mut items: Vec<Item>,
        policy: &Policy,
        select: Option<&Path>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        if !self.show_hidden {
            items.retain(|item| !item.is_hidden());
        }
        if !matches!(self.location, Location::Recents) {
            sort_items(&mut items, self.sort, self.ascending);
        }
        self.rows = items
            .into_iter()
            .map(|item| Row {
                enabled: policy.enabled(&item),
                item,
            })
            .collect();
        self.selected.clear();
        self.anchor = None;
        if let Some(select) = select {
            if let Some(index) = self.rows.iter().position(|row| row.item.path == select) {
                if self.rows[index].enabled {
                    self.select_only(index);
                }
            }
        }
        true
    }

    /// Re-evaluate enabled rows after the filter pop-up changes.
    pub fn apply_policy(&mut self, policy: &Policy) {
        for row in &mut self.rows {
            row.enabled = policy.enabled(&row.item);
        }
        let rows = &self.rows;
        self.selected.retain(|index| rows[*index].enabled);
        if self
            .anchor
            .is_some_and(|anchor| !self.selected.contains(&anchor))
        {
            self.anchor = self.selected.iter().next().copied();
        }
    }

    pub fn resort(&mut self, sort: SortKey, ascending: bool) {
        self.sort = sort;
        self.ascending = ascending;
        let selected: BTreeSet<PathBuf> = self
            .selected
            .iter()
            .map(|index| self.rows[*index].item.path.clone())
            .collect();
        let anchor = self.anchor.map(|index| self.rows[index].item.path.clone());
        let mut items: Vec<Item> = self.rows.iter().map(|row| row.item.clone()).collect();
        sort_items(&mut items, sort, ascending);
        let mut enabled = std::collections::HashMap::new();
        for row in &self.rows {
            enabled.insert(row.item.path.clone(), row.enabled);
        }
        self.rows = items
            .into_iter()
            .map(|item| Row {
                enabled: enabled.get(&item.path).copied().unwrap_or(false),
                item,
            })
            .collect();
        self.selected = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| selected.contains(&row.item.path))
            .map(|(index, _)| index)
            .collect();
        self.anchor =
            anchor.and_then(|path| self.rows.iter().position(|row| row.item.path == path));
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.contains(&index)
    }

    pub fn selected_rows(&self) -> impl Iterator<Item = &Row> {
        self.selected.iter().map(|index| &self.rows[*index])
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.selected_rows()
            .map(|row| row.item.path.clone())
            .collect()
    }

    /// The single selected folder, if exactly one folder is selected.
    pub fn selected_folder(&self) -> Option<PathBuf> {
        let mut rows = self.selected_rows();
        let first = rows.next()?;
        (rows.next().is_none() && first.item.is_dir).then(|| first.item.path.clone())
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn select_only(&mut self, index: usize) {
        if index < self.rows.len() && self.rows[index].enabled {
            self.selected.clear();
            self.selected.insert(index);
            self.anchor = Some(index);
        }
    }

    /// A click: plain selects one, ⌘ toggles, ⇧ extends from the anchor.
    /// Modifiers only act when the request allows multiple selection.
    pub fn click(&mut self, index: usize, toggle: bool, extend: bool, multiple: bool) {
        if index >= self.rows.len() {
            self.clear_selection();
            return;
        }
        if !self.rows[index].enabled {
            return;
        }
        if multiple && toggle {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
        } else if multiple && extend {
            let anchor = self.anchor.unwrap_or(index);
            let (low, high) = (anchor.min(index), anchor.max(index));
            self.selected = (low..=high).filter(|i| self.rows[*i].enabled).collect();
        } else {
            self.select_only(index);
        }
    }

    pub fn select_all(&mut self, multiple: bool) {
        if multiple {
            self.selected = (0..self.rows.len())
                .filter(|index| self.rows[*index].enabled)
                .collect();
        }
    }

    /// Arrow keys: `step` rows (±1 horizontally or in a list, ±columns
    /// vertically in the icon grid), skipping dimmed rows.
    pub fn move_selection(&mut self, step: isize) {
        if self.rows.is_empty() {
            return;
        }
        let Some(current) = self.anchor else {
            let first = if step >= 0 {
                self.rows.iter().position(|row| row.enabled)
            } else {
                self.rows.iter().rposition(|row| row.enabled)
            };
            if let Some(first) = first {
                self.select_only(first);
            }
            return;
        };
        let mut index = current as isize;
        loop {
            index += step;
            if index < 0 || index >= self.rows.len() as isize {
                return;
            }
            if self.rows[index as usize].enabled {
                self.select_only(index as usize);
                return;
            }
            if step.unsigned_abs() > 1 {
                return;
            }
        }
    }

    /// Type-to-select: characters typed within a second extend the prefix.
    pub fn type_select(&mut self, text: &str, now: Instant) -> Option<usize> {
        let fresh = self
            .type_at
            .is_none_or(|at| now.saturating_duration_since(at) > TYPE_SELECT_TIMEOUT);
        if fresh {
            self.type_buffer.clear();
        }
        self.type_buffer.push_str(&text.to_lowercase());
        self.type_at = Some(now);
        let index = self.rows.iter().position(|row| {
            row.enabled && row.item.name.to_lowercase().starts_with(&self.type_buffer)
        })?;
        self.select_only(index);
        Some(index)
    }

    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }
}
