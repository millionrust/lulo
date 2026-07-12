use std::ffi::{c_int, CStr, CString};
use std::fmt;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::thread::{self, JoinHandle};

use pam_sys2::{pam_conv, pam_handle_t};

use super::callback::{converse, PamCallback};
use crate::pam_conversation::Conversation;

const SERVICE: &CStr = c"rmac-lock";

pub fn spawn_authentication<C>(username: String, conversation: C) -> std::io::Result<Worker>
where
    C: Conversation + 'static,
{
    let handle = thread::Builder::new()
        .name("rmac-pam-worker".into())
        .spawn(move || authenticate_on_worker(&username, conversation))?;
    Ok(Worker { handle })
}

fn authenticate_on_worker<C: Conversation>(username: &str, conversation: C) -> Result<(), Error> {
    let username =
        CString::new(username).map_err(|_| Error::local(Stage::Start, LocalCode::Nul))?;
    Transaction::start(&username, conversation)?.run()
}

pub struct Worker {
    handle: JoinHandle<Result<(), Error>>,
}

impl Worker {
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    pub fn join(self) -> Result<(), WorkerError> {
        match self.handle.join() {
            Ok(result) => result.map_err(WorkerError::Pam),
            Err(_) => Err(WorkerError::Panicked),
        }
    }
}

impl fmt::Debug for Worker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PamWorker")
            .field("finished", &self.handle.is_finished())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Pam(Error),
    Panicked,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pam(error) => error.fmt(formatter),
            Self::Panicked => formatter.write_str("PAM worker terminated unexpectedly"),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pam(error) => Some(error),
            Self::Panicked => None,
        }
    }
}

struct Transaction<C: Conversation> {
    handle: NonNull<pam_handle_t>,
    _callback: Box<PamCallback<C>>,
    api: PamApi,
    last_status: c_int,
    ended: bool,
    _thread_bound: PhantomData<Rc<()>>,
}

impl<C: Conversation> Transaction<C> {
    fn start(username: &CStr, conversation: C) -> Result<Self, Error> {
        Self::start_with_api(username, conversation, PamApi::SYSTEM)
    }

    fn start_with_api(username: &CStr, conversation: C, api: PamApi) -> Result<Self, Error> {
        let mut callback = Box::new(PamCallback::new(conversation));
        let pam_conversation = pam_conv {
            conv: Some(converse::<C>),
            appdata_ptr: callback.appdata(),
        };
        let mut raw_handle = ptr::null_mut();
        // SAFETY: strings and pointers remain valid for the call. Callback is
        // boxed at a stable address for the complete transaction.
        let status = unsafe {
            (api.start)(
                SERVICE.as_ptr(),
                username.as_ptr(),
                &pam_conversation,
                &mut raw_handle,
            )
        };
        if status != pam_sys2::PAM_SUCCESS {
            return Err(Error::pam(Stage::Start, status, None));
        }
        let handle = NonNull::new(raw_handle)
            .ok_or_else(|| Error::local(Stage::Start, LocalCode::NullHandle))?;
        Ok(Self {
            handle,
            _callback: callback,
            api,
            last_status: pam_sys2::PAM_SUCCESS,
            ended: false,
            _thread_bound: PhantomData,
        })
    }

    fn run(mut self) -> Result<(), Error> {
        // SAFETY: the successful-start handle is uniquely owned and thread-bound.
        let auth_status = unsafe {
            (self.api.authenticate)(
                self.handle.as_ptr(),
                pam_sys2::PAM_DISALLOW_NULL_AUTHTOK as c_int,
            )
        };
        self.last_status = auth_status;
        if auth_status != pam_sys2::PAM_SUCCESS {
            return self.finish(Err(PamFailure {
                stage: Stage::Authenticate,
                code: auth_status,
            }));
        }

        // Authentication alone is insufficient; account policy decides lock,
        // expiry, time, and access restrictions.
        let account_status = unsafe { (self.api.account)(self.handle.as_ptr(), 0) };
        self.last_status = account_status;
        if account_status != pam_sys2::PAM_SUCCESS {
            return self.finish(Err(PamFailure {
                stage: Stage::Account,
                code: account_status,
            }));
        }
        self.finish(Ok(()))
    }

    fn finish(mut self, result: Result<(), PamFailure>) -> Result<(), Error> {
        let status_for_end = result
            .as_ref()
            .err()
            .map_or(pam_sys2::PAM_SUCCESS, |failure| failure.code);
        // SAFETY: exactly one explicit end for this successful start.
        let end_status = unsafe { (self.api.end)(self.handle.as_ptr(), status_for_end) };
        self.ended = true;
        match (result, end_status) {
            (Ok(()), pam_sys2::PAM_SUCCESS) => Ok(()),
            (Ok(()), code) => Err(Error::pam(Stage::End, code, None)),
            (Err(failure), pam_sys2::PAM_SUCCESS) => {
                Err(Error::pam(failure.stage, failure.code, None))
            }
            (Err(failure), end_code) => {
                Err(Error::pam(failure.stage, failure.code, Some(end_code)))
            }
        }
    }
}

impl<C: Conversation> Drop for Transaction<C> {
    fn drop(&mut self) {
        if !self.ended {
            // SAFETY: fallback for unwind between start and finish; Drop runs once.
            unsafe { (self.api.end)(self.handle.as_ptr(), self.last_status) };
            self.ended = true;
        }
    }
}

#[derive(Clone, Copy)]
struct PamFailure {
    stage: Stage,
    code: c_int,
}

#[derive(Clone, Copy)]
struct PamApi {
    start: unsafe extern "C" fn(
        *const libc::c_char,
        *const libc::c_char,
        *const pam_conv,
        *mut *mut pam_handle_t,
    ) -> c_int,
    authenticate: unsafe extern "C" fn(*mut pam_handle_t, c_int) -> c_int,
    account: unsafe extern "C" fn(*mut pam_handle_t, c_int) -> c_int,
    end: unsafe extern "C" fn(*mut pam_handle_t, c_int) -> c_int,
}

impl PamApi {
    const SYSTEM: Self = Self {
        start: pam_sys2::pam_start,
        authenticate: pam_sys2::pam_authenticate,
        account: pam_sys2::pam_acct_mgmt,
        end: pam_sys2::pam_end,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Start,
    Authenticate,
    Account,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalCode {
    Nul,
    NullHandle,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Error {
    stage: Stage,
    code: ErrorCode,
    end_code: Option<c_int>,
}

impl Error {
    fn pam(stage: Stage, code: c_int, end_code: Option<c_int>) -> Self {
        Self {
            stage,
            code: ErrorCode::Pam(code),
            end_code,
        }
    }

    fn local(stage: Stage, code: LocalCode) -> Self {
        Self {
            stage,
            code: ErrorCode::Local(code),
            end_code: None,
        }
    }

    pub fn stage(self) -> Stage {
        self.stage
    }

    pub fn pam_code(self) -> Option<c_int> {
        match self.code {
            ErrorCode::Pam(code) => Some(code),
            ErrorCode::Local(_) => None,
        }
    }

    pub fn pam_end_code(self) -> Option<c_int> {
        self.end_code
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ErrorCode {
    Pam(c_int),
    Local(LocalCode),
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PamError")
            .field("stage", &self.stage)
            .field("code", &self.code)
            .field("pam_end_failed", &self.end_code.is_some())
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "PAM authentication failed during {:?}",
            self.stage
        )
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::pam_conversation::{ConversationError, Reply, Request};

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static START_STATUS: AtomicI32 = AtomicI32::new(pam_sys2::PAM_SUCCESS);
    static AUTH_STATUS: AtomicI32 = AtomicI32::new(pam_sys2::PAM_SUCCESS);
    static ACCOUNT_STATUS: AtomicI32 = AtomicI32::new(pam_sys2::PAM_SUCCESS);
    static END_STATUS: AtomicI32 = AtomicI32::new(pam_sys2::PAM_SUCCESS);
    static END_ARGUMENT: AtomicI32 = AtomicI32::new(-1);
    static START_CALLS: AtomicUsize = AtomicUsize::new(0);
    static AUTH_CALLS: AtomicUsize = AtomicUsize::new(0);
    static ACCOUNT_CALLS: AtomicUsize = AtomicUsize::new(0);
    static END_CALLS: AtomicUsize = AtomicUsize::new(0);
    static NULL_SUCCESS_HANDLE: AtomicUsize = AtomicUsize::new(0);

    struct UnusedConversation;

    impl Conversation for UnusedConversation {
        fn respond(&mut self, _request: Request<'_>) -> Result<Reply, ConversationError> {
            Err(ConversationError::Unavailable)
        }
    }

    unsafe extern "C" fn fake_start(
        _service: *const libc::c_char,
        _user: *const libc::c_char,
        _conversation: *const pam_conv,
        handle: *mut *mut pam_handle_t,
    ) -> c_int {
        START_CALLS.fetch_add(1, Ordering::SeqCst);
        let status = START_STATUS.load(Ordering::SeqCst);
        if status == pam_sys2::PAM_SUCCESS && NULL_SUCCESS_HANDLE.load(Ordering::SeqCst) == 0 {
            // Never dereferenced by the fake API.
            unsafe { *handle = NonNull::<pam_handle_t>::dangling().as_ptr() };
        }
        status
    }

    unsafe extern "C" fn fake_authenticate(_handle: *mut pam_handle_t, _flags: c_int) -> c_int {
        AUTH_CALLS.fetch_add(1, Ordering::SeqCst);
        AUTH_STATUS.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn fake_account(_handle: *mut pam_handle_t, _flags: c_int) -> c_int {
        ACCOUNT_CALLS.fetch_add(1, Ordering::SeqCst);
        ACCOUNT_STATUS.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn fake_end(_handle: *mut pam_handle_t, status: c_int) -> c_int {
        END_CALLS.fetch_add(1, Ordering::SeqCst);
        END_ARGUMENT.store(status, Ordering::SeqCst);
        END_STATUS.load(Ordering::SeqCst)
    }

    const FAKE_API: PamApi = PamApi {
        start: fake_start,
        authenticate: fake_authenticate,
        account: fake_account,
        end: fake_end,
    };

    fn reset() {
        START_STATUS.store(pam_sys2::PAM_SUCCESS, Ordering::SeqCst);
        AUTH_STATUS.store(pam_sys2::PAM_SUCCESS, Ordering::SeqCst);
        ACCOUNT_STATUS.store(pam_sys2::PAM_SUCCESS, Ordering::SeqCst);
        END_STATUS.store(pam_sys2::PAM_SUCCESS, Ordering::SeqCst);
        END_ARGUMENT.store(-1, Ordering::SeqCst);
        START_CALLS.store(0, Ordering::SeqCst);
        AUTH_CALLS.store(0, Ordering::SeqCst);
        ACCOUNT_CALLS.store(0, Ordering::SeqCst);
        END_CALLS.store(0, Ordering::SeqCst);
        NULL_SUCCESS_HANDLE.store(0, Ordering::SeqCst);
    }

    fn start() -> Result<Transaction<UnusedConversation>, Error> {
        Transaction::start_with_api(c"jacob", UnusedConversation, FAKE_API)
    }

    #[test]
    fn successful_transaction_orders_every_stage_and_ends_once() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        assert!(start().unwrap().run().is_ok());
        assert_eq!(START_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(AUTH_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(ACCOUNT_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(END_ARGUMENT.load(Ordering::SeqCst), pam_sys2::PAM_SUCCESS);
    }

    #[test]
    fn authentication_failure_skips_account_and_reaches_pam_end() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        AUTH_STATUS.store(pam_sys2::PAM_AUTH_ERR, Ordering::SeqCst);
        let error = start().unwrap().run().unwrap_err();
        assert_eq!(error.stage(), Stage::Authenticate);
        assert_eq!(error.pam_code(), Some(pam_sys2::PAM_AUTH_ERR));
        assert_eq!(ACCOUNT_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(END_ARGUMENT.load(Ordering::SeqCst), pam_sys2::PAM_AUTH_ERR);
    }

    #[test]
    fn account_and_end_failures_are_both_preserved() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        ACCOUNT_STATUS.store(pam_sys2::PAM_ACCT_EXPIRED, Ordering::SeqCst);
        END_STATUS.store(pam_sys2::PAM_SYSTEM_ERR, Ordering::SeqCst);
        let error = start().unwrap().run().unwrap_err();
        assert_eq!(error.stage(), Stage::Account);
        assert_eq!(error.pam_code(), Some(pam_sys2::PAM_ACCT_EXPIRED));
        assert_eq!(error.pam_end_code(), Some(pam_sys2::PAM_SYSTEM_ERR));
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_start_and_success_with_null_handle_never_call_pam_end() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        START_STATUS.store(pam_sys2::PAM_SERVICE_ERR, Ordering::SeqCst);
        assert!(matches!(start(), Err(error) if error.stage() == Stage::Start));
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 0);

        reset();
        NULL_SUCCESS_HANDLE.store(1, Ordering::SeqCst);
        assert!(matches!(start(), Err(error) if error.stage() == Stage::Start));
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn successful_policy_with_failed_end_reports_end_stage() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        END_STATUS.store(pam_sys2::PAM_SYSTEM_ERR, Ordering::SeqCst);
        let error = start().unwrap().run().unwrap_err();
        assert_eq!(error.stage(), Stage::End);
        assert_eq!(error.pam_code(), Some(pam_sys2::PAM_SYSTEM_ERR));
        assert_eq!(END_CALLS.load(Ordering::SeqCst), 1);
    }
}
