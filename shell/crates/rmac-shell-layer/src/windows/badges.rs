//! Dock badges and progress apps publish in the Lulo layer (Windows): the
//! Notification Centre's pipe server hears them (`rmac_notifications::pipe`'s
//! `badge` line) and the Dock, in the same process, draws them, as Lulo OS's
//! Dock draws the LauncherEntry signal.

use async_channel::{Receiver, Sender};

/// One app's badge: its app id, count (none at 0) and progress bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub app_id: String,
    pub count: i64,
    pub progress: Option<f64>,
}

static OUTBOX: std::sync::Mutex<Vec<Sender<Badge>>> = std::sync::Mutex::new(Vec::new());

/// A channel that gets every badge an app publishes.
pub fn subscribe() -> Receiver<Badge> {
    let (sender, receiver) = async_channel::bounded(32);
    if let Ok(mut outbox) = OUTBOX.lock() {
        outbox.push(sender);
    }
    receiver
}

/// Publish `badge` (from any thread).
pub fn publish(badge: Badge) {
    if let Ok(mut outbox) = OUTBOX.lock() {
        outbox.retain(|subscriber| !subscriber.is_closed());
        for subscriber in outbox.iter() {
            let _ = subscriber.try_send(badge.clone());
        }
    }
}
