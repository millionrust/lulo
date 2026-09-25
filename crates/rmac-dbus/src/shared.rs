//! The actual cache behind the crate root — see the module docs there.

use std::future::Future;
use std::sync::{Mutex, OnceLock};

static SYSTEM: OnceLock<Mutex<Option<zbus::Connection>>> = OnceLock::new();
static SESSION: OnceLock<Mutex<Option<zbus::Connection>>> = OnceLock::new();

/// Return the cached connection behind `lock`, opening one with `connect`
/// the first time. Two callers racing to open the first connection may
/// both dial the bus; the loser's connection is simply dropped (closing an
/// otherwise-unused socket) once the mutex resolves who won, so the cache
/// still ends up holding exactly one connection.
async fn shared(
    lock: &'static OnceLock<Mutex<Option<zbus::Connection>>>,
    connect: impl Future<Output = zbus::Result<zbus::Connection>>,
) -> zbus::Result<zbus::Connection> {
    let mutex = lock.get_or_init(|| Mutex::new(None));
    let existing = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(connection) = existing {
        return Ok(connection);
    }
    let connection = connect.await?;
    let mut guard = mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(guard.get_or_insert(connection).clone())
}

/// This process's one connection to the system bus (NetworkManager,
/// UPower, logind, bluez, hostnamed, localed, timedated, systemd, udisks2,
/// …), opened once and shared with every other caller in this process.
pub async fn system() -> zbus::Result<zbus::Connection> {
    shared(&SYSTEM, zbus::Connection::system()).await
}

/// This process's one connection to the session bus (desktop portals, and
/// the notification/focus/lock-screen services), opened once and shared.
pub async fn session() -> zbus::Result<zbus::Connection> {
    shared(&SESSION, zbus::Connection::session()).await
}

/// [`system`], for a caller that isn't in an async context.
pub fn system_blocking() -> zbus::Result<zbus::blocking::Connection> {
    async_io::block_on(system()).map(Into::into)
}

/// [`session`], for a caller that isn't in an async context.
pub fn session_blocking() -> zbus::Result<zbus::blocking::Connection> {
    async_io::block_on(session()).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A second call reuses a connection the first one opened, but never
    /// reuses a failure — this runs both with and without a reachable bus
    /// (there may be no system bus in a sandboxed CI run), so it checks
    /// whichever behaviour actually applied rather than assuming one.
    #[test]
    fn a_success_is_cached_but_a_failure_is_retried() {
        async_io::block_on(async {
            let lock: &'static OnceLock<Mutex<Option<zbus::Connection>>> =
                Box::leak(Box::new(OnceLock::new()));
            let attempts = AtomicUsize::new(0);
            let attempt = || {
                attempts.fetch_add(1, Ordering::SeqCst);
                zbus::Connection::system()
            };
            let first = shared(lock, attempt()).await;
            let second = shared(lock, attempt()).await;
            assert_eq!(first.is_ok(), second.is_ok());
            match first {
                // A real bus was reachable: the cache handed the second
                // call the same connection instead of dialing again.
                Ok(_) => assert_eq!(attempts.load(Ordering::SeqCst), 1),
                // No bus was reachable: a failed attempt is never cached,
                // so the second call tried again rather than reusing the
                // error.
                Err(_) => assert_eq!(attempts.load(Ordering::SeqCst), 2),
            }
        });
    }
}
