use super::*;
use crate::api::{
    set_text_scale_with, snapshot_with, CommandOutput, Runner, MAX_ERROR_BYTES, SCHEMA,
    TEXT_SCALE_KEY,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

struct FakeRunner {
    outputs: RefCell<VecDeque<io::Result<CommandOutput>>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeRunner {
    fn new(outputs: Vec<io::Result<CommandOutput>>) -> Self {
        Self {
            outputs: RefCell::new(outputs.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl Runner for FakeRunner {
    fn run(&self, arguments: &[&str]) -> io::Result<CommandOutput> {
        self.calls
            .borrow_mut()
            .push(arguments.iter().map(|value| (*value).to_string()).collect());
        self.outputs
            .borrow_mut()
            .pop_front()
            .expect("unexpected gsettings call")
    }
}

fn success(stdout: &str) -> io::Result<CommandOutput> {
    Ok(CommandOutput {
        success: true,
        stdout: stdout.to_string(),
    })
}

#[test]
fn missing_gsettings_is_an_unavailable_snapshot() {
    let runner = FakeRunner::new(vec![Err(io::Error::new(
        io::ErrorKind::NotFound,
        "missing",
    ))]);
    let snapshot = snapshot_with(&runner).unwrap();
    assert!(!snapshot.available);
    assert!(!snapshot.writable);
    assert!(snapshot.detail.unwrap().contains("not installed"));
}

#[test]
fn reads_a_policy_locked_factor() {
    let runner = FakeRunner::new(vec![success("1.2\n"), success("false\n")]);
    let snapshot = snapshot_with(&runner).unwrap();
    assert!(snapshot.available);
    assert!(!snapshot.writable);
    assert_eq!(snapshot.factor, 1.2);
    assert!(snapshot.detail.unwrap().contains("locked"));
}

#[test]
fn rejects_invalid_requested_factors_before_running_a_command() {
    let runner = FakeRunner::new(Vec::new());
    assert!(set_text_scale_with(&runner, 0.9).is_err());
    assert!(set_text_scale_with(&runner, f64::NAN).is_err());
    assert!(runner.calls.borrow().is_empty());
}

#[test]
fn sets_and_confirms_the_authoritative_value() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.0\n"),
        success("true\n"),
        success(""),
        success("1.3\n"),
        success("true\n"),
    ]);
    let snapshot = set_text_scale_with(&runner, 1.3).unwrap();
    assert_eq!(snapshot.factor, 1.3);
    let calls = runner.calls.borrow();
    assert_eq!(calls[4], ["set", SCHEMA, TEXT_SCALE_KEY, "1.30"]);
}

#[test]
fn rejects_a_readback_mismatch() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.0\n"),
        success("true\n"),
        success(""),
        success("1.2\n"),
        success("true\n"),
    ]);
    let error = set_text_scale_with(&runner, 1.3).unwrap_err();
    assert!(error.to_string().contains("authority reports 1.20"));
}

#[test]
fn refuses_a_value_or_policy_changed_during_preflight() {
    let runner = FakeRunner::new(vec![
        success("1.0\n"),
        success("true\n"),
        success("1.1\n"),
        success("true\n"),
    ]);
    let error = set_text_scale_with(&runner, 1.3).unwrap_err();
    assert!(error.to_string().contains("changed before save"));
    assert_eq!(runner.calls.borrow().len(), 4);
}

#[test]
fn a_matching_value_is_a_noop() {
    let runner = FakeRunner::new(vec![success("1.3\n"), success("true\n")]);
    assert_eq!(set_text_scale_with(&runner, 1.3).unwrap().factor, 1.3);
    assert_eq!(runner.calls.borrow().len(), 2);
}

#[test]
fn errors_are_bounded_and_control_normalized() {
    let error = Error::new("test", format!("{}\nprivate", "x".repeat(600)));
    assert!(error.to_string().len() <= MAX_ERROR_BYTES + "test: ".len());
    assert!(!error.to_string().contains('\n'));
}
