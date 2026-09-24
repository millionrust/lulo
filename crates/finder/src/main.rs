//! rmac Finder — a functional macOS-style file manager. See SPEC.md.

mod conflict;
mod directory_state;
mod file_ops;
mod operation_journal;
mod pasteboard;
mod recovery_ui;
#[cfg(any(target_os = "linux", test))]
mod trash_store;
mod undo_journal;
mod view;
mod watchers;

use std::path::{Path, PathBuf};

fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let current_dir = std::env::current_dir().ok();
    let windows = match StartupDestination::parse_launch(arguments, current_dir.as_deref()) {
        Ok((windows, skipped)) => {
            for message in skipped {
                eprintln!("{message}");
            }
            windows
        }
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    // One Files process owns every Files window; a later launch passes each
    // window's arguments to the running process instead of starting another.
    view::run(
        windows
            .iter()
            .map(StartupDestination::window_arguments)
            .collect(),
    );
}

const USAGE: &str =
    "usage: rmac-files [--trash | --path DIRECTORY | --reveal PATH | --search QUERY | PATH-OR-FILE-URI…]";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum StartupDestination {
    #[default]
    Default,
    Trash,
    Directory(PathBuf),
    /// A file named on the command line: its folder opens with it selected,
    /// as `open -R` does.
    Reveal(PathBuf),
    /// Spotlight's "Search in Files": search the home folder for this.
    Search(String),
}

impl StartupDestination {
    /// One window's arguments, as a launch forwards them to the running
    /// process (absolute paths only, since that process has its own cwd).
    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Self, &'static str> {
        match (arguments.next(), arguments.next(), arguments.next()) {
            (None, None, None) => Ok(Self::Default),
            (Some(flag), None, None) if flag == "--trash" => Ok(Self::Trash),
            (Some(flag), Some(path), None) if flag == "--path" => {
                let path = PathBuf::from(path);
                if path.is_absolute() && path.is_dir() {
                    Ok(Self::Directory(path))
                } else {
                    Err("rmac-files --path requires an existing absolute directory")
                }
            }
            (Some(flag), Some(path), None) if flag == "--reveal" => {
                let path = PathBuf::from(path);
                if path.is_absolute() && path.exists() && path.parent().is_some() {
                    Ok(Self::Reveal(path))
                } else {
                    Err("rmac-files --reveal requires an existing absolute path")
                }
            }
            (Some(flag), Some(query), None) if flag == "--search" => {
                let query = query.trim();
                if query.is_empty() || query.len() > 512 || query.chars().any(char::is_control) {
                    Err("rmac-files --search requires a short, printable query")
                } else {
                    Ok(Self::Search(query.to_owned()))
                }
            }
            (Some(item), None, None) if !item.starts_with("--") => {
                Self::for_item(&item, None).map_err(|_| USAGE)
            }
            _ => Err(USAGE),
        }
    }

    /// A launch's command line: the flag forms open one window; otherwise
    /// every argument is a path or `file://` URI (as a file manager is
    /// handed by `xdg-open` and `%U`) and each gets its own window. Items
    /// that do not exist are reported and skipped; the launch fails only
    /// when none remain.
    fn parse_launch(
        arguments: Vec<String>,
        current_dir: Option<&Path>,
    ) -> Result<(Vec<Self>, Vec<String>), String> {
        if arguments
            .first()
            .is_none_or(|first| first.starts_with("--"))
        {
            return Self::parse(arguments.into_iter())
                .map(|destination| (vec![destination], Vec::new()))
                .map_err(str::to_owned);
        }
        let mut windows = Vec::new();
        let mut skipped = Vec::new();
        for item in &arguments {
            match Self::for_item(item, current_dir) {
                Ok(destination) if !windows.contains(&destination) => windows.push(destination),
                Ok(_) => {}
                Err(message) => skipped.push(message),
            }
        }
        if windows.is_empty() {
            return Err(skipped.join("\n"));
        }
        windows.truncate(MAX_LAUNCH_WINDOWS);
        Ok((windows, skipped))
    }

    /// A folder opens as itself; any other existing item is revealed.
    fn for_item(item: &str, current_dir: Option<&Path>) -> Result<Self, String> {
        let path = item_path(item, current_dir)
            .ok_or_else(|| format!("rmac-files cannot open “{item}”: not a local path"))?;
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => Ok(Self::Directory(path)),
            Ok(_) if path.parent().is_some() => Ok(Self::Reveal(path)),
            Ok(_) => Err(format!("rmac-files cannot open “{item}”")),
            Err(error) => Err(format!("rmac-files cannot open “{item}”: {error}")),
        }
    }

    /// The arguments that open this destination's window in any process.
    fn window_arguments(&self) -> Vec<String> {
        let path_argument =
            |flag: &str, path: &Path| vec![flag.to_owned(), path.to_string_lossy().into_owned()];
        match self {
            Self::Default => Vec::new(),
            Self::Trash => vec!["--trash".to_owned()],
            Self::Directory(path) => path_argument("--path", path),
            Self::Reveal(path) => path_argument("--reveal", path),
            Self::Search(query) => vec!["--search".to_owned(), query.clone()],
        }
    }
}

/// More windows than this from one launch is almost certainly a mistake.
const MAX_LAUNCH_WINDOWS: usize = 8;

/// A plain path (relative ones against `current_dir`) or a local `file://`
/// URI, with `.` and `..` folded away.
fn item_path(item: &str, current_dir: Option<&Path>) -> Option<PathBuf> {
    if item.is_empty() || item.contains('\0') {
        return None;
    }
    let raw = if let Some(rest) = item.strip_prefix("file://") {
        // file:///path or file://localhost/path; other hosts are remote.
        let path = if rest.starts_with('/') {
            rest
        } else {
            rest.strip_prefix("localhost")
                .filter(|path| path.starts_with('/'))?
        };
        let path = path.split(['?', '#']).next().unwrap_or_default();
        use std::os::unix::ffi::OsStringExt as _;
        PathBuf::from(std::ffi::OsString::from_vec(percent_decode(path)?))
    } else if item.contains("://") {
        return None;
    } else {
        let path = PathBuf::from(item);
        if path.is_absolute() {
            path
        } else {
            current_dir?.join(path)
        }
    };
    let mut normal = PathBuf::from("/");
    for component in raw.components() {
        match component {
            std::path::Component::Normal(part) => normal.push(part),
            std::path::Component::ParentDir => {
                normal.pop();
            }
            _ => {}
        }
    }
    Some(normal)
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == 0 {
                return None;
            }
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::{item_path, StartupDestination};
    use std::path::{Path, PathBuf};

    #[test]
    fn startup_destination_accepts_only_the_explicit_trash_option() {
        assert_eq!(
            StartupDestination::parse(std::iter::empty()),
            Ok(StartupDestination::Default)
        );
        assert_eq!(
            StartupDestination::parse(["--trash".to_owned()].into_iter()),
            Ok(StartupDestination::Trash)
        );
        assert_eq!(
            StartupDestination::parse(
                [
                    "--path".to_owned(),
                    std::env::temp_dir().display().to_string()
                ]
                .into_iter()
            ),
            Ok(StartupDestination::Directory(std::env::temp_dir()))
        );
        assert_eq!(
            StartupDestination::parse(["--search".to_owned(), " report ".to_owned()].into_iter()),
            Ok(StartupDestination::Search("report".into()))
        );
        assert!(
            StartupDestination::parse(["--search".to_owned(), "  ".to_owned()].into_iter())
                .is_err()
        );
        assert!(
            StartupDestination::parse(["--search".to_owned(), "a\nb".to_owned()].into_iter())
                .is_err()
        );
        assert!(StartupDestination::parse(["trash:///".to_owned()].into_iter()).is_err());
        assert!(
            StartupDestination::parse(["--trash".to_owned(), "extra".to_owned()].into_iter())
                .is_err()
        );
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rmac-files-args-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(path.join("My Docs")).unwrap();
            std::fs::write(path.join("My Docs/report.txt"), b"x").unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn plain_paths_and_file_uris_name_local_items() {
        let cwd = Path::new("/home/u");
        assert_eq!(
            item_path("/tmp/a/../b", None),
            Some(PathBuf::from("/tmp/b"))
        );
        assert_eq!(
            item_path("Docs", Some(cwd)),
            Some(PathBuf::from("/home/u/Docs"))
        );
        assert_eq!(item_path("Docs", None), None);
        assert_eq!(
            item_path("file:///home/u/My%20Docs", None),
            Some(PathBuf::from("/home/u/My Docs"))
        );
        assert_eq!(
            item_path("file://localhost/srv/x", None),
            Some(PathBuf::from("/srv/x"))
        );
        assert_eq!(item_path("file://server/share", None), None);
        assert_eq!(item_path("sftp://host/x", None), None);
        assert_eq!(item_path("file:///bad%zz", None), None);
        assert_eq!(item_path("file:///nul%00", None), None);
    }

    #[test]
    fn a_launch_opens_one_window_per_folder_and_reveals_files() {
        let scratch = Scratch::new("launch");
        let folder = scratch.0.join("My Docs");
        let file = folder.join("report.txt");
        let uri = format!(
            "file://{}",
            folder.display().to_string().replace(' ', "%20")
        );
        let (windows, skipped) = StartupDestination::parse_launch(
            vec![
                uri,
                file.display().to_string(),
                scratch.0.join("missing").display().to_string(),
                folder.display().to_string(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            windows,
            [
                StartupDestination::Directory(folder.clone()),
                StartupDestination::Reveal(file.clone()),
            ]
        );
        assert_eq!(skipped.len(), 1);
        assert_eq!(
            windows[1].window_arguments(),
            ["--reveal".to_owned(), file.display().to_string()]
        );
        assert_eq!(
            StartupDestination::parse(windows[1].window_arguments().into_iter()),
            Ok(StartupDestination::Reveal(file))
        );
        assert!(StartupDestination::parse_launch(
            vec![scratch.0.join("missing").display().to_string()],
            None
        )
        .is_err());
        assert_eq!(
            StartupDestination::parse_launch(vec!["--trash".to_owned()], None),
            Ok((vec![StartupDestination::Trash], Vec::new()))
        );
        assert_eq!(
            StartupDestination::parse_launch(Vec::new(), None),
            Ok((vec![StartupDestination::Default], Vec::new()))
        );
    }
}
