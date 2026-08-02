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

mod model;
mod runtime;
#[cfg(test)]
mod tests;
mod translate;
mod transport;
mod wire;

pub use model::*;
pub use runtime::*;
use translate::*;
use transport::*;

pub const SOCKET_PATH_ENV: &str = "NIRI_SOCKET";
const MAX_INITIAL_EVENTS: usize = 4096;
