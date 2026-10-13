//! When the layer has finished starting: the Windows apps read from the
//! Apps folder, the icons its helper processes read, and start-up's own
//! memory trims done. Each piece of that work holds a [`Busy`]; when the
//! last one ends the shell traces `settled` (`LULO_SHELL_TRACE=1`), and
//! the Windows CI's idle gate measures from that mark instead of after a
//! fixed delay (`scripts/windows/shell_smoke.py`). Nothing waits or polls:
//! it is a counter.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Work under way; start-up itself counts as one until [`started`].
static BUSY: AtomicUsize = AtomicUsize::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Work that keeps the layer from being settled while it lives.
pub struct Busy(());

/// Mark work as under way until the returned guard is dropped.
pub fn busy() -> Busy {
    BUSY.fetch_add(1, Ordering::AcqRel);
    Busy(())
}

impl Drop for Busy {
    fn drop(&mut self) {
        done();
    }
}

/// Start-up's own work is done (the second memory trim). Only the first
/// call counts.
pub fn started() {
    if !STARTED.swap(true, Ordering::AcqRel) {
        done();
    }
}

fn done() {
    if BUSY.fetch_sub(1, Ordering::AcqRel) == 1 {
        super::trace(|| "settled".into());
    }
}
