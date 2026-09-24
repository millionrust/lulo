//! Private, crash-safe Notification Center history and per-app policy.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use rmac_notifications::{
    Action, ActionTarget, AppId, BannerPolicy, Content, Delivery, DeliveryPolicy, DisplayHints,
    HistoryPolicy, Indicator, LockScreenVisibility, Notification, NotificationId, Priority, Sound,
    Source, Time,
};
use serde::{Deserialize, Serialize};

mod label;
mod model;
mod serialization;
mod store;
#[cfg(test)]
mod tests;

use label::StoredLabel;
pub use label::*;
pub use model::*;
use serialization::*;
pub use store::*;

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_HISTORY: usize = 500;
const MAX_PER_APP: usize = 100;
const MAX_POLICIES: usize = 512;
const MAX_LOCK_PREVIEWS: usize = 16;
