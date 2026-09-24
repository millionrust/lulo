//! SIGTERM, SIGHUP and SIGINT delivered to the app as a message.
//!
//! A signal handler may do almost nothing, so it only writes the signal's
//! number into a pipe. A thread blocked reading that pipe (idle until a
//! signal arrives: no polling) forwards it to the main thread, which quits
//! the app through GPUI so every quit hook runs, the way a Mac app gets
//! `applicationWillTerminate` before the session ends. Each handler is
//! installed with `SA_RESETHAND`, so a second signal of the same kind ends
//! the process at once, and if the app has not finished quitting after
//! `forced_exit_after` the thread ends it as the signal would have.

use std::io::Read as _;
use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

/// The signals that end a session or an app from outside: systemd and
/// logind send SIGTERM, a closing terminal SIGHUP, Control-C SIGINT.
pub(crate) const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGHUP, libc::SIGINT];

/// The pipe's write end, or -1 before [`listen`] succeeds.
static WRITE_END: AtomicI32 = AtomicI32::new(-1);

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the caller only reads or writes this thread's errno.
    unsafe { libc::__errno_location() }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the caller only reads or writes this thread's errno.
    unsafe { libc::__error() }
}

extern "C" fn record_signal(signal: libc::c_int) {
    let fd = WRITE_END.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    let byte = u8::try_from(signal).unwrap_or(u8::MAX);
    // SAFETY: write(2) is async-signal-safe and the pipe's write end stays
    // open for the life of the process. errno is restored so the
    // interrupted code never sees this write's result.
    unsafe {
        let errno = errno_location();
        let saved = *errno;
        libc::write(fd, (&byte as *const u8).cast(), 1);
        *errno = saved;
    }
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn add_descriptor_flag(fd: libc::c_int, get: libc::c_int, set: libc::c_int, flag: libc::c_int) {
    // SAFETY: fcntl on a descriptor this module owns.
    unsafe {
        let flags = libc::fcntl(fd, get);
        if flags >= 0 {
            libc::fcntl(fd, set, flags | flag);
        }
    }
}

/// Start listening for [`SIGNALS`]. The receiver yields the first one to
/// arrive. A signal the process inherited as ignored (as under `nohup`)
/// stays ignored. Fails if the process already listens.
pub(crate) fn listen(
    forced_exit_after: Option<Duration>,
) -> std::io::Result<async_channel::Receiver<libc::c_int>> {
    let mut fds = [-1 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe(2) returns.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(last_error());
    }
    // SAFETY: pipe(2) just returned these two descriptors, owned by nothing
    // else.
    let (read_end, write_end) =
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [read_end.as_raw_fd(), write_end.as_raw_fd()] {
        add_descriptor_flag(fd, libc::F_GETFD, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    // A full pipe must never block the handler.
    add_descriptor_flag(
        write_end.as_raw_fd(),
        libc::F_GETFL,
        libc::F_SETFL,
        libc::O_NONBLOCK,
    );
    if WRITE_END
        .compare_exchange(
            -1,
            write_end.as_raw_fd(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_err()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "termination signals are already handled",
        ));
    }
    // The handler writes here until the process ends.
    let _ = write_end.into_raw_fd();

    for signal in SIGNALS {
        // SAFETY: sigaction(2) reads and writes plain structs owned here.
        unsafe {
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, std::ptr::null(), &mut previous) != 0 {
                return Err(last_error());
            }
            if previous.sa_sigaction == libc::SIG_IGN {
                continue;
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = record_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART | libc::SA_RESETHAND;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(last_error());
            }
        }
    }

    let (sender, receiver) = async_channel::bounded(1);
    let mut reader = std::fs::File::from(read_end);
    std::thread::Builder::new()
        .name("rmac-session-end".into())
        .spawn(move || {
            let mut byte = [0_u8; 1];
            let signal = loop {
                match reader.read(&mut byte) {
                    Ok(1) => break libc::c_int::from(byte[0]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Ok(_) | Err(_) => return,
                }
            };
            if sender.send_blocking(signal).is_err() {
                return;
            }
            let Some(delay) = forced_exit_after else {
                return;
            };
            std::thread::sleep(delay);
            eprintln!("still running {delay:?} after signal {signal}; ending now");
            // SA_RESETHAND restored the default action, so this ends the
            // process exactly as the signal would have.
            // SAFETY: signalling our own process.
            unsafe {
                libc::kill(libc::getpid(), signal);
            }
            std::process::exit(128 + signal);
        })?;
    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_termination_signal_arrives_as_a_message_and_listening_is_once_only() {
        let receiver = listen(None).expect("listen for termination signals");
        // SAFETY: the handler installed above only writes to its pipe.
        assert_eq!(unsafe { libc::raise(libc::SIGHUP) }, 0);
        assert_eq!(receiver.recv_blocking(), Ok(libc::SIGHUP));
        assert_eq!(
            listen(None).map(|_| ()).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }
}
