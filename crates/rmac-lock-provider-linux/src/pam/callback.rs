use std::mem::{self, ManuallyDrop};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{self, NonNull};
use std::slice;

use libc::{c_char, c_int, c_void, size_t};
use pam_sys2::{pam_message, pam_response};

use crate::pam_conversation::{Conversation, Reply, Request};

const MAX_MESSAGES: usize = pam_sys2::PAM_MAX_NUM_MSG as usize;
const MAX_MESSAGE_BYTES: usize = pam_sys2::PAM_MAX_MSG_SIZE as usize;

pub(super) struct PamCallback<C> {
    conversation: C,
    allocator: Allocator,
}

impl<C> PamCallback<C> {
    pub(super) fn new(conversation: C) -> Self {
        Self {
            conversation,
            allocator: Allocator::SYSTEM,
        }
    }

    pub(super) fn appdata(&mut self) -> *mut c_void {
        (self as *mut Self).cast()
    }
}

#[derive(Clone, Copy)]
struct Allocator {
    calloc: unsafe extern "C" fn(size_t, size_t) -> *mut c_void,
    free: unsafe extern "C" fn(*mut c_void),
    zero: unsafe extern "C" fn(*mut c_void, size_t),
}

impl Allocator {
    const SYSTEM: Self = Self {
        calloc: libc::calloc,
        free: libc::free,
        zero: libc::explicit_bzero,
    };
}

struct ResponseArray {
    pointer: NonNull<pam_response>,
    count: usize,
    lengths: [usize; MAX_MESSAGES],
    allocator: Allocator,
}

impl ResponseArray {
    fn new(count: usize, allocator: Allocator) -> Result<Self, CallbackError> {
        // SAFETY: count is in 1..=32 and the result is checked before use.
        let raw = unsafe { (allocator.calloc)(count, mem::size_of::<pam_response>()) };
        let pointer = NonNull::new(raw.cast()).ok_or(CallbackError::Buffer)?;
        Ok(Self {
            pointer,
            count,
            lengths: [0; MAX_MESSAGES],
            allocator,
        })
    }

    fn put_text(&mut self, index: usize, value: &[u8]) -> Result<(), CallbackError> {
        if value.len() > crate::MAX_SECRET_BYTES || value.contains(&0) {
            return Err(CallbackError::Conversation);
        }
        let allocation_len = value.len() + 1;
        // SAFETY: nonzero bounded size; the result is checked before use.
        let raw = unsafe { (self.allocator.calloc)(allocation_len, 1) };
        let response = NonNull::new(raw.cast::<c_char>()).ok_or(CallbackError::Buffer)?;
        // SAFETY: allocation_len includes the zero byte supplied by calloc.
        unsafe { ptr::copy_nonoverlapping(value.as_ptr(), response.as_ptr().cast(), value.len()) };
        self.item_mut(index).resp = response.as_ptr();
        self.lengths[index] = allocation_len;
        Ok(())
    }

    fn put_binary(&mut self, index: usize, kind: u8, value: &[u8]) -> Result<(), CallbackError> {
        let allocation_len = value.len().checked_add(5).ok_or(CallbackError::Buffer)?;
        if allocation_len > crate::MAX_SECRET_BYTES {
            return Err(CallbackError::Conversation);
        }
        // SAFETY: nonzero bounded size; the result is checked before use.
        let raw = unsafe { (self.allocator.calloc)(allocation_len, 1) };
        let response = NonNull::new(raw.cast::<u8>()).ok_or(CallbackError::Buffer)?;
        let total = (allocation_len as u32).to_be_bytes();
        // SAFETY: every copy remains within the allocation_len-byte object.
        unsafe {
            ptr::copy_nonoverlapping(total.as_ptr(), response.as_ptr(), total.len());
            *response.as_ptr().add(4) = kind;
            ptr::copy_nonoverlapping(value.as_ptr(), response.as_ptr().add(5), value.len());
        }
        self.item_mut(index).resp = response.as_ptr().cast();
        self.lengths[index] = allocation_len;
        Ok(())
    }

    fn item_mut(&mut self, index: usize) -> &mut pam_response {
        debug_assert!(index < self.count);
        // SAFETY: every caller provides an index within the bounded array.
        unsafe { &mut *self.pointer.as_ptr().add(index) }
    }

    fn into_raw(self) -> *mut pam_response {
        ManuallyDrop::new(self).pointer.as_ptr()
    }
}

impl Drop for ResponseArray {
    fn drop(&mut self) {
        for index in 0..self.count {
            // SAFETY: index is within the allocated response array.
            let response = unsafe { (*self.pointer.as_ptr().add(index)).resp };
            let length = self.lengths[index];
            if !response.is_null() {
                // SAFETY: this object allocated each response and still owns it.
                unsafe {
                    (self.allocator.zero)(response.cast(), length);
                    (self.allocator.free)(response.cast());
                }
            }
        }
        // SAFETY: this object owns the checked calloc allocation.
        unsafe {
            (self.allocator.zero)(
                self.pointer.as_ptr().cast(),
                self.count * mem::size_of::<pam_response>(),
            );
            (self.allocator.free)(self.pointer.as_ptr().cast());
        }
    }
}

pub(super) unsafe extern "C" fn converse<C: Conversation>(
    num_msg: c_int,
    messages: *mut *const pam_message,
    output: *mut *mut pam_response,
    appdata: *mut c_void,
) -> c_int {
    match catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `converse_inner` validates every outer pointer and bound
        // before constructing references. PAM modules remain trusted C code.
        unsafe { converse_inner::<C>(num_msg, messages, output, appdata) }
    })) {
        Ok(Ok(())) => pam_sys2::PAM_SUCCESS,
        Ok(Err(CallbackError::Buffer)) => pam_sys2::PAM_BUF_ERR,
        Ok(Err(CallbackError::Conversation)) | Err(_) => pam_sys2::PAM_CONV_ERR,
    }
}

unsafe fn converse_inner<C: Conversation>(
    num_msg: c_int,
    messages: *mut *const pam_message,
    output: *mut *mut pam_response,
    appdata: *mut c_void,
) -> Result<(), CallbackError> {
    if output.is_null() {
        return Err(CallbackError::Conversation);
    }
    // SAFETY: output was checked and is writable under the PAM callback ABI.
    unsafe { *output = ptr::null_mut() };
    let count = usize::try_from(num_msg).map_err(|_| CallbackError::Conversation)?;
    if !(1..=MAX_MESSAGES).contains(&count) || messages.is_null() || appdata.is_null() {
        return Err(CallbackError::Conversation);
    }

    // SAFETY: outer pointers and count have been validated. Individual message
    // pointers are checked before dereference below.
    let state = unsafe { &mut *appdata.cast::<PamCallback<C>>() };
    let message_pointers = unsafe { slice::from_raw_parts(messages.cast_const(), count) };
    let mut responses = ResponseArray::new(count, state.allocator)?;

    for (index, message) in message_pointers.iter().copied().enumerate() {
        // SAFETY: null is rejected; configured PAM modules own the pointee.
        let message = unsafe { message.as_ref() }.ok_or(CallbackError::Conversation)?;
        let request = unsafe { parse_request(message) }?;
        let kind = request.kind();
        let reply = state
            .conversation
            .respond(request)
            .map_err(|_| CallbackError::Conversation)?;
        if !reply.matches(kind) {
            return Err(CallbackError::Conversation);
        }
        match reply {
            Reply::Text(value) => value.expose(|bytes| responses.put_text(index, bytes))?,
            Reply::Secret(value) => value.expose(|bytes| responses.put_text(index, bytes))?,
            Reply::Acknowledged => {}
            Reply::Radio(value) => responses.put_text(index, if value { b"yes" } else { b"no" })?,
            Reply::Binary(value) => {
                value.expose(|bytes| responses.put_binary(index, value.kind(), bytes))?
            }
        }
    }

    // SAFETY: output remains the validated PAM-provided destination. Ownership
    // of the C allocations transfers to Linux-PAM on this success path.
    unsafe { *output = responses.into_raw() };
    Ok(())
}

unsafe fn parse_request(message: &pam_message) -> Result<Request<'_>, CallbackError> {
    match message.msg_style {
        pam_sys2::PAM_PROMPT_ECHO_ON => Ok(Request::EchoOn(unsafe { bounded_text(message.msg) }?)),
        pam_sys2::PAM_PROMPT_ECHO_OFF => {
            Ok(Request::EchoOff(unsafe { bounded_text(message.msg) }?))
        }
        pam_sys2::PAM_TEXT_INFO => Ok(Request::Info(unsafe { bounded_text(message.msg) }?)),
        pam_sys2::PAM_ERROR_MSG => Ok(Request::Error(unsafe { bounded_text(message.msg) }?)),
        pam_sys2::PAM_RADIO_TYPE => Ok(Request::Radio(unsafe { bounded_text(message.msg) }?)),
        pam_sys2::PAM_BINARY_PROMPT => {
            let (kind, data) = unsafe { bounded_binary(message.msg.cast()) }?;
            Ok(Request::Binary { kind, data })
        }
        _ => Err(CallbackError::Conversation),
    }
}

unsafe fn bounded_text<'a>(pointer: *const c_char) -> Result<&'a std::ffi::CStr, CallbackError> {
    if pointer.is_null() {
        return Err(CallbackError::Conversation);
    }
    // SAFETY: configured PAM modules are trusted to supply an allocation; the
    // bounded scan prevents unbounded reads when termination is malformed.
    let length = unsafe { libc::strnlen(pointer, MAX_MESSAGE_BYTES + 1) };
    if length > MAX_MESSAGE_BYTES {
        return Err(CallbackError::Conversation);
    }
    let bytes = unsafe { slice::from_raw_parts(pointer.cast(), length + 1) };
    std::ffi::CStr::from_bytes_with_nul(bytes).map_err(|_| CallbackError::Conversation)
}

unsafe fn bounded_binary<'a>(pointer: *const u8) -> Result<(u8, &'a [u8]), CallbackError> {
    if pointer.is_null() {
        return Err(CallbackError::Conversation);
    }
    // C exposes no allocation extent. The configured PAM module is inside the
    // trust boundary; its declared length is bounded before constructing data.
    let length = u32::from_be(unsafe { ptr::read_unaligned(pointer.cast::<u32>()) }) as usize;
    if !(5..=MAX_MESSAGE_BYTES).contains(&length) {
        return Err(CallbackError::Conversation);
    }
    let kind = unsafe { *pointer.add(4) };
    let data = unsafe { slice::from_raw_parts(pointer.add(5), length - 5) };
    Ok((kind, data))
}

#[derive(Clone, Copy)]
enum CallbackError {
    Buffer,
    Conversation,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::pam_conversation::{BinaryResponse, ConversationError, TextResponse};
    use crate::SecretInput;

    struct CompleteConversation;

    impl Conversation for CompleteConversation {
        fn respond(&mut self, request: Request<'_>) -> Result<Reply, ConversationError> {
            Ok(match request {
                Request::EchoOn(_) => Reply::Text(TextResponse::new("jacob").unwrap()),
                Request::EchoOff(_) => {
                    let mut input = SecretInput::new();
                    input.push('s').unwrap();
                    Reply::Secret(input.finish())
                }
                Request::Info(_) | Request::Error(_) => Reply::Acknowledged,
                Request::Radio(_) => Reply::Radio(true),
                Request::Binary { kind, data } => {
                    Reply::Binary(BinaryResponse::new(kind, data).unwrap())
                }
            })
        }
    }

    #[test]
    fn abi_limits_match_the_reviewed_linux_pam_contract() {
        assert_eq!(MAX_MESSAGES, 32);
        assert_eq!(MAX_MESSAGE_BYTES, 512);
        assert_eq!(crate::MAX_SECRET_BYTES, 512);
    }

    #[test]
    fn callback_rejects_nulls_and_invalid_counts() {
        let mut state = PamCallback::new(CompleteConversation);
        let mut output = ptr::null_mut();
        for count in [-1, 0, 33] {
            let result = unsafe {
                converse::<CompleteConversation>(
                    count,
                    ptr::null_mut(),
                    &mut output,
                    state.appdata(),
                )
            };
            assert_eq!(result, pam_sys2::PAM_CONV_ERR);
            assert!(output.is_null());
        }
    }

    struct PanickingConversation;

    impl Conversation for PanickingConversation {
        fn respond(&mut self, _request: Request<'_>) -> Result<Reply, ConversationError> {
            panic!("deliberate callback panic")
        }
    }

    #[test]
    fn callback_contains_a_rust_panic() {
        let prompt = c"Password:";
        let message = pam_message {
            msg_style: pam_sys2::PAM_PROMPT_ECHO_OFF,
            msg: prompt.as_ptr(),
        };
        let mut message_pointer = &message as *const pam_message;
        let mut output = ptr::null_mut();
        let mut state = PamCallback::new(PanickingConversation);
        let result = unsafe {
            converse::<PanickingConversation>(1, &mut message_pointer, &mut output, state.appdata())
        };
        assert_eq!(result, pam_sys2::PAM_CONV_ERR);
        assert!(output.is_null());
    }

    unsafe extern "C" fn failing_calloc(_count: size_t, _size: size_t) -> *mut c_void {
        ptr::null_mut()
    }

    #[test]
    fn callback_reports_response_array_allocation_failure() {
        let prompt = c"Password:";
        let message = pam_message {
            msg_style: pam_sys2::PAM_PROMPT_ECHO_OFF,
            msg: prompt.as_ptr(),
        };
        let mut message_pointer = &message as *const pam_message;
        let mut output = ptr::null_mut();
        let mut state = PamCallback {
            conversation: CompleteConversation,
            allocator: Allocator {
                calloc: failing_calloc,
                ..Allocator::SYSTEM
            },
        };
        let result = unsafe {
            converse::<CompleteConversation>(1, &mut message_pointer, &mut output, state.appdata())
        };
        assert_eq!(result, pam_sys2::PAM_BUF_ERR);
        assert!(output.is_null());
    }

    static ALLOCATION_CALLS: AtomicUsize = AtomicUsize::new(0);
    static ZERO_CALLS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn fail_third_calloc(count: size_t, size: size_t) -> *mut c_void {
        if ALLOCATION_CALLS.fetch_add(1, Ordering::SeqCst) == 2 {
            ptr::null_mut()
        } else {
            unsafe { libc::calloc(count, size) }
        }
    }

    unsafe extern "C" fn tracking_zero(pointer: *mut c_void, size: size_t) {
        ZERO_CALLS.fetch_add(1, Ordering::SeqCst);
        unsafe { libc::explicit_bzero(pointer, size) };
    }

    #[test]
    fn partial_batch_is_zeroed_when_a_later_allocation_fails() {
        ALLOCATION_CALLS.store(0, Ordering::SeqCst);
        ZERO_CALLS.store(0, Ordering::SeqCst);
        let first = pam_message {
            msg_style: pam_sys2::PAM_PROMPT_ECHO_ON,
            msg: c"User:".as_ptr(),
        };
        let second = pam_message {
            msg_style: pam_sys2::PAM_PROMPT_ECHO_ON,
            msg: c"Again:".as_ptr(),
        };
        let mut messages = [&first as *const pam_message, &second as *const pam_message];
        let mut output = ptr::null_mut();
        let mut state = PamCallback {
            conversation: CompleteConversation,
            allocator: Allocator {
                calloc: fail_third_calloc,
                zero: tracking_zero,
                ..Allocator::SYSTEM
            },
        };
        let result = unsafe {
            converse::<CompleteConversation>(2, messages.as_mut_ptr(), &mut output, state.appdata())
        };
        assert_eq!(result, pam_sys2::PAM_BUF_ERR);
        assert!(output.is_null());
        // The first text allocation and the response array are both wiped.
        assert_eq!(ZERO_CALLS.load(Ordering::SeqCst), 2);
    }
}
