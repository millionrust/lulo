//! Holds logind's delay inhibitor while any Lulo OS app has unsaved work,
//! and uses the delay to keep that work when a shutdown or sleep starts from
//! anywhere but the menu bar (a `systemctl poweroff` in Terminal, the power
//! button, a lid close, a low-battery power-off). See `session_guard` for the
//! policy and `rmac_app_menu::unsaved` for the app side.
//!
//! Everything here waits on D-Bus signals; nothing polls while idle. The
//! short window checks while asking windows to close happen only during a
//! shutdown, as in Log Out.

use std::time::{Duration, Instant};

use futures_util::{FutureExt as _, StreamExt as _};
use gpui::{App, AsyncApp};
use rmac_app_menu::unsaved;

use crate::menu_model::QUIT_ALL_CHECK;
use crate::session_guard::{
    on_prepare, preserve_budget, wants_inhibitor, GuardAction, Prepare, UnsavedApps, INHIBIT_MODE,
    INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY,
};

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;

    #[zbus(property, name = "InhibitDelayMaxUSec")]
    fn inhibit_delay_max_usec(&self) -> zbus::Result<u64>;

    #[zbus(signal)]
    fn prepare_for_shutdown(&self, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

enum Event {
    Owner(String, bool),
    Prepare(Prepare, bool),
}

pub fn start(cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        if let Err(error) = guard(cx).await {
            eprintln!(
                "a shutdown started outside the menu bar will not wait for unsaved documents: {error}"
            );
        }
    })
    .detach();
}

async fn guard(cx: &mut AsyncApp) -> Result<(), String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|error| format!("could not reach the system bus: {error}"))?;
    let manager = LoginManagerProxy::new(&system)
        .await
        .map_err(|error| format!("could not reach logind: {error}"))?;
    let mut shutdowns = manager
        .receive_prepare_for_shutdown()
        .await
        .map_err(|error| format!("could not follow logind shutdowns: {error}"))?;
    let mut sleeps = manager
        .receive_prepare_for_sleep()
        .await
        .map_err(|error| format!("could not follow logind sleeps: {error}"))?;
    // Subscribe before listing, so no owner change falls between the two.
    let mut owners = unsaved::watch_owners()
        .await
        .map_err(|error| error.to_string())?;
    let mut apps = UnsavedApps::default();
    apps.seed(
        unsaved::current_owners()
            .await
            .map_err(|error| error.to_string())?,
    );
    let budget = preserve_budget(
        manager
            .inhibit_delay_max_usec()
            .await
            .ok()
            .map(Duration::from_micros),
    );

    let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
    let mut preparing = false;
    loop {
        let wanted = wants_inhibitor(apps.any(), preparing);
        if wanted && inhibitor.is_none() {
            match manager
                .inhibit(INHIBIT_WHAT, INHIBIT_WHO, INHIBIT_WHY, INHIBIT_MODE)
                .await
            {
                Ok(lock) => inhibitor = Some(lock),
                Err(error) => eprintln!(
                    "could not ask logind to wait for unsaved documents: {error}; \
                     a shutdown from Terminal may lose up to 2 s of typing"
                ),
            }
        } else if !wanted && !preparing {
            // Dropping the descriptor releases the inhibitor. While logind
            // is preparing, the release comes after the drafts are kept.
            inhibitor = None;
        }

        let event = {
            let owner = owners.next().fuse();
            let shutdown = shutdowns.next().fuse();
            let sleep = sleeps.next().fuse();
            futures_util::pin_mut!(owner, shutdown, sleep);
            futures_util::select! {
                owner = owner => match owner {
                    Some((name, owned)) => Event::Owner(name, owned),
                    None => return Err("the session bus closed".into()),
                },
                signal = shutdown => {
                    let signal = signal.ok_or("logind stopped sending shutdown signals")?;
                    let start = *signal
                        .args()
                        .map_err(|error| format!("unreadable PrepareForShutdown: {error}"))?
                        .start();
                    Event::Prepare(Prepare::Shutdown, start)
                },
                signal = sleep => {
                    let signal = signal.ok_or("logind stopped sending sleep signals")?;
                    let start = *signal
                        .args()
                        .map_err(|error| format!("unreadable PrepareForSleep: {error}"))?
                        .start();
                    Event::Prepare(Prepare::Sleep, start)
                },
            }
        };

        match event {
            Event::Owner(name, owned) => apps.update(name, owned),
            Event::Prepare(_, false) => {
                // Awake again, or the shutdown was cancelled.
                preparing = false;
            }
            Event::Prepare(kind, true) => {
                preparing = true;
                match on_prepare(kind, inhibitor.is_some()) {
                    GuardAction::Nothing => {}
                    GuardAction::PreserveThenRelease { close_windows } => {
                        let deadline = Instant::now() + budget;
                        preserve_everywhere(apps.names(), deadline, cx).await;
                        if close_windows {
                            ask_every_window_to_close(deadline, cx).await;
                        }
                        // logind continues once this is gone.
                        inhibitor = None;
                    }
                }
            }
        }
    }
}

/// Ask every app with unsaved work to write its recovery drafts, and wait
/// for all of them until `deadline`.
async fn preserve_everywhere(names: Vec<String>, deadline: Instant, cx: &mut AsyncApp) {
    let requests = futures_util::future::join_all(names.iter().map(|name| async move {
        if let Err(error) = unsaved::preserve(name).await {
            eprintln!("an app could not keep its unsaved work before the session ended: {error}");
        }
    }))
    .fuse();
    let timeout = cx
        .background_executor()
        .timer(deadline.saturating_duration_since(Instant::now()))
        .fuse();
    futures_util::pin_mut!(requests, timeout);
    futures_util::select! {
        _ = requests => {},
        _ = timeout => eprintln!("an app was still keeping its unsaved work when logind went on"),
    }
}

/// As Log Out does, ask every window to close so each app ends through its
/// own close path, and wait until they have gone or `deadline` passes. A
/// window with unsaved changes shows its Save alert; its draft is already
/// on disk.
async fn ask_every_window_to_close(deadline: Instant, cx: &mut AsyncApp) {
    let snapshot = match rmac_compositor_niri::snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("could not read windows before the session ended: {error:?}");
            return;
        }
    };
    for window in &snapshot.windows {
        let close = rmac_compositor::Action::CloseWindow { window: window.id };
        if let Err(error) = rmac_compositor_niri::execute_action(&close).await {
            eprintln!("could not ask a window to close: {error:?}");
        }
    }
    while Instant::now() < deadline {
        cx.background_executor()
            .timer(QUIT_ALL_CHECK.min(deadline.saturating_duration_since(Instant::now())))
            .await;
        match rmac_compositor_niri::snapshot().await {
            Ok(snapshot) if snapshot.windows.is_empty() => return,
            Ok(_) => {}
            Err(_) => return,
        }
    }
}
