//! Window pictures on Windows for Mission Control (ADR 0023, "Mission
//! Control on Windows"): every window draws itself (`PrintWindow`, in
//! `rmac-compositor-system`), so a window another one covers still gets its
//! real picture (a screen crop cannot give it, which is why Lulo OS keeps
//! the last picture of a covered window, MC-01). The pictures are taken in
//! parallel off the UI thread, once per open; a window that does not
//! answer in time keeps its icon card.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::Captured;
use crate::model::Scene;

/// How long an open waits for the pictures before it flies in with what
/// it has (a hung app's window keeps its icon card).
const DEADLINE: Duration = Duration::from_millis(400);

/// Picture every open window of the scene. Windows has no picture of a
/// whole Space, so the Spaces bar shows its window plates.
pub fn capture_scene(scene: &Scene) -> Captured {
    let (sender, pictures) = mpsc::channel();
    let mut asked = 0usize;
    for window in scene.windows.iter().filter(|window| !window.minimised) {
        let id = window.id;
        let sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("mission-control-picture".into())
            .spawn(move || {
                let _ = sender.send((id, rmac_compositor_system::window_picture(id)));
            });
        if spawned.is_ok() {
            asked += 1;
        }
    }
    drop(sender);
    let deadline = Instant::now() + DEADLINE;
    let mut windows = Vec::with_capacity(asked);
    for _ in 0..asked {
        let left = deadline.saturating_duration_since(Instant::now());
        match pictures.recv_timeout(left) {
            Ok((id, Some(pixels))) => windows.push((id, pixels)),
            Ok((_, None)) => {}
            Err(_) => break,
        }
    }
    Captured {
        desktop: None,
        windows,
    }
}
