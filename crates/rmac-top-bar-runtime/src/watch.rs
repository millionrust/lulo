use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use chrono::{DateTime, FixedOffset, Local};
use futures_util::FutureExt as _;

use crate::Coordinator;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
}

impl Error {
    fn new(operation: &'static str) -> Self {
        Self { operation }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}", self.operation)
    }
}

impl std::error::Error for Error {}

enum Input {
    Shell(Box<rmac_shell_runtime::Update>),
    Locale(rmac_locale::WatchEvent),
    Time,
    ClockDeadline,
    OutputClosed,
}

/// Publish renderer-visible top-bar projections until the receiver closes.
///
/// The runtime waits for shell, locale, and clock subscriptions before its
/// first publication. Afterwards it wakes only for authority events or the
/// exact next visible clock boundary.
pub async fn watch(sender: Sender<rmac_top_bar::Update>) -> Result<(), Error> {
    let (shell_tx, shell_rx) = async_channel::bounded(8);
    let (locale_tx, locale_rx) = async_channel::bounded(4);
    let (time_tx, time_rx) = async_channel::bounded(4);

    let shell = async {
        rmac_shell_runtime::watch(shell_tx)
            .await
            .map_err(|_| Error::new("watch shell status"))
    };
    let locale = async {
        rmac_locale_linux::watch(locale_tx)
            .await
            .map_err(|_| Error::new("watch system locale"))
    };
    let time = async {
        rmac_time_linux::watch(time_tx)
            .await
            .map_err(|_| Error::new("watch system clock"))
    };
    let consumer = consume(sender, shell_rx, locale_rx, time_rx);
    let (_, _, _, _) = futures_util::try_join!(shell, locale, time, consumer)?;
    Ok(())
}

async fn consume(
    sender: Sender<rmac_top_bar::Update>,
    shell: Receiver<rmac_shell_runtime::Update>,
    locale: Receiver<rmac_locale::WatchEvent>,
    time: Receiver<rmac_time::WatchEvent>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut deadline = None;
    loop {
        let input = next_input(&sender, &shell, &locale, &time, deadline).await?;
        let now = Local::now().fixed_offset();
        let update = match input {
            Input::Shell(update) => coordinator.apply_shell(*update, now),
            Input::Locale(rmac_locale::WatchEvent::Changed) => {
                let hour_cycle = blocking::unblock(rmac_locale_linux::hour_cycle)
                    .await
                    .map_err(|_| ());
                coordinator.apply_locale(hour_cycle, now)
            }
            Input::Locale(rmac_locale::WatchEvent::Unavailable) => {
                coordinator.apply_locale(Err(()), now)
            }
            Input::Time => coordinator.apply_time_signal(now),
            Input::ClockDeadline => coordinator.clock_deadline(now),
            Input::OutputClosed => return Ok(()),
        };
        if let Some(update) = update {
            deadline = Some(realtime_deadline(now, update.next_clock_update)?);
            if update.redraw && sender.send(update).await.is_err() {
                return Ok(());
            }
        }
    }
}

async fn next_input(
    sender: &Sender<rmac_top_bar::Update>,
    shell: &Receiver<rmac_shell_runtime::Update>,
    locale: &Receiver<rmac_locale::WatchEvent>,
    time: &Receiver<rmac_time::WatchEvent>,
    deadline: Option<SystemTime>,
) -> Result<Input, Error> {
    let shell = shell.recv().fuse();
    let locale = locale.recv().fuse();
    let time = time.recv().fuse();
    let closed = sender.closed().fuse();
    let timer = async move {
        match deadline {
            Some(deadline) => wait_for_deadline(deadline).await,
            None => futures_util::future::pending::<()>().await,
        }
    }
    .fuse();
    futures_util::pin_mut!(shell, locale, time, closed, timer);
    futures_util::select! {
        update = shell => update.map(|update| Input::Shell(Box::new(update))).map_err(|_| Error::new("receive shell status")),
        event = locale => event.map(Input::Locale).map_err(|_| Error::new("receive locale state")),
        event = time => event.map(|_| Input::Time).map_err(|_| Error::new("receive clock state")),
        _ = timer => Ok(Input::ClockDeadline),
        _ = closed => Ok(Input::OutputClosed),
    }
}

pub(crate) fn realtime_deadline(
    now: DateTime<FixedOffset>,
    delay: Duration,
) -> Result<SystemTime, Error> {
    let epoch_millis = u64::try_from(now.timestamp_millis())
        .map_err(|_| Error::new("calculate the top bar clock deadline"))?;
    UNIX_EPOCH
        .checked_add(Duration::from_millis(epoch_millis))
        .and_then(|now| now.checked_add(delay))
        .ok_or_else(|| Error::new("calculate the top bar clock deadline"))
}

#[cfg(target_os = "linux")]
async fn wait_for_deadline(deadline: SystemTime) {
    if rmac_time_linux::wait_until_realtime(deadline)
        .await
        .is_err()
    {
        // Retain a bounded, non-spinning fallback if timerfd is unavailable.
        // The live clock-change watcher still replaces this sleep when it can.
        wait_with_monotonic_fallback(deadline).await;
    }
}

#[cfg(not(target_os = "linux"))]
async fn wait_for_deadline(deadline: SystemTime) {
    wait_with_monotonic_fallback(deadline).await;
}

async fn wait_with_monotonic_fallback(deadline: SystemTime) {
    let delay = deadline
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::ZERO);
    async_io::Timer::after(delay).await;
}
