use rmac_notifications::banner::Schedule;
use rmac_notifications::NotificationId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Redraw,
    CapturePreviousFocus,
    RestorePreviousFocus,
    Expire(NotificationId),
    Dismiss(NotificationId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Update {
    pub commands: Vec<Command>,
    pub schedule: Schedule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidConfig,
    NoOutput,
    UnknownNotification,
    UnknownBanner,
}
