use super::*;
use crate::open::{containing_directory, failure, uri_failure};
use std::path::Path;

#[test]
fn fallback_targets_the_containing_directory_for_files() {
    assert_eq!(
        containing_directory(Path::new("/usr/share/applications/demo.desktop")),
        Path::new("/usr/share/applications")
    );
}

#[test]
fn errors_preserve_the_requested_path() {
    let error = failure(
        Operation::Show,
        Path::new("demo.desktop"),
        "portal unavailable",
    );
    assert_eq!(error.operation, Operation::Show);
    assert_eq!(error.path, Path::new("demo.desktop"));
    assert!(error.to_string().contains("portal unavailable"));
}

#[test]
fn open_and_show_errors_name_the_operation() {
    let path = Path::new("report.txt");
    assert!(failure(Operation::Open, path, "offline")
        .to_string()
        .starts_with("Could not open"));
    assert!(failure(Operation::Show, path, "offline")
        .to_string()
        .starts_with("Could not show"));
}

#[test]
fn uri_errors_do_not_retain_or_display_the_requested_uri() {
    let private_uri = "https://example.test/private?token=secret";
    let error = uri_failure(private_uri);

    assert!(!format!("{error:?}").contains(private_uri));
    assert!(!error.to_string().contains(private_uri));
    assert_eq!(
        error.to_string(),
        "Could not open link: the desktop handler did not accept the request"
    );
}
