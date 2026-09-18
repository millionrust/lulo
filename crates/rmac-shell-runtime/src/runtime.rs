use async_channel::Sender;

use crate::coordinator::read_service_batch;
use crate::{Coordinator, Error, Snapshot, Update};

pub trait ServiceReader: Send + Sync + 'static {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    >;
    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String>;
    fn audio(&self) -> Result<rmac_audio::Snapshot, String>;
    fn power(&self) -> Result<rmac_power::Snapshot, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemServiceReader;

impl ServiceReader for SystemServiceReader {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    > {
        let network = rmac_network::network_snapshot().map_err(|error| error.to_string())?;
        let wifi = rmac_network::snapshot().map_err(|error| error.to_string())?;
        let vpn = rmac_network::vpn_snapshot().map_err(|error| error.to_string())?;
        Ok((network, wifi, vpn))
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        rmac_bluetooth::snapshot().map_err(|error| error.to_string())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        rmac_audio::snapshot().map_err(|error| error.to_string())
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        rmac_power::snapshot().map_err(|error| error.to_string())
    }
}

/// Publish coherent shell snapshots until the receiving side closes.
pub async fn watch(sender: Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (service_tx, service_rx) = async_channel::bounded(8);
    let (settings_tx, settings_rx) = async_channel::bounded(2);
    let (focus_tx, focus_rx) = async_channel::bounded(4);
    let (notification_tx, notification_rx) = async_channel::bounded(4);

    let compositor = watch_compositor(compositor_tx);
    let services = rmac_shell_status_linux::watch(service_tx);
    let settings = watch_settings(settings_tx);
    let focus = watch_focus(focus_tx);
    let notifications = watch_notifications(notification_tx);
    let consumer = consume(
        sender,
        compositor_rx,
        service_rx,
        settings_rx,
        focus_rx,
        notification_rx,
    );
    let (_, _, _, _, _, _) = futures_util::try_join!(
        compositor,
        async {
            services
                .await
                .map_err(|error| Error::new("watch Linux services", error.to_string()))
        },
        settings,
        focus,
        notifications,
        consumer,
    )?;
    Ok(())
}

async fn watch_notifications(
    sender: Sender<Result<rmac_notifications::Indicator, String>>,
) -> Result<(), Error> {
    rmac_notifications_linux::center::watch(sender)
        .await
        .map_err(|error| Error::new("watch Notification Center", error.to_string()))
}

async fn watch_focus(
    sender: Sender<Result<rmac_focus_runtime::Projection, String>>,
) -> Result<(), Error> {
    rmac_focus_linux::client::watch(sender)
        .await
        .map_err(|error| Error::new("watch Focus authority", error.to_string()))
}

async fn watch_compositor(sender: Sender<rmac_compositor::Event>) -> Result<(), Error> {
    loop {
        let watcher = futures_util::FutureExt::fuse(rmac_compositor_niri::watch(sender.clone()));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(watcher, closed);
        futures_util::select! {
            result = watcher => match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if sender.send(rmac_compositor::Event::ConnectionChanged {
                        state: rmac_compositor::ConnectionState::Disconnected,
                    }).await.is_err() {
                        return Ok(());
                    }
                    eprintln!("rmac-shell-runtime: compositor watch reconnect: {error}");
                }
            },
            _ = closed => return Ok(()),
        }
        async_io::Timer::after(std::time::Duration::from_secs(1)).await;
    }
}

async fn watch_settings(
    sender: Sender<Result<rmac_shell_settings::ShellSettings, String>>,
) -> Result<(), Error> {
    loop {
        let setup = blocking::unblock(|| {
            let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
            let watcher = store.watch()?;
            let snapshot = store.load()?;
            Ok::<_, rmac_shell_settings::Error>((store, watcher, snapshot.settings))
        })
        .await;
        let (mut store, watcher, initial) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
                wait_or_closed(&sender, std::time::Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return Ok(());
                }
                continue;
            }
        };
        if sender.send(Ok(initial)).await.is_err() {
            return Ok(());
        }
        loop {
            let changed = futures_util::FutureExt::fuse(watcher.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed, closed);
            let event = futures_util::select! {
                event = changed => event,
                _ = closed => return Ok(()),
            };
            match event {
                Ok(rmac_shell_settings::StoreEvent::Changed) => {
                    let (returned_store, loaded) = blocking::unblock(move || {
                        let loaded = store.load().map(|snapshot| snapshot.settings);
                        (store, loaded)
                    })
                    .await;
                    store = returned_store;
                    if sender
                        .send(loaded.map_err(|error| error.to_string()))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                    if sender.send(Err(error.to_string())).await.is_err() {
                        return Ok(());
                    }
                }
                Err(_) => break,
            }
        }
    }
}

async fn wait_or_closed<T>(sender: &Sender<T>, duration: std::time::Duration) {
    let timer = futures_util::FutureExt::fuse(async_io::Timer::after(duration));
    let closed = futures_util::FutureExt::fuse(sender.closed());
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

async fn consume(
    sender: Sender<Update>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    services: async_channel::Receiver<rmac_shell_status_linux::Event>,
    settings: async_channel::Receiver<Result<rmac_shell_settings::ShellSettings, String>>,
    focus: async_channel::Receiver<Result<rmac_focus_runtime::Projection, String>>,
    notifications: async_channel::Receiver<Result<rmac_notifications::Indicator, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published = coordinator.snapshot();
    if sender
        .send(Update {
            snapshot: published.clone(),
            visible: true,
            quick_settings_visible: true,
        })
        .await
        .is_err()
    {
        return Ok(());
    }
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let service_event = futures_util::FutureExt::fuse(services.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let focus_event = futures_util::FutureExt::fuse(focus.recv());
        let notification_event = futures_util::FutureExt::fuse(notifications.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(
            compositor_event,
            service_event,
            settings_event,
            focus_event,
            notification_event,
            closed
        );
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new("receive compositor state", "watcher stopped"))?;
                coordinator.apply_compositor(event);
            },
            event = service_event => {
                let event = event.map_err(|_| Error::new("receive Linux service state", "watcher stopped"))?;
                match event {
                    rmac_shell_status_linux::Event::Refresh(sources) => {
                        let batch = blocking::unblock(move || {
                            read_service_batch(sources, &SystemServiceReader)
                        }).await;
                        coordinator.apply_service_batch(batch);
                    }
                    rmac_shell_status_linux::Event::Unavailable { sources, detail } => {
                        coordinator.apply_service_unavailable(sources, detail);
                    }
                }
            },
            event = settings_event => {
                let event = event.map_err(|_| Error::new("receive shell settings", "watcher stopped"))?;
                coordinator.apply_settings(event);
            },
            event = focus_event => {
                let event = event.map_err(|_| Error::new("receive Focus state", "watcher stopped"))?;
                let writable = event.is_ok();
                coordinator.apply_focus(event, writable);
            },
            event = notification_event => {
                let event = event.map_err(|_| Error::new("receive Notification Center state", "watcher stopped"))?;
                coordinator.apply_notifications(event);
            },
            _ = closed => return Ok(()),
        }
        let next = coordinator.snapshot();
        if let Some(update) = publication(&published, next) {
            let next = update.snapshot.clone();
            if sender.send(update).await.is_err() {
                return Ok(());
            }
            published = next;
        }
    }
}

/// The status as the shell draws it: raw Wi-Fi signal percent is snapped to the
/// bar levels the glyph shows, so a drifting signal no longer counts as a
/// visible change (the cause of idle menubar redraws).
fn visible_status(status: &rmac_shell_status::Snapshot) -> rmac_shell_status::Snapshot {
    let mut normalized = status.clone();
    if let Some(network) = normalized.network.as_mut() {
        network.wifi_strength = network.wifi_bars().map(|bars| u8::from(bars) * 25);
    }
    normalized
}

pub(crate) fn publication(previous: &Snapshot, next: Snapshot) -> Option<Update> {
    (next != *previous).then(|| Update {
        // Sub-bar Wi-Fi strength drift is not drawn, so it must not request a
        // frame even though the snapshot changed.
        visible: visible_status(&next.status) != visible_status(&previous.status),
        quick_settings_visible: next.quick_settings != previous.quick_settings,
        snapshot: next,
    })
}
