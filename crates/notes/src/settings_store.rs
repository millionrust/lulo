//! Notes ▸ Settings… choices that live outside the notes library: the
//! default text size, Group notes by date, Automatically sort ticked items,
//! Use dark backgrounds for note content and Always resume to last Quick
//! Note. Kept in `~/.config/lulo/notes-settings.conf` (one `key=value` per
//! line) so they survive a relaunch; read and written off the UI thread.

use std::path::PathBuf;

use crate::NotesView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotesPrefs {
    pub(crate) zoom: i8,
    pub(crate) group_by_date: bool,
    pub(crate) auto_sort_ticked: bool,
    pub(crate) light_background_default: bool,
    pub(crate) always_resume_quick_note: bool,
}

impl Default for NotesPrefs {
    /// The values a fresh `NotesView` starts with.
    fn default() -> Self {
        Self {
            zoom: 0,
            group_by_date: true,
            auto_sort_ticked: false,
            light_background_default: false,
            always_resume_quick_note: false,
        }
    }
}

impl NotesPrefs {
    pub(crate) fn of(view: &NotesView) -> Self {
        Self {
            zoom: view.note_zoom,
            group_by_date: view.group_notes_by_date,
            auto_sort_ticked: view.auto_sort_ticked_items,
            light_background_default: view.light_background_default,
            always_resume_quick_note: view.always_resume_quick_note,
        }
    }

    pub(crate) fn apply(self, view: &mut NotesView) {
        view.note_zoom = self.zoom;
        view.group_notes_by_date = self.group_by_date;
        view.auto_sort_ticked_items = self.auto_sort_ticked;
        view.light_background_default = self.light_background_default;
        view.always_resume_quick_note = self.always_resume_quick_note;
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("lulo/notes-settings.conf"))
}

fn parse(contents: &str) -> NotesPrefs {
    let mut prefs = NotesPrefs::default();
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let on = value == "true";
        match key {
            "zoom" => {
                if let Ok(zoom) = value.parse::<i8>() {
                    prefs.zoom = zoom.clamp(-5, 12);
                }
            }
            "group_by_date" => prefs.group_by_date = on,
            "auto_sort_ticked" => prefs.auto_sort_ticked = on,
            "light_background_default" => prefs.light_background_default = on,
            "always_resume_quick_note" => prefs.always_resume_quick_note = on,
            _ => {}
        }
    }
    prefs
}

fn serialize(prefs: NotesPrefs) -> String {
    format!(
        "version=1\nzoom={}\ngroup_by_date={}\nauto_sort_ticked={}\nlight_background_default={}\nalways_resume_quick_note={}\n",
        prefs.zoom,
        prefs.group_by_date,
        prefs.auto_sort_ticked,
        prefs.light_background_default,
        prefs.always_resume_quick_note,
    )
}

/// Blocking: call on a background thread.
pub(crate) fn load() -> NotesPrefs {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|contents| parse(&contents))
        .unwrap_or_default()
}

/// Blocking: call on a background thread.
pub(crate) fn save(prefs: NotesPrefs) {
    let Some(path) = path() else {
        return;
    };
    let result = path
        .parent()
        .map_or(Ok(()), rmac_storage::create_dir_all_private)
        .and_then(|()| rmac_storage::atomic_write_private(&path, serialize(prefs).as_bytes()));
    if let Err(error) = result {
        eprintln!("rmac-notes: could not save Settings: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_clamp() {
        let prefs = NotesPrefs {
            zoom: 4,
            group_by_date: false,
            auto_sort_ticked: true,
            light_background_default: true,
            always_resume_quick_note: true,
        };
        assert_eq!(parse(&serialize(prefs)), prefs);
        assert_eq!(parse("zoom=99\n").zoom, 12);
        assert_eq!(parse(""), NotesPrefs::default());
    }
}
