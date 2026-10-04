//! One authentication request at a time. polkitd may ask for several at
//! once (two apps, or one app twice); each waits its turn in arrival order,
//! and `CancelAuthentication` removes a waiting request or ends the one on
//! screen. The running request drives the helper conversation and tells the
//! dialog what to show.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_channel::{Receiver, Sender};

use crate::helper::{self, HelperConfig, HelperEvent, HelperSession};
use crate::identity::{self, Identity, WireIdentity};
use crate::secret::Secret;
use crate::text;

/// An unattended dialog gives up after this long, as the design requires.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Wrong passwords before the request ends, as polkit's own agents do.
pub const MAX_FAILURES: u32 = 3;

/// What polkitd asked for, as received.
#[derive(Clone, Debug)]
pub struct Request {
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: HashMap<String, String>,
    pub cookie: String,
    pub identities: Vec<WireIdentity>,
}

/// What the dialog shows for one request. Every string is display text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dialog {
    pub cookie: String,
    pub app_name: String,
    pub message: String,
    /// A freedesktop icon name, already checked to be a plain name.
    pub icon_name: Option<String>,
    pub identities: Vec<Identity>,
    /// The signed-in user is not one of the identities.
    pub administrator_needed: bool,
    pub confirm_label: String,
}

/// Coordinator → dialog.
#[derive(Debug)]
pub enum ToUi {
    Open {
        dialog: Dialog,
        responder: Responder,
    },
    /// The helper is ready for an answer; the field's placeholder and
    /// whether it is a secret.
    Prompt {
        cookie: String,
        placeholder: String,
        echo: bool,
    },
    /// PAM text to show (`error` draws it in red).
    Info {
        cookie: String,
        text: String,
        error: bool,
    },
    /// Checking the password: OK is disabled meanwhile.
    Busy {
        cookie: String,
        busy: bool,
    },
    /// Wrong password: shake, clear the field, try again.
    Retry {
        cookie: String,
    },
    Close {
        cookie: String,
    },
}

/// Dialog → coordinator.
#[derive(Debug)]
pub enum FromUi {
    Submit { identity: usize, secret: Secret },
    SelectIdentity(usize),
    Cancel,
}

#[derive(Debug)]
enum Input {
    Turn,
    User(FromUi),
    Helper(u64, HelperEvent),
    Cancel,
}

/// The dialog's way back to the request it shows.
#[derive(Clone, Debug)]
pub struct Responder {
    inputs: Sender<Input>,
}

impl Responder {
    pub fn send(&self, message: FromUi) {
        let _ = self.inputs.try_send(Input::User(message));
    }
}

/// How a request ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Authorized,
    Cancelled,
    Failed,
}

struct Slot {
    cookie: String,
    inputs: Sender<Input>,
}

#[derive(Default)]
struct State {
    active: Option<Slot>,
    queue: VecDeque<Slot>,
}

/// Facts about the running system the dialog text depends on.
pub trait Environment: Send + Sync {
    fn current_uid(&self) -> u32;
    fn resolve(&self, identities: &[WireIdentity], current_uid: u32) -> Vec<Identity>;
    fn executable_of(&self, pid: u32) -> Option<String>;
}

/// The real system: passwd/group lookups and /proc.
pub struct System;

impl Environment for System {
    fn current_uid(&self) -> u32 {
        identity::current_uid()
    }

    fn resolve(&self, identities: &[WireIdentity], current_uid: u32) -> Vec<Identity> {
        identity::resolve(identities, current_uid)
    }

    fn executable_of(&self, pid: u32) -> Option<String> {
        if pid == 0 {
            return None;
        }
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| {
                std::fs::read_to_string(format!("/proc/{pid}/comm"))
                    .ok()
                    .map(|comm| comm.trim_end().to_owned())
            })
    }
}

pub struct Coordinator {
    state: Mutex<State>,
    ui: Sender<ToUi>,
    helper: HelperConfig,
    environment: Box<dyn Environment>,
}

fn plain_icon_name(name: &str) -> Option<String> {
    (!name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.'))
    .then(|| name.to_owned())
}

impl Coordinator {
    pub fn new(
        ui: Sender<ToUi>,
        helper: HelperConfig,
        environment: Box<dyn Environment>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            ui,
            helper,
            environment,
        })
    }

    /// What the dialog shows for `request`.
    pub fn dialog(&self, request: &Request) -> Dialog {
        let current = self.environment.current_uid();
        let identities = self.environment.resolve(&request.identities, current);
        let app_name = request
            .details
            .get("polkit.subject-pid")
            .and_then(|pid| pid.trim().parse::<u32>().ok())
            .and_then(|pid| self.environment.executable_of(pid))
            .map(|exe| text::app_name(&exe))
            .unwrap_or_else(|| "An app".to_owned());
        let confirm_label = if app_name == "System Settings" {
            "Modify Settings"
        } else {
            "OK"
        };
        Dialog {
            cookie: request.cookie.clone(),
            administrator_needed: !identities.iter().any(|id| id.uid == current),
            identities,
            message: text::display(&request.message, text::MAX_MESSAGE_CHARS),
            icon_name: plain_icon_name(&request.icon_name),
            app_name,
            confirm_label: confirm_label.to_owned(),
        }
    }

    /// Handle `BeginAuthentication`: wait for this request's turn, show it,
    /// and return when it ends.
    pub async fn begin(self: &Arc<Self>, request: Request) -> Outcome {
        let (inputs, inputs_rx) = async_channel::unbounded();
        let first = {
            let mut state = self.state.lock().unwrap();
            let slot = Slot {
                cookie: request.cookie.clone(),
                inputs: inputs.clone(),
            };
            if state.active.is_none() {
                state.active = Some(slot);
                true
            } else {
                state.queue.push_back(slot);
                false
            }
        };
        if !first {
            match inputs_rx.recv().await {
                Ok(Input::Turn) => {}
                _ => {
                    // Cancelled while waiting. If its turn was handed over
                    // at the same moment, pass the turn on.
                    self.forget(&inputs);
                    self.finish(&inputs);
                    return Outcome::Cancelled;
                }
            }
        }
        let outcome = self.run(&request, &inputs, &inputs_rx).await;
        self.finish(&inputs);
        outcome
    }

    /// Handle `CancelAuthentication`.
    pub fn cancel(&self, cookie: &str) {
        let state = self.state.lock().unwrap();
        let slot = state
            .active
            .iter()
            .chain(state.queue.iter())
            .find(|slot| slot.cookie == cookie);
        if let Some(slot) = slot {
            let _ = slot.inputs.try_send(Input::Cancel);
        }
    }

    /// Requests waiting behind the one on screen.
    pub fn queued(&self) -> usize {
        self.state.lock().unwrap().queue.len()
    }

    fn forget(&self, inputs: &Sender<Input>) {
        let mut state = self.state.lock().unwrap();
        state.queue.retain(|slot| !slot.inputs.same_channel(inputs));
    }

    fn finish(&self, inputs: &Sender<Input>) {
        let mut state = self.state.lock().unwrap();
        if state
            .active
            .as_ref()
            .is_some_and(|slot| slot.inputs.same_channel(inputs))
        {
            state.active = state.queue.pop_front();
            if let Some(next) = &state.active {
                let _ = next.inputs.try_send(Input::Turn);
            }
        }
    }

    fn start_helper(
        &self,
        identity: &Identity,
        cookie: &str,
        generation: u64,
        inputs: &Sender<Input>,
    ) -> Option<HelperSession> {
        let events = inputs.clone();
        helper::start(&self.helper, &identity.user_name, cookie, move |event| {
            let _ = events.try_send(Input::Helper(generation, event));
        })
        .map_err(|error| eprintln!("could not start polkit's agent helper: {error}"))
        .ok()
    }

    fn show(&self, message: ToUi) {
        let _ = self.ui.try_send(message);
    }

    async fn run(
        &self,
        request: &Request,
        inputs: &Sender<Input>,
        inputs_rx: &Receiver<Input>,
    ) -> Outcome {
        let dialog = self.dialog(request);
        if dialog.identities.is_empty() {
            return Outcome::Failed;
        }
        let cookie = request.cookie.clone();
        let identities = dialog.identities.clone();
        self.show(ToUi::Open {
            dialog,
            responder: Responder {
                inputs: inputs.clone(),
            },
        });
        let close = |outcome| {
            self.show(ToUi::Close {
                cookie: cookie.clone(),
            });
            outcome
        };

        let mut selected = 0;
        let mut generation = 0_u64;
        let mut failures = 0;
        let mut session = self.start_helper(&identities[selected], &cookie, generation, inputs);
        if session.is_none() {
            return close(Outcome::Failed);
        }
        let mut waiting_prompt = false;
        let mut answered = false;
        let mut early_answer: Option<Secret> = None;
        let mut timer = async_io::Timer::after(IDLE_TIMEOUT);

        loop {
            let input = futures_lite::future::or(async { inputs_rx.recv().await.ok() }, async {
                (&mut timer).await;
                None
            })
            .await;
            let Some(input) = input else {
                // Timed out (or every sender is gone).
                drop(session.take());
                return close(Outcome::Cancelled);
            };
            match input {
                Input::Turn => {}
                Input::Cancel | Input::User(FromUi::Cancel) => {
                    drop(session.take());
                    return close(Outcome::Cancelled);
                }
                Input::Helper(event_generation, _) if event_generation != generation => {}
                Input::Helper(_, HelperEvent::Prompt { echo, text: prompt }) => {
                    if let Some(secret) = early_answer.take() {
                        if let Some(session) = &session {
                            session.answer(secret);
                            answered = true;
                        }
                    } else {
                        waiting_prompt = true;
                        self.show(ToUi::Prompt {
                            cookie: cookie.clone(),
                            placeholder: text::prompt_placeholder(&prompt),
                            echo,
                        });
                        self.show(ToUi::Busy {
                            cookie: cookie.clone(),
                            busy: false,
                        });
                    }
                }
                Input::Helper(_, HelperEvent::Info(text)) => self.show(ToUi::Info {
                    cookie: cookie.clone(),
                    text,
                    error: false,
                }),
                Input::Helper(_, HelperEvent::Error(text)) => self.show(ToUi::Info {
                    cookie: cookie.clone(),
                    text,
                    error: true,
                }),
                Input::Helper(_, HelperEvent::Finished(true)) => {
                    return close(Outcome::Authorized);
                }
                Input::Helper(_, HelperEvent::Finished(false)) => {
                    if !answered {
                        // The helper failed before checking anything: PAM or
                        // polkit refused outright, so asking again cannot help.
                        return close(Outcome::Failed);
                    }
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        return close(Outcome::Failed);
                    }
                    self.show(ToUi::Retry {
                        cookie: cookie.clone(),
                    });
                    generation += 1;
                    waiting_prompt = false;
                    answered = false;
                    session = self.start_helper(&identities[selected], &cookie, generation, inputs);
                    if session.is_none() {
                        return close(Outcome::Failed);
                    }
                }
                Input::User(FromUi::SelectIdentity(index)) => {
                    timer.set_after(IDLE_TIMEOUT);
                    if index != selected && index < identities.len() {
                        selected = index;
                        generation += 1;
                        waiting_prompt = false;
                        answered = false;
                        early_answer = None;
                        drop(session.take());
                        session =
                            self.start_helper(&identities[selected], &cookie, generation, inputs);
                        if session.is_none() {
                            return close(Outcome::Failed);
                        }
                    }
                }
                Input::User(FromUi::Submit { identity, secret }) => {
                    timer.set_after(IDLE_TIMEOUT);
                    self.show(ToUi::Busy {
                        cookie: cookie.clone(),
                        busy: true,
                    });
                    if identity != selected && identity < identities.len() {
                        selected = identity;
                        generation += 1;
                        waiting_prompt = false;
                        answered = false;
                        drop(session.take());
                        session =
                            self.start_helper(&identities[selected], &cookie, generation, inputs);
                        if session.is_none() {
                            return close(Outcome::Failed);
                        }
                    }
                    if waiting_prompt {
                        waiting_prompt = false;
                        if let Some(session) = &session {
                            session.answer(secret);
                            answered = true;
                        }
                    } else {
                        early_answer = Some(secret);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl Environment for Fake {
        fn current_uid(&self) -> u32 {
            1000
        }
        fn resolve(&self, identities: &[WireIdentity], _: u32) -> Vec<Identity> {
            identities
                .iter()
                .enumerate()
                .map(|(index, _)| Identity {
                    uid: 1001 + index as u32,
                    user_name: format!("admin{index}"),
                    display_name: format!("Admin {index}"),
                })
                .collect()
        }
        fn executable_of(&self, pid: u32) -> Option<String> {
            (pid == 42).then(|| "/usr/libexec/rmac/rmac-system-settings".to_owned())
        }
    }

    fn request(cookie: &str, pid: &str) -> Request {
        Request {
            action_id: "org.freedesktop.accounts.user-administration".into(),
            message: "Authentication is required\nto change <b>data</b>".into(),
            icon_name: "../../etc/passwd".into(),
            details: HashMap::from([("polkit.subject-pid".into(), pid.into())]),
            cookie: cookie.into(),
            identities: vec![("unix-user".into(), HashMap::new())],
        }
    }

    #[test]
    fn the_dialog_names_the_app_and_never_trusts_the_icon_path() {
        let (ui, _rx) = async_channel::unbounded();
        let coordinator = Coordinator::new(ui, HelperConfig::default(), Box::new(Fake));
        let dialog = coordinator.dialog(&request("c1", "42"));
        assert_eq!(dialog.app_name, "System Settings");
        assert_eq!(dialog.confirm_label, "Modify Settings");
        assert_eq!(
            dialog.message,
            "Authentication is required to change <b>data</b>"
        );
        assert_eq!(dialog.icon_name, None);
        assert!(dialog.administrator_needed);

        let other = coordinator.dialog(&request("c2", "7"));
        assert_eq!(other.app_name, "An app");
        assert_eq!(other.confirm_label, "OK");
        assert_eq!(plain_icon_name("system-users"), Some("system-users".into()));
    }

    #[test]
    fn a_request_with_no_usable_helper_fails_without_hanging() {
        let (ui, ui_rx) = async_channel::unbounded();
        let coordinator = Coordinator::new(ui, HelperConfig::default(), Box::new(Fake));
        let outcome = async_io::block_on(coordinator.begin(request("c1", "42")));
        assert_eq!(outcome, Outcome::Failed);
        assert!(matches!(ui_rx.try_recv(), Ok(ToUi::Open { .. })));
        assert!(matches!(ui_rx.try_recv(), Ok(ToUi::Close { .. })));
        assert_eq!(coordinator.queued(), 0);
    }
}
