//! The notification service without a session bus (ADR 0023,
//! "Notification Centre and the app switcher on Windows").
//!
//! Windows has no session bus, so Lulo's Notification Centre runs this
//! service inside `lulo-shell`: the same reducer, history, policies and
//! banner session as Lulo OS's `rmac-notification-center`, with Lulo apps
//! posting over `rmac_notifications::pipe` instead of
//! `org.freedesktop.Notifications`, and the Center panel (in the same
//! process) reading history through [`crate::center`]'s functions, which
//! come here once [`serve`] runs. A notification's action and closing go
//! back to the app that posted it over its pipe connection, as
//! `ActionInvoked` and `NotificationClosed` do on the bus.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rmac_notifications::pipe::{Answer, Post};
use rmac_notifications::protocol::{FreedesktopHints, FreedesktopInput};

use super::*;

/// The bus-less service's links: the apps waiting to hear about their
/// notifications, and the Center panels following history.
#[derive(Default)]
pub struct Link {
    answers: Mutex<HashMap<NotificationId, std::sync::mpsc::Sender<Answer>>>,
    watchers: Mutex<Vec<Sender<()>>>,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Link(<redacted>)")
    }
}

impl Link {
    /// Tell the app that posted `id` (if it still listens).
    pub(super) fn answer(&self, id: NotificationId, answer: Answer) {
        let mut answers = self
            .answers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let closed = matches!(answer, Answer::Closed(_));
        let gone = answers
            .get(&id)
            .is_some_and(|sender| sender.send(answer).is_err());
        if gone || closed {
            answers.remove(&id);
        }
    }

    /// History changed: every following Center reads it again.
    pub(super) fn changed(&self) {
        self.watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|watcher| {
                // A full channel already holds a change to read.
                !matches!(
                    watcher.try_send(()),
                    Err(async_channel::TrySendError::Closed(_))
                )
            });
    }

    fn subscribe(&self) -> Receiver<()> {
        let (sender, receiver) = async_channel::bounded(1);
        self.watchers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(sender);
        receiver
    }
}

static CENTER: OnceLock<ServiceHandle> = OnceLock::new();

/// The running bus-less service, for [`crate::center`] in this process.
pub fn center() -> Option<&'static ServiceHandle> {
    CENTER.get()
}

/// Serve notifications without a bus: history from the same store as Lulo
/// OS (`rmac_notifications_store::Store::from_environment`). The events
/// receiver feeds the banner host, as [`super::serve`]'s does.
pub async fn serve() -> Result<(ServiceHandle, Receiver<RuntimeEvent>), ServiceError> {
    let store =
        rmac_notifications_store::Store::from_environment().map_err(|_| ServiceError::History)?;
    let center = blocking::unblock({
        let store = store.clone();
        move || store.load()
    })
    .await
    .map_err(|_| ServiceError::History)?
    .center;
    let history = HistoryAuthority {
        center: Arc::new(Mutex::new(center)),
        store,
        focus_connection: None,
        focus_enforced: false,
        origins: crate::origin::Origins::default(),
    };
    Ok(serve_with(history))
}

fn serve_with(history: HistoryAuthority) -> (ServiceHandle, Receiver<RuntimeEvent>) {
    let core = SharedCore::new(500, BANNER_TIMEOUT);
    core.reserve_ids(history.ids());
    let (events, receiver) = async_channel::bounded(EVENT_CAPACITY);
    let handle = ServiceHandle {
        connection: None,
        core,
        events,
        history,
        local: Some(Arc::new(Link::default())),
    };
    let _ = CENTER.set(handle.clone());
    (handle, receiver)
}

/// Who posted, as the pipe's client process says.
#[derive(Clone, Debug, Default)]
pub struct Poster {
    /// The poster's app id (a Lulo app's, or `exe:<name>`), which its
    /// Center policy is kept under.
    pub app_id: String,
    /// Its desktop entry, when it is a Lulo app.
    pub desktop_id: Option<String>,
    /// Its executable's path.
    pub executable: Option<String>,
    /// Its name, for a sender without a desktop entry.
    pub app_name: Option<String>,
}

fn invalid() -> ServiceError {
    ServiceError::Invalid
}

impl ServiceHandle {
    /// A Lulo app's notification, from the pipe. `answers` hears its
    /// action and its closing. Returns its id.
    pub async fn post_local(
        &self,
        poster: Poster,
        post: Post,
        answers: std::sync::mpsc::Sender<Answer>,
    ) -> Result<NotificationId, ServiceError> {
        let local = self.local.as_ref().ok_or(ServiceError::Bus)?;
        let actions = post
            .actions
            .into_iter()
            .flat_map(|(key, label)| [key, label])
            .collect();
        let request = rmac_notifications::protocol::freedesktop(FreedesktopInput {
            authenticated_app_id: poster.app_id,
            replaces_id: 0,
            summary: post.summary,
            body_plain: post.body,
            actions,
            hints: FreedesktopHints {
                urgency: Some(post.urgency),
                resident: post.resident,
                transient: post.transient,
                suppress_sound: post.suppress_sound,
                category: None,
            },
            // A resident (alert) notification stays until answered.
            expire_timeout: if post.resident { 0 } else { -1 },
        })
        .map_err(|_| invalid())?;
        let policy = self.history.policy(request.source.app_id()).await;
        let (outcome, notification, evictions) = self
            .core
            .post_event(request, policy)
            .map_err(|_| invalid())?;
        local
            .answers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(outcome.id, answers);
        self.history.annotate_sender(
            outcome.id,
            (poster.desktop_id.clone(), poster.executable),
            crate::origin::SenderHints {
                desktop_entry: poster.desktop_id,
                app_name: poster.app_name,
                icon: None,
            },
        );
        for eviction in evictions {
            let id = eviction.closed.id;
            publish(&self.events, RuntimeEvent::Closed(eviction.closed))
                .await
                .map_err(|_| ServiceError::Bus)?;
            local.answer(id, Answer::Closed(1));
        }
        publish(
            &self.events,
            RuntimeEvent::Posted {
                outcome,
                notification,
                media: crate::media::NotificationMedia::default(),
            },
        )
        .await
        .map_err(|_| ServiceError::Bus)?;
        Ok(outcome.id)
    }

    /// The Center's history and policies, as `org.rmac.NotificationCenter1`
    /// gives them.
    pub fn center_snapshot(&self) -> Result<crate::center::Snapshot, crate::center::Error> {
        let (history, applications) = self.history.snapshot(&self.core.snapshot());
        let mut snapshot = crate::center::decode_snapshot(history, applications)?;
        crate::center::attach_origins(&mut snapshot, self.history.origins());
        Ok(snapshot)
    }

    fn center_mutation(
        &self,
        outcome: RecordOutcome,
    ) -> Result<rmac_notifications::Indicator, crate::center::Error> {
        if outcome.changed {
            if let Some(local) = &self.local {
                local.changed();
            }
        }
        if !outcome.persisted {
            return Err(crate::center::Error::Persistence);
        }
        Ok(outcome.indicator)
    }

    pub fn center_mark_read(
        &self,
        app_id: Option<&str>,
    ) -> Result<rmac_notifications::Indicator, crate::center::Error> {
        let app_id = optional(app_id)?;
        self.center_mutation(self.history.mark_read(app_id.as_ref()))
    }

    pub fn center_clear(
        &self,
        app_id: Option<&str>,
    ) -> Result<rmac_notifications::Indicator, crate::center::Error> {
        let app_id = optional(app_id)?;
        self.center_mutation(self.history.clear(app_id.as_ref()))
    }

    pub fn center_remove(
        &self,
        id: NotificationId,
    ) -> Result<rmac_notifications::Indicator, crate::center::Error> {
        self.center_mutation(self.history.remove(id))
    }

    pub fn center_set_policy(
        &self,
        app_id: &str,
        policy: rmac_notifications_store::AppPolicy,
    ) -> Result<rmac_notifications::Indicator, crate::center::Error> {
        let app_id = AppId::parse(app_id).map_err(|_| crate::center::Error::Invalid)?;
        let outcome = self
            .history
            .set_policy(app_id, policy)
            .map_err(|_| crate::center::Error::Invalid)?;
        self.center_mutation(outcome)
    }

    /// One of a Center card's actions (or its click), as the Center's
    /// `Invoke` runs it.
    pub async fn center_invoke(
        &self,
        id: NotificationId,
        selection: crate::center::ActionSelection,
    ) -> Result<(), crate::center::Error> {
        let selection = match selection {
            crate::center::ActionSelection::Default => ActionSelection::Default,
            crate::center::ActionSelection::Button(index) if index <= 7 => {
                ActionSelection::Button(index.into())
            }
            crate::center::ActionSelection::Button(_) => return Err(crate::center::Error::Invalid),
        };
        self.invoke_from_center(id, selection, None)
            .await
            .map(drop)
            .map_err(|_| crate::center::Error::Call)
    }

    /// Publish the Center's snapshot now and after every change, until
    /// `sender` closes.
    pub async fn center_watch_snapshot(
        &self,
        sender: &Sender<Result<crate::center::Snapshot, String>>,
    ) -> Result<(), crate::center::Error> {
        let local = self.local.as_ref().ok_or(crate::center::Error::Connect)?;
        let changes = local.subscribe();
        loop {
            let handle = self.clone();
            let snapshot = blocking::unblock(move || handle.center_snapshot())
                .await
                .map_err(|error| error.to_string());
            if sender.send(snapshot).await.is_err() {
                return Ok(());
            }
            let changed = futures_util::FutureExt::fuse(changes.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed, closed);
            futures_util::select! {
                change = changed => if change.is_err() { return Ok(()) },
                _ = closed => return Ok(()),
            }
        }
    }
}

fn optional(app_id: Option<&str>) -> Result<Option<AppId>, crate::center::Error> {
    match app_id.filter(|app_id| !app_id.is_empty()) {
        None => Ok(None),
        Some(app_id) => AppId::parse(app_id)
            .map(Some)
            .map_err(|_| crate::center::Error::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(path: std::path::PathBuf) -> (ServiceHandle, Receiver<RuntimeEvent>) {
        let mut history = HistoryAuthority::empty_at(path);
        history.focus_enforced = false;
        let core = SharedCore::new(500, BANNER_TIMEOUT);
        let (events, receiver) = async_channel::bounded(EVENT_CAPACITY);
        (
            ServiceHandle {
                connection: None,
                core,
                events,
                history,
                local: Some(Arc::new(Link::default())),
            },
            receiver,
        )
    }

    fn poster() -> Poster {
        Poster {
            app_id: "org.rmac.Clock".into(),
            desktop_id: Some("org.rmac.Clock".into()),
            executable: Some(r"C:\Lulo\rmac-clock.exe".into()),
            app_name: Some("Clock".into()),
        }
    }

    fn alarm() -> Post {
        Post {
            summary: "Alarm".into(),
            body: "7:00 AM".into(),
            actions: vec![
                ("default".into(), "Open".into()),
                ("stop".into(), "Stop".into()),
            ],
            urgency: 2,
            resident: false,
            transient: false,
            suppress_sound: true,
        }
    }

    #[test]
    fn a_pipe_post_reaches_banners_and_history_and_its_answers_reach_the_app() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-local-notifications-{}-{}",
            std::process::id(),
            crate::origin::unix_ms_now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let (service, events) = handle(directory.join("history.json"));
        let (answers_tx, answers) = std::sync::mpsc::channel();
        let id = async_io::block_on(service.post_local(poster(), alarm(), answers_tx)).unwrap();

        // The banner host gets the post, unchanged from the bus path.
        let event = events.try_recv().unwrap();
        let RuntimeEvent::Posted {
            outcome,
            notification: Some(notification),
            ..
        } = &event
        else {
            panic!("expected a posted notification");
        };
        assert_eq!(outcome.id, id);
        assert!(outcome.delivery.banner);
        assert_eq!(notification.source.app_id().as_str(), "org.rmac.Clock");
        // History records it, labelled with its sender.
        let record = service.history.record(&event).unwrap();
        assert!(record.changed);
        let snapshot = service.center_snapshot().unwrap();
        assert_eq!(snapshot.records.len(), 1);
        assert_eq!(
            snapshot.records[0].origin.desktop_id.as_deref(),
            Some("org.rmac.Clock")
        );

        // The banner's Stop button goes back to the app, then its closing.
        let invocation =
            async_io::block_on(service.invoke(id, ActionSelection::Button(0), None)).unwrap();
        assert_eq!(invocation.action_id, "stop");
        assert_eq!(answers.try_recv(), Ok(Answer::Action("stop".into())));
        assert_eq!(answers.try_recv(), Ok(Answer::Closed(2)));

        // Clearing the Center tells its panels.
        let changes = service.local.as_ref().unwrap().subscribe();
        service.center_clear(None).unwrap();
        assert!(changes.try_recv().is_ok());
        assert!(service.center_snapshot().unwrap().records.is_empty());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn without_focus_the_policy_alone_decides_delivery() {
        let (service, _events) = handle(std::env::temp_dir().join("rmac-local-policy.json"));
        let app = AppId::parse("org.rmac.Clock").unwrap();
        let policy = async_io::block_on(service.history.policy(&app));
        assert!(!policy.focus_active);
    }
}
