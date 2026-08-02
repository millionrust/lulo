//! Framework-neutral App Drawer contracts.

pub mod accessibility;

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
