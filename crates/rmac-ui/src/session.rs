//! Keeping unsaved work when the session ends, however it ends.
//!
//! Log Out, Restart and Shut Down from the menu bar ask every window to
//! close, so an edited document gets its Save alert. Everything else — a
//! `systemctl poweroff` in Terminal, the power button, a low-battery
//! shutdown, the compositor exiting — reaches an app only as a signal, or not
//! at all. This module gives both a way to keep the user's work:
//!
//! * [`preserve_on_session_end`] registers a view's "write my recovery draft
//!   now" hook. It runs when the app quits for any reason (SIGTERM, SIGHUP
//!   and SIGINT quit the app through GPUI, see `signals`), and whenever the
//!   menu bar asks, before logind shuts down or sleeps.
//! * [`set_unsaved`] tells the session this view holds unsaved work. While
//!   any view does, the process owns `org.rmac.UnsavedWork.p<pid>` on the
//!   session bus, which is what makes the menu bar hold its logind delay
//!   inhibitor. Nothing is connected until the first unsaved document.
//!
//! Nothing here polls: signals arrive on a blocked thread, and bus requests
//! and state changes on channels.

use std::collections::HashSet;

use gpui::{App, Context, EntityId, Global};

#[cfg(unix)]
mod signals;
mod unsaved_set;

use unsaved_set::UnsavedSet;

/// How long an app may take to quit after a termination signal before it is
/// ended anyway: long enough to write every draft, short enough that a hung
/// app never holds up a shutdown.
#[cfg(unix)]
const FORCED_EXIT_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

type Preserver = Box<dyn FnMut(&mut App) -> bool>;

#[derive(Default)]
struct SessionEnd {
    installed: bool,
    preservers: Vec<Preserver>,
    releases_watched: HashSet<EntityId>,
    unsaved: UnsavedSet<EntityId>,
    publish: Option<async_channel::Sender<bool>>,
}

impl Global for SessionEnd {}

/// Called once from `init_application`.
pub(crate) fn install(cx: &mut App) {
    if std::mem::replace(&mut cx.default_global::<SessionEnd>().installed, true) {
        return;
    }
    #[cfg(unix)]
    {
        match signals::listen(Some(FORCED_EXIT_AFTER)) {
            Ok(received) => {
                cx.spawn(async move |cx| {
                    if let Ok(signal) = received.recv().await {
                        eprintln!("quitting on signal {signal}; keeping unsaved work first");
                        cx.update(|cx| cx.quit());
                    }
                })
                .detach();
            }
            Err(error) => eprintln!("unsaved work is not kept on SIGTERM: {error}"),
        }
    }
    cx.on_app_quit(|cx| {
        preserve_all(cx);
        async {}
    })
    .detach();
    #[cfg(target_os = "linux")]
    {
        publish_unsaved_work(cx);
    }
}

/// Serve `org.rmac.UnsavedWork1` and own its name while unsaved.
#[cfg(target_os = "linux")]
fn publish_unsaved_work(cx: &mut App) {
    use rmac_app_menu::unsaved;

    let (state_tx, state_rx) = async_channel::unbounded::<bool>();
    let (preserve_tx, preserve_rx) = unsaved::preserve_channel();
    cx.default_global::<SessionEnd>().publish = Some(state_tx);
    cx.spawn(async move |cx| {
        while let Ok(done) = preserve_rx.recv().await {
            cx.update(|cx| preserve_all(cx));
            if done.try_send(()).is_err() {
                eprintln!("the session stopped waiting for unsaved work to be kept");
            }
        }
    })
    .detach();
    cx.background_executor()
        .spawn(async move {
            let mut endpoint: Option<unsaved::UnsavedEndpoint> = None;
            while let Ok(mut wanted) = state_rx.recv().await {
                while let Ok(newer) = state_rx.try_recv() {
                    wanted = newer;
                }
                if endpoint.is_none() {
                    if !wanted {
                        continue;
                    }
                    match unsaved::UnsavedEndpoint::connect(preserve_tx.clone()).await {
                        Ok(connected) => endpoint = Some(connected),
                        Err(error) => {
                            eprintln!("the session cannot see this app's unsaved work: {error}");
                            continue;
                        }
                    }
                }
                if let Some(endpoint) = endpoint.as_mut() {
                    if let Err(error) = endpoint.set_unsaved(wanted).await {
                        eprintln!("could not tell the session about unsaved work: {error}");
                    }
                }
            }
        })
        .detach();
}

/// Run `preserve` on this view whenever the session may be about to end:
/// on every quit of the app (including one forced by SIGTERM) and when the
/// session asks before a shutdown or sleep. It must write the view's
/// recovery draft synchronously, and do nothing for a clean view.
pub fn preserve_on_session_end<T: 'static>(
    cx: &mut Context<T>,
    mut preserve: impl FnMut(&mut T, &mut Context<T>) + 'static,
) {
    let view = cx.weak_entity();
    cx.default_global::<SessionEnd>()
        .preservers
        .push(Box::new(move |cx| {
            view.update(cx, |this, cx| preserve(this, cx)).is_ok()
        }));
    watch_release(cx);
}

/// Say whether this view holds unsaved work. A released view counts as
/// saved.
pub fn set_unsaved<T: 'static>(cx: &mut Context<T>, unsaved: bool) {
    let id = cx.entity_id();
    watch_release(cx);
    update_unsaved(id, unsaved, cx);
}

fn watch_release<T: 'static>(cx: &mut Context<T>) {
    let id = cx.entity_id();
    if cx
        .default_global::<SessionEnd>()
        .releases_watched
        .insert(id)
    {
        cx.on_release(move |_, cx| {
            update_unsaved(id, false, cx);
            cx.default_global::<SessionEnd>()
                .releases_watched
                .remove(&id);
        })
        .detach();
    }
}

fn update_unsaved(id: EntityId, unsaved: bool, cx: &mut App) {
    let state = cx.default_global::<SessionEnd>();
    let Some(changed) = state.unsaved.set(id, unsaved) else {
        return;
    };
    if let Some(publish) = &state.publish {
        if publish.try_send(changed).is_err() {
            eprintln!("could not tell the session about unsaved work");
        }
    }
}

/// Run every live view's preserve hook, dropping those of released views.
fn preserve_all(cx: &mut App) {
    let preservers = std::mem::take(&mut cx.default_global::<SessionEnd>().preservers);
    let mut kept = Vec::with_capacity(preservers.len());
    for mut preserver in preservers {
        if preserver(cx) {
            kept.push(preserver);
        }
    }
    let state = cx.default_global::<SessionEnd>();
    // Hooks registered while these ran come after the ones kept.
    kept.append(&mut state.preservers);
    state.preservers = kept;
}
