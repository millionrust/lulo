//! rmac Finder — a functional macOS-style file manager. See SPEC.md.

mod conflict;
mod directory_state;
mod file_ops;
mod operation_journal;
mod pasteboard;
mod quick_look;
mod recovery_ui;
#[cfg(any(target_os = "linux", test))]
mod trash_store;
mod undo_journal;
mod view;
mod watchers;

fn main() {
    let destination = match StartupDestination::parse(std::env::args().skip(1)) {
        Ok(destination) => destination,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    view::run(destination);
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StartupDestination {
    Default,
    Trash,
    Directory(std::path::PathBuf),
}

impl StartupDestination {
    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Self, &'static str> {
        match (arguments.next(), arguments.next(), arguments.next()) {
            (None, None, None) => Ok(Self::Default),
            (Some(flag), None, None) if flag == "--trash" => Ok(Self::Trash),
            (Some(flag), Some(path), None) if flag == "--path" => {
                let path = std::path::PathBuf::from(path);
                if path.is_absolute() && path.is_dir() {
                    Ok(Self::Directory(path))
                } else {
                    Err("rmac-files --path requires an existing absolute directory")
                }
            }
            _ => Err("usage: rmac-files [--trash | --path ABSOLUTE_DIRECTORY]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StartupDestination;

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
        assert!(StartupDestination::parse(["trash:///".to_owned()].into_iter()).is_err());
        assert!(
            StartupDestination::parse(["--trash".to_owned(), "extra".to_owned()].into_iter())
                .is_err()
        );
    }
}
