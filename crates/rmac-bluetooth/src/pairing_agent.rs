#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroize as _;

#[cfg(not(target_os = "macos"))]
use zbus::blocking::{connection::Builder, Connection, Proxy};
#[cfg(not(target_os = "macos"))]
use zbus::message::Header;
#[cfg(not(target_os = "macos"))]
use zbus::zvariant::OwnedObjectPath;

#[cfg(not(target_os = "macos"))]
const SERVICE: &str = "org.bluez";
#[cfg(not(target_os = "macos"))]
const AGENT_PATH: &str = "/org/rmac/SystemSettings/BluetoothAgent";
#[cfg(not(target_os = "macos"))]
const AGENT_MANAGER_PATH: &str = "/org/bluez";
#[cfg(not(target_os = "macos"))]
const AGENT_MANAGER_INTERFACE: &str = "org.bluez.AgentManager1";
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairingPromptId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingPromptKind {
    ConfirmPasskey { passkey: u32 },
    EnterPinCode,
    EnterPasskey,
    AuthorizePairing,
    AuthorizeService { uuid: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingPrompt {
    pub id: PairingPromptId,
    pub kind: PairingPromptKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingEvent {
    Prompt(PairingPrompt),
    DisplayPinCode { pin_code: String },
    DisplayPasskey { passkey: u32, entered: u16 },
    Canceled,
    TimedOut,
}

pub struct PairingPinCode(String);

impl PairingPinCode {
    pub fn new(mut value: String) -> Result<Self, PairingInputError> {
        let valid = (1..=16).contains(&value.len())
            && value.is_ascii()
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric());
        if !valid {
            value.zeroize();
            return Err(PairingInputError::PinCode);
        }
        Ok(Self(value))
    }

    fn into_string(mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl fmt::Debug for PairingPinCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairingPinCode(<redacted>)")
    }
}

impl Drop for PairingPinCode {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(PartialEq, Eq)]
pub struct PairingPasskey(u32);

impl PairingPasskey {
    pub fn new(mut value: String) -> Result<Self, PairingInputError> {
        let valid =
            (1..=6).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit());
        let parsed = valid.then(|| value.parse::<u32>().ok()).flatten();
        value.zeroize();
        parsed
            .filter(|passkey| *passkey <= 999_999)
            .map(Self)
            .ok_or(PairingInputError::Passkey)
    }

    fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for PairingPasskey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PairingPasskey(<redacted>)")
    }
}

impl Drop for PairingPasskey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingInputError {
    PinCode,
    Passkey,
}

impl fmt::Display for PairingInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PinCode => "enter 1–16 letters or numbers",
            Self::Passkey => "enter up to six digits from 0 to 999999",
        })
    }
}

impl std::error::Error for PairingInputError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedResponse {
    Confirmation,
    PinCode,
    Passkey,
}

enum PairingResponse {
    Accept,
    PinCode(PairingPinCode),
    Passkey(PairingPasskey),
    Reject,
}

struct PendingResponse {
    id: PairingPromptId,
    expected: ExpectedResponse,
    response: Option<PairingResponse>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PairingOutcome {
    pub(super) canceled: bool,
    pub(super) rejected: bool,
    pub(super) timed_out: bool,
}

#[derive(Default)]
struct PairingState {
    next_id: u64,
    pending: Option<PendingResponse>,
    canceled: bool,
    rejected: bool,
    timed_out: bool,
    finished: bool,
}

struct PairingInner {
    state: Mutex<PairingState>,
    changed: Condvar,
}

#[derive(Clone)]
pub struct PairingSession {
    inner: Arc<PairingInner>,
    events: async_channel::Sender<PairingEvent>,
}

impl PairingSession {
    pub fn new() -> (Self, async_channel::Receiver<PairingEvent>) {
        let (events, receiver) = async_channel::bounded(16);
        (
            Self {
                inner: Arc::new(PairingInner {
                    state: Mutex::new(PairingState::default()),
                    changed: Condvar::new(),
                }),
                events,
            },
            receiver,
        )
    }

    pub fn accept(&self, id: PairingPromptId) -> bool {
        self.respond(id, PairingResponse::Accept)
    }

    pub fn submit_pin_code(&self, id: PairingPromptId, value: PairingPinCode) -> bool {
        self.respond(id, PairingResponse::PinCode(value))
    }

    pub fn submit_passkey(&self, id: PairingPromptId, value: PairingPasskey) -> bool {
        self.respond(id, PairingResponse::Passkey(value))
    }

    pub fn reject(&self, id: PairingPromptId) -> bool {
        self.respond(id, PairingResponse::Reject)
    }

    pub fn cancel(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.canceled = true;
            state.pending = None;
            self.inner.changed.notify_all();
        }
        let _ = self.events.try_send(PairingEvent::Canceled);
    }

    fn respond(&self, id: PairingPromptId, response: PairingResponse) -> bool {
        let Ok(mut state) = self.inner.state.lock() else {
            return false;
        };
        let Some(pending) = &mut state.pending else {
            return false;
        };
        if pending.id != id || pending.response.is_some() {
            return false;
        }
        let response_matches = matches!(
            (&pending.expected, &response),
            (
                ExpectedResponse::Confirmation,
                PairingResponse::Accept | PairingResponse::Reject
            ) | (
                ExpectedResponse::PinCode,
                PairingResponse::PinCode(_) | PairingResponse::Reject
            ) | (
                ExpectedResponse::Passkey,
                PairingResponse::Passkey(_) | PairingResponse::Reject
            )
        );
        if !response_matches {
            return false;
        }
        pending.response = Some(response);
        self.inner.changed.notify_all();
        true
    }

    fn request(
        &self,
        expected: ExpectedResponse,
        kind: PairingPromptKind,
    ) -> Result<PairingResponse, RequestFailure> {
        self.request_with_timeout(expected, kind, PROMPT_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        expected: ExpectedResponse,
        kind: PairingPromptKind,
        timeout: Duration,
    ) -> Result<PairingResponse, RequestFailure> {
        let id = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| RequestFailure::Canceled)?;
            if state.canceled || state.finished {
                return Err(RequestFailure::Canceled);
            }
            if state.pending.is_some() {
                return Err(RequestFailure::Rejected);
            }
            state.next_id = state.next_id.saturating_add(1).max(1);
            let id = PairingPromptId(state.next_id);
            state.pending = Some(PendingResponse {
                id,
                expected,
                response: None,
            });
            id
        };

        if self
            .events
            .try_send(PairingEvent::Prompt(PairingPrompt { id, kind }))
            .is_err()
        {
            self.cancel();
            return Err(RequestFailure::Canceled);
        }

        let deadline = Instant::now() + timeout;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| RequestFailure::Canceled)?;
        loop {
            if state.canceled || state.finished {
                state.pending = None;
                return Err(RequestFailure::Canceled);
            }
            if state.pending.as_ref().map(|pending| pending.id) != Some(id) {
                return Err(RequestFailure::Canceled);
            }
            if let Some(response) = state
                .pending
                .as_mut()
                .and_then(|pending| pending.response.take())
            {
                state.pending = None;
                if matches!(response, PairingResponse::Reject) {
                    state.rejected = true;
                    return Err(RequestFailure::Rejected);
                }
                return Ok(response);
            }

            let now = Instant::now();
            if now >= deadline {
                state.pending = None;
                state.timed_out = true;
                drop(state);
                let _ = self.events.try_send(PairingEvent::TimedOut);
                return Err(RequestFailure::TimedOut);
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, _) = self
                .inner
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| RequestFailure::Canceled)?;
            state = next;
        }
    }

    fn display(&self, event: PairingEvent) -> Result<(), RequestFailure> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| RequestFailure::Canceled)?;
        if state.canceled || state.finished {
            return Err(RequestFailure::Canceled);
        }
        drop(state);
        self.events
            .try_send(event)
            .map_err(|_| RequestFailure::Canceled)
    }

    fn agent_request_canceled(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.pending = None;
            self.inner.changed.notify_all();
        }
        let _ = self.events.try_send(PairingEvent::Canceled);
    }

    pub(super) fn finish(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.finished = true;
            state.pending = None;
            self.inner.changed.notify_all();
        }
        self.events.close();
    }

    pub(super) fn outcome(&self) -> PairingOutcome {
        self.inner.state.lock().map_or(
            PairingOutcome {
                canceled: true,
                ..PairingOutcome::default()
            },
            |state| PairingOutcome {
                canceled: state.canceled,
                rejected: state.rejected,
                timed_out: state.timed_out,
            },
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestFailure {
    Canceled,
    Rejected,
    TimedOut,
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug, PartialEq, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error", impl_display = true)]
enum PairingAgentError {
    Rejected(String),
    Canceled(String),
}

#[cfg(not(target_os = "macos"))]
impl From<RequestFailure> for PairingAgentError {
    fn from(failure: RequestFailure) -> Self {
        match failure {
            RequestFailure::Rejected => Self::Rejected("the pairing request was rejected".into()),
            RequestFailure::Canceled => Self::Canceled("the pairing request was canceled".into()),
            RequestFailure::TimedOut => Self::Canceled("the pairing request timed out".into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct PairingAgent {
    device: OwnedObjectPath,
    session: PairingSession,
    /// BlueZ's unique bus name when the agent registered; only it may call.
    service_owner: String,
}

#[cfg(not(target_os = "macos"))]
impl PairingAgent {
    fn new(device: OwnedObjectPath, session: PairingSession, service_owner: String) -> Self {
        Self {
            device,
            session,
            service_owner,
        }
    }

    fn sent_by_service(&self, header: &Header<'_>) -> bool {
        caller_is_service(
            header.sender().map(|name| name.as_str()),
            &self.service_owner,
        )
    }

    fn check_caller(&self, header: &Header<'_>) -> Result<(), PairingAgentError> {
        if self.sent_by_service(header) {
            Ok(())
        } else {
            Err(PairingAgentError::Rejected(
                "pairing requests are accepted only from BlueZ".into(),
            ))
        }
    }

    fn check_device(&self, device: &OwnedObjectPath) -> Result<(), PairingAgentError> {
        if device == &self.device {
            Ok(())
        } else {
            Err(PairingAgentError::Rejected(
                "the request did not match the selected Bluetooth device".into(),
            ))
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[zbus::interface(name = "org.bluez.Agent1")]
impl PairingAgent {
    fn release(&self, #[zbus(header)] header: Header<'_>) {
        if self.sent_by_service(&header) {
            self.session.cancel();
        }
    }

    fn request_pin_code(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<String, PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        match self
            .session
            .request(ExpectedResponse::PinCode, PairingPromptKind::EnterPinCode)?
        {
            PairingResponse::PinCode(value) => Ok(value.into_string()),
            _ => Err(PairingAgentError::Rejected(
                "a PIN code response was required".into(),
            )),
        }
    }

    fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pin_code: String,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        self.session
            .display(PairingEvent::DisplayPinCode { pin_code })?;
        Ok(())
    }

    fn request_passkey(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<u32, PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        match self
            .session
            .request(ExpectedResponse::Passkey, PairingPromptKind::EnterPasskey)?
        {
            PairingResponse::Passkey(value) => Ok(value.get()),
            _ => Err(PairingAgentError::Rejected(
                "a numeric passkey response was required".into(),
            )),
        }
    }

    fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        self.session
            .display(PairingEvent::DisplayPasskey { passkey, entered })?;
        Ok(())
    }

    fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        self.session.request(
            ExpectedResponse::Confirmation,
            PairingPromptKind::ConfirmPasskey { passkey },
        )?;
        Ok(())
    }

    fn request_authorization(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        self.session.request(
            ExpectedResponse::Confirmation,
            PairingPromptKind::AuthorizePairing,
        )?;
        Ok(())
    }

    fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), PairingAgentError> {
        self.check_caller(&header)?;
        self.check_device(&device)?;
        self.session.request(
            ExpectedResponse::Confirmation,
            PairingPromptKind::AuthorizeService { uuid },
        )?;
        Ok(())
    }

    fn cancel(&self, #[zbus(header)] header: Header<'_>) {
        if self.sent_by_service(&header) {
            self.session.agent_request_canceled();
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) struct RegisteredPairingAgent {
    connection: Connection,
}

/// Outgoing calls on this connection (to apps, portals or the bus) give up
/// after this long, so a peer that never replies cannot hold a call open.
#[cfg(not(target_os = "macos"))]
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(not(target_os = "macos"))]
impl RegisteredPairingAgent {
    pub(super) fn register(device: OwnedObjectPath, session: PairingSession) -> zbus::Result<Self> {
        let connection = Builder::system()?.method_timeout(CALL_TIMEOUT).build()?;
        let owner = zbus::blocking::fdo::DBusProxy::new(&connection)?
            .get_name_owner(SERVICE.try_into()?)?
            .to_string();
        connection
            .object_server()
            .at(AGENT_PATH, PairingAgent::new(device, session, owner))?;
        let agent_path = OwnedObjectPath::try_from(AGENT_PATH)?;
        agent_manager(&connection)?
            .call::<_, _, ()>("RegisterAgent", &(agent_path, "KeyboardDisplay"))?;
        Ok(Self { connection })
    }

    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(not(target_os = "macos"))]
impl Drop for RegisteredPairingAgent {
    fn drop(&mut self) {
        if let (Ok(proxy), Ok(path)) = (
            agent_manager(&self.connection),
            OwnedObjectPath::try_from(AGENT_PATH),
        ) {
            let _ = proxy.call::<_, _, ()>("UnregisterAgent", &(path,));
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn agent_manager(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(
        connection,
        SERVICE,
        AGENT_MANAGER_PATH,
        AGENT_MANAGER_INTERFACE,
    )
}

/// Whether a call came from the service's unique name. Stock system-bus
/// policy lets only root send these, but a permissive policy must not let
/// another user show fake pairing prompts or answer them.
fn caller_is_service(sender: Option<&str>, owner: &str) -> bool {
    !owner.is_empty() && sender == Some(owner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bluez_s_unique_name_is_a_valid_caller() {
        assert!(caller_is_service(Some(":1.3"), ":1.3"));
        assert!(!caller_is_service(Some(":1.4"), ":1.3"));
        assert!(!caller_is_service(None, ":1.3"));
        assert!(!caller_is_service(Some(":1.3"), ""));
    }

    #[test]
    fn pin_codes_are_bounded_ascii_and_redacted() {
        let pin = PairingPinCode::new("A1b2".into()).unwrap();
        assert_eq!(format!("{pin:?}"), "PairingPinCode(<redacted>)");
        assert_eq!(pin.into_string(), "A1b2");
        assert!(PairingPinCode::new("".into()).is_err());
        assert!(PairingPinCode::new("1234-5678".into()).is_err());
        assert!(PairingPinCode::new("12345678901234567".into()).is_err());
    }

    #[test]
    fn passkeys_accept_leading_zeroes_and_reject_other_text() {
        let passkey = PairingPasskey::new("000042".into()).unwrap();
        assert_eq!(format!("{passkey:?}"), "PairingPasskey(<redacted>)");
        assert_eq!(passkey.get(), 42);
        assert!(PairingPasskey::new("1000000".into()).is_err());
        assert!(PairingPasskey::new("12a456".into()).is_err());
    }

    #[test]
    fn confirmation_requires_the_current_prompt_identifier() {
        let (session, events) = PairingSession::new();
        let worker = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request(
                    ExpectedResponse::Confirmation,
                    PairingPromptKind::ConfirmPasskey { passkey: 42 },
                )
            })
        };
        let PairingEvent::Prompt(prompt) = events.recv_blocking().unwrap() else {
            panic!("expected pairing prompt");
        };
        assert!(!session.accept(PairingPromptId(prompt.id.0 + 1)));
        assert!(session.accept(prompt.id));
        assert!(matches!(
            worker.join().unwrap(),
            Ok(PairingResponse::Accept)
        ));
    }

    #[test]
    fn rejecting_a_prompt_is_recorded() {
        let (session, events) = PairingSession::new();
        let worker = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request(
                    ExpectedResponse::Confirmation,
                    PairingPromptKind::AuthorizePairing,
                )
            })
        };
        let PairingEvent::Prompt(prompt) = events.recv_blocking().unwrap() else {
            panic!("expected pairing prompt");
        };
        assert!(session.reject(prompt.id));
        assert!(matches!(
            worker.join().unwrap(),
            Err(RequestFailure::Rejected)
        ));
        assert!(session.outcome().rejected);
    }

    #[test]
    fn cancellation_releases_a_waiting_agent_request() {
        let (session, events) = PairingSession::new();
        let worker = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request(ExpectedResponse::PinCode, PairingPromptKind::EnterPinCode)
            })
        };
        assert!(matches!(
            events.recv_blocking(),
            Ok(PairingEvent::Prompt(_))
        ));
        session.cancel();
        assert!(matches!(
            worker.join().unwrap(),
            Err(RequestFailure::Canceled)
        ));
    }

    #[test]
    fn unanswered_prompts_time_out_and_publish_the_outcome() {
        let (session, events) = PairingSession::new();
        let worker = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request_with_timeout(
                    ExpectedResponse::Confirmation,
                    PairingPromptKind::AuthorizePairing,
                    Duration::from_millis(5),
                )
            })
        };
        assert!(matches!(
            events.recv_blocking(),
            Ok(PairingEvent::Prompt(_))
        ));
        assert!(matches!(
            worker.join().unwrap(),
            Err(RequestFailure::TimedOut)
        ));
        assert!(matches!(events.recv_blocking(), Ok(PairingEvent::TimedOut)));
        assert!(session.outcome().timed_out);
    }

    #[test]
    fn bluez_cancel_clears_one_request_without_canceling_the_session() {
        let (session, events) = PairingSession::new();
        let first = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request(
                    ExpectedResponse::Confirmation,
                    PairingPromptKind::AuthorizePairing,
                )
            })
        };
        assert!(matches!(
            events.recv_blocking(),
            Ok(PairingEvent::Prompt(_))
        ));
        session.agent_request_canceled();
        assert!(matches!(
            first.join().unwrap(),
            Err(RequestFailure::Canceled)
        ));
        assert!(matches!(events.recv_blocking(), Ok(PairingEvent::Canceled)));

        let second = {
            let session = session.clone();
            std::thread::spawn(move || {
                session.request(
                    ExpectedResponse::Confirmation,
                    PairingPromptKind::ConfirmPasskey { passkey: 7 },
                )
            })
        };
        let PairingEvent::Prompt(prompt) = events.recv_blocking().unwrap() else {
            panic!("expected replacement pairing prompt");
        };
        assert!(session.accept(prompt.id));
        assert!(matches!(
            second.join().unwrap(),
            Ok(PairingResponse::Accept)
        ));
    }
}
