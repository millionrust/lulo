use std::path::Path;
use std::sync::Mutex;

use super::*;

#[derive(Default)]
struct FakeBackend {
    calls: Mutex<Vec<String>>,
    failure: Mutex<Option<BackendError>>,
}

impl FakeBackend {
    fn complete(&self, call: String) -> Result<(), BackendError> {
        self.calls.lock().expect("calls lock").push(call);
        self.failure
            .lock()
            .expect("failure lock")
            .take()
            .map_or(Ok(()), Err)
    }
}

impl Backend for FakeBackend {
    fn launch<'a>(
        &'a self,
        spec: &'a rmac_apps::LaunchSpec,
    ) -> BackendFuture<'a, Result<rmac_app_launch::Outcome, BackendError>> {
        Box::pin(async move {
            self.complete(format!("launch {spec:?}"))?;
            Ok(rmac_app_launch::Outcome {
                process_id: Some(42),
                delivery: rmac_app_launch::Delivery::DirectFallback,
            })
        })
    }

    fn open_setting<'a>(&'a self, pane_id: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.complete(format!("setting {pane_id}")) })
    }

    fn open_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.complete(format!("open {}", path.display())) })
    }

    fn reveal_file<'a>(&'a self, path: &'a Path) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.complete(format!("reveal {}", path.display())) })
    }

    fn copy_text<'a>(&'a self, text: &'a str) -> BackendFuture<'a, Result<(), BackendError>> {
        Box::pin(async move { self.complete(format!("copy {text}")) })
    }
}

fn run(action: rmac_launcher::Action, backend: &FakeBackend) -> Result<Receipt, Error> {
    futures_lite::future::block_on(execute(ActivationId(7), &action, backend))
}

#[test]
fn every_typed_action_dispatches_once_and_returns_a_payload_free_receipt() {
    let backend = FakeBackend::default();
    let actions = [
        rmac_launcher::Action::LaunchApplication {
            app_id: "notes.desktop".into(),
            spec: rmac_apps::LaunchSpec::Command {
                program: "notes".into(),
                args: vec!["--new".into()],
                working_dir: None,
                terminal: false,
            },
        },
        rmac_launcher::Action::RevealApplication {
            source: "/usr/share/applications/notes.desktop".into(),
        },
        rmac_launcher::Action::OpenSetting {
            pane_id: "sound".into(),
        },
        rmac_launcher::Action::OpenFile {
            path: "/home/alex/Report.txt".into(),
        },
        rmac_launcher::Action::RevealFile {
            path: "/home/alex/Report.txt".into(),
        },
        rmac_launcher::Action::CopyText { text: "42".into() },
    ];
    let expected = [
        Outcome::ApplicationLaunched {
            launch: rmac_app_launch::Outcome {
                process_id: Some(42),
                delivery: rmac_app_launch::Delivery::DirectFallback,
            },
        },
        Outcome::ApplicationRevealed,
        Outcome::SettingOpened,
        Outcome::FileOpened,
        Outcome::FileRevealed,
        Outcome::TextCopied,
    ];
    for (action, expected) in actions.into_iter().zip(expected) {
        let receipt = run(action, &backend).expect("action succeeds");
        assert_eq!(receipt.activation, ActivationId(7));
        assert_eq!(receipt.outcome, expected);
    }
    assert_eq!(backend.calls.lock().expect("calls lock").len(), 6);
}

#[test]
fn invalid_actions_never_reach_the_backend() {
    let backend = FakeBackend::default();
    for action in [
        rmac_launcher::Action::OpenSetting {
            pane_id: " ".into(),
        },
        rmac_launcher::Action::RevealApplication {
            source: "relative.desktop".into(),
        },
        rmac_launcher::Action::OpenFile {
            path: "relative.txt".into(),
        },
        rmac_launcher::Action::CopyText {
            text: String::new(),
        },
    ] {
        let error = run(action, &backend).expect_err("action is rejected");
        assert_eq!(error.kind, FailureKind::InvalidAction);
    }
    assert!(backend.calls.lock().expect("calls lock").is_empty());
}

#[test]
fn default_error_text_redacts_file_paths_copy_text_and_backend_details() {
    let backend = FakeBackend::default();
    *backend.failure.lock().expect("failure lock") = Some(BackendError::new(
        FailureKind::Rejected,
        "secret /home/alex/Report.txt",
    ));
    let error = run(
        rmac_launcher::Action::OpenFile {
            path: "/home/alex/Report.txt".into(),
        },
        &backend,
    )
    .expect_err("backend rejects action");
    assert_eq!(error.to_string(), "Could not open the file");
    assert!(!error.to_string().contains("alex"));
    assert!(error.detail().contains("alex"));
}
