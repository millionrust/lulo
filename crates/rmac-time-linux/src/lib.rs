//! Date and time adapter: systemd-timedated on Linux, the Win32 time APIs on
//! Windows.

mod api;
mod system;
mod watch;
#[cfg(windows)]
mod win;

pub use api::*;
pub use watch::*;

#[cfg(test)]
mod tests;
