//! The Dock's tiles: pinned apps in their order, each marked running when
//! it has a window, then every other running app, as on the Mac. The
//! Recycle Bin sits after them, past a separator, where the Mac has the
//! Trash.

use super::apps::{LuloApp, LULO_APPS};

/// How a tile opens its app when it has no window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Launch {
    /// A Lulo app's executable, beside `lulo-shell.exe`.
    Lulo(&'static LuloApp),
    /// Anything `ShellExecute` opens: an executable path, a
    /// `shell:AppsFolder\…` app, a URI.
    Shell(String),
    /// A running app that is not pinned: activating its windows is all a
    /// click does.
    None,
}

/// A pinned tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pinned {
    /// The executable's lower-case file name its windows belong to.
    pub key: String,
    pub name: String,
    pub launch: Launch,
    pub icon: TileIcon,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TileIcon {
    /// An icon in the shell's own assets.
    Asset(&'static str),
    /// The icon Windows shows for this executable or shell item.
    Shell(String),
}

/// A running app: its windows, front-most first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Running {
    pub key: String,
    pub exe_path: String,
    pub name: String,
    pub windows: Vec<isize>,
    /// The Lulo app this is, when it is one (by executable, by the app id
    /// it gave the menu bar, or by its AppUserModelID): it then shows the
    /// Lulo app's own icon and name, never whatever icon its executable
    /// happens to carry.
    pub lulo: Option<&'static LuloApp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tile {
    pub key: String,
    pub name: String,
    pub pinned: bool,
    pub launch: Launch,
    pub icon: TileIcon,
    /// Front-most first; empty when not running.
    pub windows: Vec<isize>,
}

impl Tile {
    pub fn running(&self) -> bool {
        !self.windows.is_empty()
    }
}

/// The default pinned set: Files first (where the Finder sits), File
/// Explorer, then the other Lulo apps.
pub fn default_pins() -> Vec<Pinned> {
    let lulo = |app: &'static LuloApp| Pinned {
        key: app.exe.to_owned(),
        name: app.name.to_owned(),
        launch: Launch::Lulo(app),
        icon: TileIcon::Asset(app.icon),
    };
    let mut pins = vec![
        lulo(&LULO_APPS[0]),
        Pinned {
            key: "explorer.exe".to_owned(),
            name: "File Explorer".to_owned(),
            launch: Launch::Shell("explorer.exe".to_owned()),
            icon: TileIcon::Shell("explorer.exe".to_owned()),
        },
    ];
    pins.extend(LULO_APPS[1..].iter().map(lulo));
    pins
}

/// Whether running app `app` is pinned tile `pin`.
fn is_pin(pin: &Pinned, app: &Running) -> bool {
    pin.key == app.key
        || matches!((&pin.launch, app.lulo), (Launch::Lulo(pinned), Some(lulo)) if pinned.app_id == lulo.app_id)
}

/// The Dock's tiles for `pins` and the apps that have windows now.
pub fn tiles(pins: &[Pinned], running: &[Running]) -> Vec<Tile> {
    let mut tiles = pins
        .iter()
        .map(|pin| Tile {
            key: pin.key.clone(),
            name: pin.name.clone(),
            pinned: true,
            launch: pin.launch.clone(),
            icon: pin.icon.clone(),
            windows: running
                .iter()
                .filter(|app| is_pin(pin, app))
                .flat_map(|app| app.windows.iter().copied())
                .collect(),
        })
        .collect::<Vec<_>>();
    for app in running {
        if pins.iter().any(|pin| is_pin(pin, app)) || app.windows.is_empty() {
            continue;
        }
        let (name, launch, icon) = match app.lulo {
            Some(lulo) => (
                lulo.name.to_owned(),
                Launch::Lulo(lulo),
                TileIcon::Asset(lulo.icon),
            ),
            None => (
                app.name.clone(),
                Launch::None,
                TileIcon::Shell(app.exe_path.clone()),
            ),
        };
        tiles.push(Tile {
            key: app.key.clone(),
            name,
            pinned: false,
            launch,
            icon,
            windows: app.windows.clone(),
        });
    }
    tiles
}

/// The Recycle Bin's tile picture, full or empty, as the Mac's Trash.
pub fn bin_icon(full: bool) -> &'static str {
    if full {
        "dock/trash-full.svg"
    } else {
        "dock/trash-empty.svg"
    }
}

/// What a click on a tile does: bring its windows forward (the front-most
/// first, restoring it if minimised) or open the app.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Click {
    Activate(isize),
    Launch(Launch),
}

pub fn click(tile: &Tile) -> Click {
    match tile.windows.first() {
        Some(&window) => Click::Activate(window),
        None => Click::Launch(tile.launch.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::apps::lulo_app;

    fn running(key: &str, windows: &[isize]) -> Running {
        Running {
            key: key.to_owned(),
            exe_path: format!(r"C:\Apps\{key}"),
            name: key.trim_end_matches(".exe").to_owned(),
            windows: windows.to_vec(),
            lulo: crate::model::apps::lulo_app_for_exe(key),
        }
    }

    #[test]
    fn pinned_tiles_keep_their_order_and_running_apps_follow() {
        let pins = default_pins();
        assert_eq!(pins[0].name, "Files");
        assert_eq!(pins[1].name, "File Explorer");
        assert_eq!(pins[3].key, "rmac-calculator.exe");
        let tiles = tiles(
            &pins,
            &[
                running("notepad.exe", &[7]),
                running("rmac-calculator.exe", &[3, 4]),
                running("idle.exe", &[]),
            ],
        );
        assert_eq!(tiles.len(), pins.len() + 1);
        assert!(tiles[3].running());
        assert_eq!(tiles[3].windows, [3, 4]);
        assert!(!tiles[0].running());
        let last = tiles.last().unwrap();
        assert_eq!(last.key, "notepad.exe");
        assert!(!last.pinned);
        assert_eq!(last.launch, Launch::None);
    }

    #[test]
    fn a_running_lulo_app_shows_its_own_icon_never_its_executables() {
        // Files run from a renamed executable, known by the app id it gave
        // the bar: it is the pinned Files tile, running.
        let mut files = running("files-dev.exe", &[5]);
        files.lulo = lulo_app("org.rmac.Files");
        let pins = default_pins();
        let tiles = tiles(&pins, &[files.clone()]);
        assert_eq!(tiles.len(), pins.len());
        assert_eq!(tiles[0].windows, [5]);
        // Unpinned (the user's own pins without Files), it still shows the
        // Files artwork and name rather than the executable's icon.
        let without_files = pins[1..].to_vec();
        let tiles = super::tiles(&without_files, &[files]);
        let last = tiles.last().unwrap();
        assert_eq!(last.name, "Files");
        assert_eq!(last.icon, TileIcon::Asset("apps/org.rmac.Files.svg"));
        assert!(matches!(last.launch, Launch::Lulo(app) if app.exe == "rmac-files.exe"));
    }

    #[test]
    fn a_click_activates_the_front_window_or_launches() {
        let pins = default_pins();
        let tiles = tiles(&pins, &[running("rmac-calculator.exe", &[9, 2])]);
        assert_eq!(click(&tiles[3]), Click::Activate(9));
        assert!(
            matches!(click(&tiles[2]), Click::Launch(Launch::Lulo(app)) if app.name == "Notes")
        );
        assert_eq!(
            click(&tiles[1]),
            Click::Launch(Launch::Shell("explorer.exe".to_owned()))
        );
    }

    #[test]
    fn the_bin_shows_full_or_empty() {
        assert_eq!(bin_icon(true), "dock/trash-full.svg");
        assert_eq!(bin_icon(false), "dock/trash-empty.svg");
    }
}
