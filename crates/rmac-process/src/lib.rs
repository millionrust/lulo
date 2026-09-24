//! Helper processes that live and die with the process that started them.
//!
//! A monitor such as `gsettings monitor`, `wl-paste --watch` or
//! `pw-dump --monitor` blocks until something changes, which may be never.
//! When its parent exits, nothing reaps it and nothing tells it to stop. An
//! app's cgroup scope then stays alive, and every launch leaves another one
//! behind (audit finding SES-01: two `gsettings monitor`s per System Settings
//! launch). Such a child also inherits every descriptor its parent was
//! started with and did not mark close-on-exec, such as a lock file a
//! script held open around the launch.
//!
//! [`bind_to_parent`] prepares a [`std::process::Command`] so that the child:
//! - gets `SIGTERM` from the kernel when the parent thread dies
//!   (`PR_SET_PDEATHSIG`), however the parent ends, even on `SIGKILL`;
//! - exits at once if the parent was already gone before the request
//!   took effect;
//! - keeps only its standard streams, since every other inherited
//!   descriptor is marked close-on-exec.
//!
//! [`OwnedChild`] kills and reaps the child when it is dropped, so an early
//! return cannot leave it running either.
//!
//! Only use this for helpers that serve the parent. A launched app, `orca`,
//! `wl-copy`'s selection server or the locker must outlive whoever started
//! them, and must not be bound.
//!
//! `PR_SET_PDEATHSIG` fires when the *thread* that forked the child exits.
//! Spawn bound children from a thread that lives as long as the child is
//! wanted (a dedicated watcher thread, or the call that waits for it), not
//! from a pooled thread that may retire while the child still runs.

use std::io;
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command};

/// Makes `command`'s child end with its parent and keep only its standard
/// streams. A no-op off Linux, where there is no `PR_SET_PDEATHSIG`.
pub fn bind_to_parent(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt as _;

        // SAFETY: `getpid` is async-signal-safe and has no preconditions.
        let parent = unsafe { libc::getpid() };
        // SAFETY: the closure runs in the forked child before `exec` and only
        // makes async-signal-safe system calls (`prctl`, `getppid`,
        // `close_range`, `fcntl`, `_exit`); it allocates nothing and takes
        // no locks.
        unsafe {
            command.pre_exec(move || linux::in_child(parent));
        }
    }
    command
}

/// Spawns `command` bound to this process (see [`bind_to_parent`]).
pub fn spawn_bound(command: &mut Command) -> io::Result<OwnedChild> {
    bind_to_parent(command).spawn().map(OwnedChild::new)
}

/// A child that is killed and reaped when this value is dropped.
#[derive(Debug)]
pub struct OwnedChild {
    child: Option<Child>,
}

impl OwnedChild {
    pub fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    /// Stop owning the child: it is neither killed nor reaped on drop.
    pub fn release(mut self) -> Child {
        self.child
            .take()
            .expect("an owned child is present until drop")
    }
}

impl Deref for OwnedChild {
    type Target = Child;

    fn deref(&self) -> &Child {
        self.child
            .as_ref()
            .expect("an owned child is present until drop")
    }
}

impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("an owned child is present until drop")
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        match child.try_wait() {
            // Already exited: `try_wait` reaped it.
            Ok(Some(_)) => {}
            _ => {
                // `kill` fails only when the child has already exited, and
                // then `wait` reaps it; either way nothing is left behind.
                if let Err(error) = child.kill() {
                    if error.kind() != io::ErrorKind::InvalidInput {
                        eprintln!("could not stop helper process {}: {error}", child.id());
                    }
                }
                if let Err(error) = child.wait() {
                    eprintln!("could not reap helper process {}: {error}", child.id());
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;

    const FIRST_NON_STANDARD_FD: u32 = 3;
    /// `CLOSE_RANGE_CLOEXEC` from `<linux/close_range.h>` (Linux 5.11).
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    /// Walked one by one only on kernels without `close_range`.
    const FALLBACK_FD_LIMIT: libc::c_int = 1024;

    pub(super) fn in_child(parent: libc::pid_t) -> io::Result<()> {
        // SAFETY: plain system calls on the child's own state; see the
        // caller for why they are safe between fork and exec.
        unsafe {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl, in which case
            // the signal will never come.
            if libc::getppid() != parent {
                libc::_exit(0);
            }
            // Mark rather than close: std's own exec-status pipe must stay
            // open until exec, and it is already close-on-exec.
            let marked = libc::syscall(
                libc::SYS_close_range,
                FIRST_NON_STANDARD_FD,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            );
            if marked != 0 {
                for fd in FIRST_NON_STANDARD_FD as libc::c_int..FALLBACK_FD_LIMIT {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                        libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::os::fd::AsRawFd as _;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    fn alive(pid: u32) -> bool {
        // A zombie still has a /proc entry; its state letter is Z.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(") ")
                    .and_then(|(_, rest)| rest.chars().next())
            })
            .is_some_and(|state| state != 'Z' && state != 'X')
    }

    fn wait_until(deadline: Duration, condition: impl Fn() -> bool) -> bool {
        let end = Instant::now() + deadline;
        while Instant::now() < end {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        condition()
    }

    #[test]
    fn a_bound_child_dies_with_the_thread_that_started_it() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let starter = std::thread::spawn(move || {
            let child = bind_to_parent(Command::new("sleep").arg("30"))
                .stdin(Stdio::null())
                .spawn()
                .expect("spawn sleep");
            sender.send(child.id()).unwrap();
            // Leak the handle: only the kernel's parent-death signal can
            // stop the child now.
            std::mem::forget(child);
        });
        let pid = receiver.recv().unwrap();
        starter.join().unwrap();
        assert!(
            wait_until(Duration::from_secs(5), || !alive(pid)),
            "the child outlived the thread that started it"
        );
        // Reap the zombie so the test leaves nothing behind.
        // SAFETY: waiting for our own child by pid.
        unsafe {
            libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), 0);
        }
    }

    #[test]
    fn dropping_an_owned_child_kills_and_reaps_it() {
        let child =
            spawn_bound(Command::new("sleep").arg("30").stdin(Stdio::null())).expect("spawn sleep");
        let pid = child.id();
        assert!(alive(pid));
        drop(child);
        // Reaped: the pid no longer names our child at all.
        // SAFETY: probing our own former child by pid.
        let result =
            unsafe { libc::waitpid(pid as libc::pid_t, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(result, -1);
    }

    #[test]
    fn a_released_child_is_left_alone() {
        let child = spawn_bound(Command::new("true").stdin(Stdio::null())).expect("spawn true");
        let mut child = child.release();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_bound_child_keeps_only_its_standard_streams() {
        // A descriptor this process holds without close-on-exec, like the
        // lock file a script holds open around a launch.
        let file = tempfile_without_cloexec();
        let fd = file.as_raw_fd();
        let mut child = spawn_bound(
            Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "test -e /proc/self/fd/{fd} && echo leaked || echo closed"
                ))
                .stdout(Stdio::piped()),
        )
        .expect("spawn sh");
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output.trim(), "closed");

        // Unbound, the same descriptor does leak, so the test is meaningful.
        let unbound = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "test -e /proc/self/fd/{fd} && echo leaked || echo closed"
            ))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&unbound.stdout).trim(), "leaked");
    }

    fn tempfile_without_cloexec() -> std::fs::File {
        let path = std::env::temp_dir().join(format!("rmac-process-test-{}", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let fd = file.as_raw_fd();
        // SAFETY: clearing a flag on a descriptor this test owns.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
        file
    }
}
