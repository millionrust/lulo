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
    view::run();
}
