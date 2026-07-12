//! Bounded transport between the PAM worker and the lock-screen event loop.
//!
//! PAM calls [`Conversation::respond`] on its dedicated worker. The returned
//! [`ConversationUi`] is owned by the lock UI, which renders one prompt and
//! moves one typed response back. Dropping either the UI endpoint or an active
//! [`PendingPrompt`] wakes the worker and fails closed.

use std::fmt;
use std::num::NonZeroU64;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use zeroize::Zeroize as _;

use crate::pam_conversation::{
    Conversation, ConversationError, Reply, Request, RequestKind, MAX_BINARY_RESPONSE_BYTES,
};
use crate::MAX_SECRET_BYTES;

/// Construct the two endpoints of one PAM conversation.
///
/// The request channel has capacity one and the worker waits for that request's
/// response before another request can be emitted, so prompts cannot build up
/// behind a stalled UI.
pub fn conversation_channel() -> (BrokerConversation, ConversationUi) {
    let (requests, incoming) = mpsc::sync_channel(1);
    let active = Arc::new(ActivePrompt::default());
    (
        BrokerConversation {
            requests,
            next_id: NonZeroU64::new(1),
            active: Arc::clone(&active),
        },
        ConversationUi { incoming, active },
    )
}

/// PAM-worker endpoint implementing the checked conversation contract.
pub struct BrokerConversation {
    requests: SyncSender<Exchange>,
    next_id: Option<NonZeroU64>,
    active: Arc<ActivePrompt>,
}

impl fmt::Debug for BrokerConversation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrokerConversation(<redacted>)")
    }
}

impl Conversation for BrokerConversation {
    fn respond(&mut self, request: Request<'_>) -> Result<Reply, ConversationError> {
        let id = self.next_id.ok_or(ConversationError::Unavailable)?;
        self.next_id = id.get().checked_add(1).and_then(NonZeroU64::new);

        let prompt = Prompt::from_request(id, request)?;
        let expected = prompt.kind();
        let (respond, response) = mpsc::channel();
        if !self.active.install(PromptId(id), respond) {
            return Err(ConversationError::Unavailable);
        }
        self.requests.send(Exchange { prompt }).map_err(|_| {
            self.active
                .complete(PromptId(id), Err(ConversationError::Unavailable));
            ConversationError::Unavailable
        })?;

        let reply = response
            .recv()
            .map_err(|_| ConversationError::Cancelled)??;
        if !reply.matches(expected) {
            return Err(ConversationError::InvalidResponse);
        }
        Ok(reply)
    }
}

/// Lock-UI endpoint. It is intentionally not cloneable: one event loop owns a
/// conversation and therefore one place owns each live prompt.
pub struct ConversationUi {
    incoming: Receiver<Exchange>,
    active: Arc<ActivePrompt>,
}

impl ConversationUi {
    /// Poll without blocking the render/event loop.
    pub fn try_prompt(&self) -> Result<Option<PendingPrompt>, UiDisconnected> {
        match self.incoming.try_recv() {
            Ok(exchange) => Ok(Some(self.pending(exchange))),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(UiDisconnected),
        }
    }

    /// Wait for a bounded period. This is useful for a worker-integrated event
    /// source and focused tests; interactive rendering should use
    /// [`Self::try_prompt`].
    pub fn prompt_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<PendingPrompt>, UiDisconnected> {
        match self.incoming.recv_timeout(timeout) {
            Ok(exchange) => Ok(Some(self.pending(exchange))),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(UiDisconnected),
        }
    }

    fn pending(&self, exchange: Exchange) -> PendingPrompt {
        PendingPrompt {
            prompt: exchange.prompt,
            active: Arc::clone(&self.active),
        }
    }
}

impl fmt::Debug for ConversationUi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConversationUi(<redacted>)")
    }
}

impl Drop for ConversationUi {
    fn drop(&mut self) {
        self.active.cancel_current();
    }
}

/// Opaque monotonic identity for one prompt in a conversation.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct PromptId(NonZeroU64);

impl fmt::Debug for PromptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PromptId(<redacted>)")
    }
}

/// One owned, bounded prompt safe to retain while the PAM worker waits.
pub struct Prompt {
    id: PromptId,
    kind: RequestKind,
    content: PromptContent,
}

impl Prompt {
    fn from_request(id: NonZeroU64, request: Request<'_>) -> Result<Self, ConversationError> {
        let (kind, content) = match request {
            Request::EchoOn(text) => (RequestKind::EchoOn, PromptContent::text(text.to_bytes())?),
            Request::EchoOff(text) => (RequestKind::EchoOff, PromptContent::text(text.to_bytes())?),
            Request::Info(text) => (RequestKind::Info, PromptContent::text(text.to_bytes())?),
            Request::Error(text) => (RequestKind::Error, PromptContent::text(text.to_bytes())?),
            Request::Radio(text) => (RequestKind::Radio, PromptContent::text(text.to_bytes())?),
            Request::Binary { kind, data } => {
                if data.len() > MAX_BINARY_RESPONSE_BYTES {
                    return Err(ConversationError::Unavailable);
                }
                (
                    RequestKind::Binary,
                    PromptContent::Binary {
                        kind,
                        bytes: data.to_vec(),
                    },
                )
            }
        };
        Ok(Self {
            id: PromptId(id),
            kind,
            content,
        })
    }

    pub fn id(&self) -> PromptId {
        self.id
    }

    pub fn kind(&self) -> RequestKind {
        self.kind
    }

    /// Expose a textual prompt only for immediate presentation.
    pub fn text<R>(&self, use_text: impl FnOnce(Option<&[u8]>) -> R) -> R {
        match &self.content {
            PromptContent::Text(bytes) => use_text(Some(bytes)),
            PromptContent::Binary { .. } => use_text(None),
        }
    }

    /// Expose a binary prompt only for immediate presentation or dispatch.
    pub fn binary<R>(&self, use_binary: impl FnOnce(Option<(u8, &[u8])>) -> R) -> R {
        match &self.content {
            PromptContent::Text(_) => use_binary(None),
            PromptContent::Binary { kind, bytes } => use_binary(Some((*kind, bytes))),
        }
    }
}

impl fmt::Debug for Prompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Prompt")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("content", &"<redacted>")
            .finish()
    }
}

enum PromptContent {
    Text(Vec<u8>),
    Binary { kind: u8, bytes: Vec<u8> },
}

impl PromptContent {
    fn text(bytes: &[u8]) -> Result<Self, ConversationError> {
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(ConversationError::Unavailable);
        }
        Ok(Self::Text(bytes.to_vec()))
    }
}

impl Drop for PromptContent {
    fn drop(&mut self) {
        match self {
            Self::Text(bytes) | Self::Binary { bytes, .. } => bytes.zeroize(),
        }
    }
}

/// A prompt plus its unique, single-use response capability.
pub struct PendingPrompt {
    prompt: Prompt,
    active: Arc<ActivePrompt>,
}

impl PendingPrompt {
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// Move the response to PAM. A style mismatch is reported to both sides
    /// and can never reach the C callback.
    pub fn respond(self, reply: Reply) -> Result<(), PromptResponseError> {
        let response = if reply.matches(self.prompt.kind()) {
            Ok(reply)
        } else {
            Err(ConversationError::InvalidResponse)
        };
        let valid = response.is_ok();
        if !self.active.complete(self.prompt.id(), response) {
            return Err(PromptResponseError::WorkerUnavailable);
        }
        if valid {
            Ok(())
        } else {
            Err(PromptResponseError::InvalidResponse)
        }
    }

    /// Explicitly cancel this PAM conversation prompt.
    pub fn cancel(self) -> Result<(), PromptResponseError> {
        if self
            .active
            .complete(self.prompt.id(), Err(ConversationError::Cancelled))
        {
            Ok(())
        } else {
            Err(PromptResponseError::WorkerUnavailable)
        }
    }
}

impl fmt::Debug for PendingPrompt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingPrompt")
            .field("prompt", &self.prompt)
            .finish_non_exhaustive()
    }
}

impl Drop for PendingPrompt {
    fn drop(&mut self) {
        self.active
            .complete(self.prompt.id(), Err(ConversationError::Cancelled));
    }
}

struct Exchange {
    prompt: Prompt,
}

#[derive(Default)]
struct ActivePrompt {
    response: Mutex<Option<ActiveResponse>>,
}

impl ActivePrompt {
    fn install(
        &self,
        id: PromptId,
        respond: mpsc::Sender<Result<Reply, ConversationError>>,
    ) -> bool {
        let mut response = self.lock();
        if response.is_some() {
            return false;
        }
        *response = Some(ActiveResponse { id, respond });
        true
    }

    fn complete(&self, id: PromptId, result: Result<Reply, ConversationError>) -> bool {
        let respond = {
            let mut response = self.lock();
            if response.as_ref().is_some_and(|active| active.id == id) {
                response.take().map(|active| active.respond)
            } else {
                None
            }
        };
        respond.is_some_and(|respond| respond.send(result).is_ok())
    }

    fn cancel_current(&self) {
        let respond = self.lock().take().map(|active| active.respond);
        if let Some(respond) = respond {
            let _ = respond.send(Err(ConversationError::Cancelled));
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<ActiveResponse>> {
        self.response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct ActiveResponse {
    id: PromptId,
    respond: mpsc::Sender<Result<Reply, ConversationError>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDisconnected;

impl fmt::Display for UiDisconnected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PAM conversation worker is unavailable")
    }
}

impl std::error::Error for UiDisconnected {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptResponseError {
    InvalidResponse,
    WorkerUnavailable,
}

impl fmt::Display for PromptResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PAM prompt response was not accepted")
    }
}

impl std::error::Error for PromptResponseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pam_conversation::{BinaryResponse, TextResponse};
    use crate::SecretInput;
    use std::thread;

    const WAIT: Duration = Duration::from_secs(2);

    #[test]
    fn moves_a_secret_response_to_the_waiting_worker() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));

        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        assert_eq!(pending.prompt().kind(), RequestKind::EchoOff);
        pending
            .prompt()
            .text(|text| assert_eq!(text, Some(b"Password:".as_slice())));

        let mut input = SecretInput::new();
        input.push('s').unwrap();
        input.push('3').unwrap();
        let secret = input.finish();
        let before = secret.expose(|bytes| bytes.as_ptr() as usize);
        pending.respond(Reply::Secret(secret)).unwrap();

        let reply = worker.join().unwrap().unwrap();
        let Reply::Secret(secret) = reply else {
            panic!("wrong response style");
        };
        secret.expose(|bytes| {
            assert_eq!(bytes, b"s3");
            assert_eq!(bytes.as_ptr() as usize, before);
        });
    }

    #[test]
    fn preserves_order_and_assigns_distinct_redacted_ids() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || {
            conversation.respond(Request::Info(c"First"))?;
            conversation.respond(Request::EchoOn(c"User:"))
        });

        let first = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let first_id = first.prompt().id();
        first.respond(Reply::Acknowledged).unwrap();
        let second = ui.prompt_timeout(WAIT).unwrap().unwrap();
        let second_id = second.prompt().id();
        assert_ne!(first_id, second_id);
        assert_eq!(format!("{first_id:?}"), "PromptId(<redacted>)");
        second
            .respond(Reply::Text(TextResponse::new("jacob").unwrap()))
            .unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Reply::Text(_))));
    }

    #[test]
    fn rejects_a_mismatched_response_on_both_sides() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        assert_eq!(
            pending.respond(Reply::Text(TextResponse::new("wrong style").unwrap())),
            Err(PromptResponseError::InvalidResponse)
        );
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::InvalidResponse)
        ));
    }

    #[test]
    fn drop_and_explicit_cancel_wake_the_worker() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        drop(ui.prompt_timeout(WAIT).unwrap().unwrap());
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));

        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::Info(c"Notice")));
        ui.prompt_timeout(WAIT).unwrap().unwrap().cancel().unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
    }

    #[test]
    fn dropping_ui_cancels_even_if_prompt_state_outlives_it() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || conversation.respond(Request::EchoOff(c"Password:")));
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();

        drop(ui);

        assert!(matches!(
            worker.join().unwrap(),
            Err(ConversationError::Cancelled)
        ));
        assert_eq!(
            pending.respond(Reply::Secret(SecretInput::new().finish())),
            Err(PromptResponseError::WorkerUnavailable)
        );
    }

    #[test]
    fn disconnected_ui_fails_without_stranding_the_worker() {
        let (mut conversation, ui) = conversation_channel();
        drop(ui);
        assert!(matches!(
            conversation.respond(Request::Info(c"Notice")),
            Err(ConversationError::Unavailable)
        ));
    }

    #[test]
    fn bounds_and_redacts_owned_prompt_content() {
        let (mut conversation, ui) = conversation_channel();
        let worker = thread::spawn(move || {
            conversation.respond(Request::Binary {
                kind: 7,
                data: b"private binary",
            })
        });
        let pending = ui.prompt_timeout(WAIT).unwrap().unwrap();
        pending
            .prompt()
            .binary(|binary| assert_eq!(binary, Some((7, b"private binary".as_slice()))));
        let debug = format!("{:?}", pending.prompt());
        assert!(!debug.contains("private"));
        pending
            .respond(Reply::Binary(BinaryResponse::new(7, [1, 2]).unwrap()))
            .unwrap();
        assert!(matches!(worker.join().unwrap(), Ok(Reply::Binary(_))));

        let oversized = vec![0_u8; MAX_BINARY_RESPONSE_BYTES + 1];
        let (mut conversation, _ui) = conversation_channel();
        assert!(matches!(
            conversation.respond(Request::Binary {
                kind: 1,
                data: &oversized,
            }),
            Err(ConversationError::Unavailable)
        ));
    }
}
