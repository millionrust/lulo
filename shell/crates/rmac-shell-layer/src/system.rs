//! What the shell's views ask of the system beyond windows: Lulo's own
//! programs, the session's power commands, opening a document, a notice,
//! and shell surfaces another view owns (ADR 0023, "Phase 3 revised: shared
//! shell views").
//!
//! On Lulo OS the views keep their own commands (`systemctl`, the shortcut
//! dispatcher, the notification server); these functions are what they use
//! on Windows, and a few small helpers both share.

use std::path::{Path, PathBuf};

/// A session power command, as the Lulo menu offers them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerCommand {
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
}

/// A Lulo program by its Lulo OS path (`/usr/bin/rmac-files`,
/// `/usr/libexec/rmac/…`): that path on Lulo OS; on Windows the executable
/// of the same name beside the running one (`rmac-files.exe`), where the
/// installer puts every Lulo program.
pub fn program(linux_path: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let name = Path::new(linux_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| linux_path.to_owned());
        let directory = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        directory.join(format!("{name}.exe"))
    }
    #[cfg(not(windows))]
    {
        let _ = Path::new(linux_path);
        PathBuf::from(linux_path)
    }
}

/// Carry out a power command. Lulo OS's views run their own session
/// commands; this is Windows'.
#[cfg(windows)]
pub fn power(command: PowerCommand) {
    use crate::windows::power::{self, Command};
    power::run(match command {
        PowerCommand::Sleep => Command::Sleep,
        PowerCommand::Restart => Command::Restart,
        PowerCommand::ShutDown => Command::ShutDown,
        PowerCommand::LockScreen => Command::LockScreen,
        PowerCommand::LogOut => Command::LogOut,
    });
}

/// Open `target` (a file, a folder or a URL) with its default handler.
pub fn open(target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        crate::windows::shell_open(target)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(target)
            .spawn()
            .map(|_| ())
    }
}

/// The signed-in user's display name, for "Log Out <name>…".
#[cfg(windows)]
pub fn account_display_name() -> Option<String> {
    crate::windows::account_display_name()
}

/// A shell surface or request another view owns (Spotlight, Control
/// Centre, Notification Centre), by the name the Lulo OS shortcut
/// dispatcher gives it (`launcher`, `quick-settings`, …). One process runs
/// every surface on Windows, so a view hands the request to whoever
/// registered for it there.
pub mod requests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use gpui::App;

    type Handler = Rc<dyn Fn(&mut App)>;

    thread_local! {
        static HANDLERS: RefCell<BTreeMap<&'static str, Handler>> =
            const { RefCell::new(BTreeMap::new()) };
    }

    /// Run `handler` whenever a view asks for `name`.
    pub fn register(name: &'static str, handler: impl Fn(&mut App) + 'static) {
        HANDLERS.with(|handlers| handlers.borrow_mut().insert(name, Rc::new(handler)));
    }

    /// Ask for `name`; false when nothing in this process handles it.
    pub fn run(name: &str, cx: &mut App) -> bool {
        let handler = HANDLERS.with(|handlers| handlers.borrow().get(name).cloned());
        match handler {
            Some(handler) => {
                handler(cx);
                true
            }
            None => false,
        }
    }

    /// Whether anything handles `name`.
    pub fn handles(name: &str) -> bool {
        HANDLERS.with(|handlers| handlers.borrow().contains_key(name))
    }
}

/// Lulo menu rows only one platform has (Windows: Start Lulo at Sign-In,
/// Use Files for Folders, Turn Off Lulo). Each row's action is a
/// [`requests`] name; `checked` asks whether its checkmark shows.
pub mod extra_menu {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// One row: label, action, whether it starts a new group, and whether
    /// it is ticked now.
    #[derive(Clone)]
    pub struct Row {
        pub label: &'static str,
        pub action: &'static str,
        pub separated: bool,
        pub checked: Option<Rc<dyn Fn() -> bool>>,
    }

    thread_local! {
        static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    }

    /// Add `row` to the end of the Lulo menu.
    pub fn add(row: Row) {
        ROWS.with(|rows| rows.borrow_mut().push(row));
    }

    pub fn rows() -> Vec<Row> {
        ROWS.with(|rows| rows.borrow().clone())
    }
}
