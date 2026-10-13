//! Apps: a searchable grid of installed applications. Linux runs it as its
//! own process (`rmac-app-drawer`); the Windows shell hosts the same view
//! in `lulo-shell` ([`start`], [`toggle`]; ADR 0023, "Phase 3 revised:
//! shared shell views").

pub mod accessibility;
mod catalog;
mod drag_endpoint;
mod search;
mod service;
mod view;

pub use service::{asset, run_service, run_standalone};
#[cfg(windows)]
pub use service::{open_scene, start, toggle};

gpui::actions!(
    app_drawer,
    [
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        Launch,
        ClearSearch,
        OpenApp,
        RevealInFinder
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    Standalone,
    Service { show_on_start: bool },
}

pub fn run_mode(arguments: impl IntoIterator<Item = String>) -> RunMode {
    let mut service = false;
    let mut show_on_start = false;
    for argument in arguments {
        service |= argument == "--service";
        show_on_start |= argument == "--show";
    }
    if service {
        RunMode::Service { show_on_start }
    } else {
        RunMode::Standalone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervised_mode_is_explicit_and_can_open_for_development() {
        assert_eq!(run_mode(Vec::new()), RunMode::Standalone);
        assert_eq!(run_mode(vec!["--show".to_owned()]), RunMode::Standalone);
        assert_eq!(
            run_mode(vec!["--service".to_owned()]),
            RunMode::Service {
                show_on_start: false
            }
        );
        assert_eq!(
            run_mode(vec!["--service".to_owned(), "--show".to_owned()]),
            RunMode::Service {
                show_on_start: true
            }
        );
    }
}
