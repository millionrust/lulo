//! Test-only allocator that records each thread's peak live heap bytes, so
//! tests can bound the memory a large-document operation needs.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct PerThreadPeak;

#[global_allocator]
static ALLOCATOR: PerThreadPeak = PerThreadPeak;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn track(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get() + delta;
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

// SAFETY: every call forwards to the system allocator unchanged; the
// bookkeeping only touches const-initialized thread-locals, which never
// allocate.
unsafe impl GlobalAlloc for PerThreadPeak {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            track(layout.size() as isize);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        track(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            track(new_size as isize - layout.size() as isize);
        }
        moved
    }
}

/// Run `work` and return its result with the most heap it held at once
/// on this thread, beyond what was live when it started.
pub(crate) fn peak_heap_during<R>(work: impl FnOnce() -> R) -> (R, usize) {
    let base = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(base));
    let result = work();
    let peak = PEAK.with(Cell::get);
    (result, (peak - base).max(0) as usize)
}
