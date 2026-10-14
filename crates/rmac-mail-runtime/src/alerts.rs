//! Mail ▸ Settings ▸ General's new-message choices, which the app sets as
//! they change and the event sinks read when new mail arrives.

use std::sync::atomic::{AtomicBool, Ordering};

static NOTIFY: AtomicBool = AtomicBool::new(true);
static SOUND: AtomicBool = AtomicBool::new(true);

/// "New message notifications" (Inbox Only, or None) and "New messages
/// sound" (New Mail, or None).
pub fn set(notify: bool, sound: bool) {
    NOTIFY.store(notify, Ordering::Relaxed);
    SOUND.store(sound, Ordering::Relaxed);
}

/// Whether a new message posts a notification.
pub fn notify() -> bool {
    NOTIFY.load(Ordering::Relaxed)
}

/// Whether that notification plays the new-mail sound.
pub fn sound() -> bool {
    SOUND.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn choices_round_trip() {
        super::set(false, true);
        assert!(!super::notify());
        assert!(super::sound());
        super::set(true, false);
        assert!(super::notify());
        assert!(!super::sound());
        super::set(true, true);
    }
}
