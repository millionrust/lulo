//! Direct niri IPC adapter for [`rmac_compositor`].

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_channel::Sender;
use async_io::{Async, Timer};
use futures_util::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use rmac_compositor as domain;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wire::{Event as NiriEvent, Reply, Request, Response};

mod minimize;
mod model;
mod runtime;
#[cfg(test)]
mod tests;
mod translate;
mod transport;
// `pub` so the niri_ipc fuzz target (fuzz/fuzz_targets/niri_ipc.rs) can
// deserialize into the real wire types.
pub mod wire;

pub use minimize::*;
pub use model::*;
pub use runtime::*;
use translate::*;
use transport::*;

pub const SOCKET_PATH_ENV: &str = "NIRI_SOCKET";
const MAX_INITIAL_EVENTS: usize = 4096;

/// niri 26.04 keeps at least this much of a floating window inside the
/// working area, in logical pixels, however far it is moved (measured on
/// the reference laptop: 75.2 at scale 1.25, snapped to physical pixels).
pub const FLOATING_MIN_VISIBLE: f64 = 75.0;
