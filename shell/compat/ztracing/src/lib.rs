//! License-compatible tracing surface for the standalone rmac shell graph.
//!
//! GPUI's `sum_tree` dependency only needs tracing's public instrumentation
//! macros. Re-exporting them here keeps the shell on the maintained GPUI API
//! without linking the GPL-only Zed tracing runtime into MIT-licensed rmac.

pub use tracing::{debug_span, error_span, info_span, instrument, span, trace_span, warn_span};

/// The shell does not install Zed's process-wide tracing subscriber.
pub fn init() {}
