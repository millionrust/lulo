//! The Dock's tiles: pinned apps in their order, each marked running when
//! it has a window, then every other running app, as on the Mac.

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

/// The default pinned set: File Explorer first (where the Finder sits),
/// then the Lulo apps.
pub fn default_pins() -> Vec<Pinned> {
    let mut pins = vec![Pinned {
        key: "explorer.exe".to_owned(),
        name: "File Explorer".to_owned(),
        launch: Launch::Shell("explorer.exe".to_owned()),
        icon: TileIcon::Shell("explorer.exe".to_owned()),
    }];
    pins.extend(LULO_APPS.iter().map(|app| Pinned {
        key: app.exe.to_owned(),
        name: app.name.to_owned(),
        launch: Launch::Lulo(app),
        icon: TileIcon::Asset(app.icon),
    }));
    pins
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
                .find(|app| app.key == pin.key)
                .map(|app| app.windows.clone())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    for app in running {
        if pins.iter().any(|pin| pin.key == app.key) || app.windows.is_empty() {
            continue;
        }
        tiles.push(Tile {
            key: app.key.clone(),
            name: app.name.clone(),
            pinned: false,
            launch: Launch::None,
            icon: TileIcon::Shell(app.exe_path.clone()),
            windows: app.windows.clone(),
        });
    }
    tiles
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

    fn running(key: &str, windows: &[isize]) -> Running {
        Running {
            key: key.to_owned(),
            exe_path: format!(r"C:\Apps\{key}"),
            name: key.trim_end_matches(".exe").to_owned(),
            windows: windows.to_vec(),
        }
    }

    #[test]
    fn pinned_tiles_keep_their_order_and_running_apps_follow() {
        let pins = default_pins();
        assert_eq!(pins[0].name, "File Explorer");
        assert_eq!(pins[2].key, "rmac-calculator.exe");
        let tiles = tiles(
            &pins,
            &[
                running("notepad.exe", &[7]),
                running("rmac-calculator.exe", &[3, 4]),
                running("idle.exe", &[]),
            ],
        );
        assert_eq!(tiles.len(), pins.len() + 1);
        assert!(tiles[2].running());
        assert_eq!(tiles[2].windows, [3, 4]);
        assert!(!tiles[0].running());
        let last = tiles.last().unwrap();
        assert_eq!(last.key, "notepad.exe");
        assert!(!last.pinned);
        assert_eq!(last.launch, Launch::None);
    }

    #[test]
    fn a_click_activates_the_front_window_or_launches() {
        let pins = default_pins();
        let tiles = tiles(&pins, &[running("rmac-calculator.exe", &[9, 2])]);
        assert_eq!(click(&tiles[2]), Click::Activate(9));
        assert!(
            matches!(click(&tiles[1]), Click::Launch(Launch::Lulo(app)) if app.name == "Notes")
        );
        assert_eq!(
            click(&tiles[0]),
            Click::Launch(Launch::Shell("explorer.exe".to_owned()))
        );
    }
}
