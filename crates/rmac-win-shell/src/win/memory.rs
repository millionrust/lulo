//! The Lulo layer's memory on 8 GB PCs: what it holds at each step (for
//! the start-up trace and the CI memory gate), and giving back what setup
//! touched once and will not touch again.
//!
//! Start-up touches far more than an idle bar and Dock keep using: the
//! Direct3D driver's code and tables, DirectWrite's font scan, the shell
//! libraries behind the first icons, the first frames' scratch heap. Once
//! the layer is up and idle, [`trim`] compacts the heap and empties the
//! working set; the pages an idle shell does use come straight back, and
//! the rest wait on the standby list, where Windows can use the memory.

use windows::Win32::System::Memory::{
    GetProcessHeap, HeapCompact, SetProcessWorkingSetSizeEx, HEAP_FLAGS,
    SETPROCESSWORKINGSETSIZEEX_FLAGS,
};
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use super::trace;

/// (working set, private bytes) of this process, in bytes.
pub fn usage() -> Option<(usize, usize)> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: the EX structure is passed with its own size, as the call
    // allows; this process's pseudo-handle needs no closing.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    }
    .as_bool();
    ok.then_some((counters.WorkingSetSize, counters.PrivateUsage))
}

/// Trace this process's memory at `phase`.
pub fn report(phase: &str) {
    if let Some((working_set, private)) = usage() {
        trace(|| {
            format!(
                "memory {phase}: working set {:.1} MB, private {:.1} MB",
                working_set as f64 / 1_048_576.0,
                private as f64 / 1_048_576.0
            )
        });
    }
}

/// Compact the heap and empty the working set (see the module notes).
pub fn trim(phase: &str) {
    report(&format!("before {phase}"));
    // SAFETY: the process heap and this process's pseudo-handle; (-1, -1)
    // is the documented request to empty the working set.
    unsafe {
        if let Ok(heap) = GetProcessHeap() {
            HeapCompact(heap, HEAP_FLAGS(0));
        }
        let _ = SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        );
    }
    report(phase);
}
