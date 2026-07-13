//! Concurrent provider orchestration and overlay-facing launcher lifecycle.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt as _};
use rmac_launcher::{
    ActivationMode, MoveSelection, ProviderDescriptor, ProviderError, Request, ResultId,
};
use rmac_launcher_providers::{Batch, Provider};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CatalogHealth {
    #[default]
    Unmanaged,
    Starting,
    Healthy,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogUpdate {
    pub health: CatalogHealth,
    pub revision: u64,
    pub changed: bool,
    /// Diagnostics only. Overlay snapshots deliberately omit this value.
    pub detail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CatalogEffect {
    pub visible: bool,
    pub request: Option<Request>,
}

/// Watch installed applications without a discovery/watch race. Failures keep
/// the provider's last-known-good catalog and retry with a bounded delay.
pub async fn watch_application_catalog(
    provider: rmac_launcher_providers::ApplicationProvider,
    sender: async_channel::Sender<CatalogUpdate>,
) {
    if sender
        .send(CatalogUpdate {
            health: CatalogHealth::Starting,
            revision: provider.revision(),
            changed: false,
            detail: None,
        })
        .await
        .is_err()
    {
        return;
    }
    loop {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let setup = blocking::unblock(move || {
            let callback = changed_tx.clone();
            rmac_apps::watch_catalog(move || {
                let _ = callback.try_send(());
            })
            .map(|watcher| (watcher, changed_rx))
        })
        .await;
        let (watcher, changed) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if send_catalog_failure(&provider, &sender, error.to_string())
                    .await
                    .is_err()
                {
                    return;
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return;
                }
                continue;
            }
        };
        let _watcher = watcher;
        let mut refresh = true;
        loop {
            if refresh {
                let discovery = blocking::unblock(rmac_apps::discover).await;
                refresh = discovery.is_err();
                let update = match discovery {
                    Ok(catalog) => CatalogUpdate {
                        changed: provider.replace_catalog(catalog),
                        health: CatalogHealth::Healthy,
                        revision: provider.revision(),
                        detail: None,
                    },
                    Err(error) => CatalogUpdate {
                        health: CatalogHealth::Unavailable,
                        revision: provider.revision(),
                        changed: false,
                        detail: Some(error.to_string()),
                    },
                };
                if sender.send(update).await.is_err() {
                    return;
                }
            }

            let changed_event = futures_util::FutureExt::fuse(changed.recv());
            let should_retry = refresh;
            let retry = futures_util::FutureExt::fuse(async move {
                if should_retry {
                    async_io::Timer::after(Duration::from_secs(1)).await;
                } else {
                    std::future::pending::<()>().await;
                }
            });
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed_event, retry, closed);
            futures_util::select! {
                event = changed_event => {
                    if event.is_err() {
                        break;
                    }
                    refresh = true;
                },
                _ = retry => refresh = true,
                _ = closed => return,
            }
        }
    }
}

async fn send_catalog_failure(
    provider: &rmac_launcher_providers::ApplicationProvider,
    sender: &async_channel::Sender<CatalogUpdate>,
    detail: String,
) -> Result<(), async_channel::SendError<CatalogUpdate>> {
    sender
        .send(CatalogUpdate {
            health: CatalogHealth::Unavailable,
            revision: provider.revision(),
            changed: false,
            detail: Some(detail),
        })
        .await
}

async fn wait_or_closed<T>(sender: &async_channel::Sender<T>, duration: Duration) {
    let timer = futures_util::FutureExt::fuse(async_io::Timer::after(duration));
    let closed = futures_util::FutureExt::fuse(sender.closed());
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

#[derive(Clone, Debug)]
pub enum SettingsUpdate {
    Snapshot(Box<rmac_shell_settings::ShellSettings>),
    Unavailable(String),
}

/// Watch the complete C4 authority without a load/watch race. Consumers retain
/// their last-good settings across failures and rebuild provider scope only
/// from complete snapshots.
pub async fn watch_shell_settings(sender: async_channel::Sender<SettingsUpdate>) {
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
                if sender
                    .send(SettingsUpdate::Unavailable(error.to_string()))
                    .await
                    .is_err()
                {
                    return;
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return;
                }
                continue;
            }
        };
        if sender
            .send(SettingsUpdate::Snapshot(Box::new(initial)))
            .await
            .is_err()
        {
            return;
        }
        loop {
            let changed = futures_util::FutureExt::fuse(watcher.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed, closed);
            let event = futures_util::select! {
                event = changed => event,
                _ = closed => return,
            };
            match event {
                Ok(rmac_shell_settings::StoreEvent::Changed) => {
                    let (returned_store, loaded) = blocking::unblock(move || {
                        let loaded = store.load().map(|snapshot| snapshot.settings);
                        (store, loaded)
                    })
                    .await;
                    store = returned_store;
                    let update = loaded.map_or_else(
                        |error| SettingsUpdate::Unavailable(error.to_string()),
                        |settings| SettingsUpdate::Snapshot(Box::new(settings)),
                    );
                    if sender.send(update).await.is_err() {
                        return;
                    }
                }
                Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                    if sender
                        .send(SettingsUpdate::Unavailable(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
                Err(error) => {
                    if sender
                        .send(SettingsUpdate::Unavailable(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryError {
    detail: String,
}

impl RegistryError {
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for RegistryError {}

/// Immutable provider registry. A descriptor is captured once so provider
/// identity cannot change between privacy admission and dispatch.
#[derive(Default)]
pub struct Registry {
    providers: BTreeMap<rmac_shell_settings::ProviderId, RegisteredProvider>,
}

struct RegisteredProvider {
    descriptor: ProviderDescriptor,
    provider: Arc<dyn Provider>,
}

impl Registry {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Result<Self, RegistryError> {
        let mut registered = BTreeMap::new();
        for provider in providers {
            let descriptor = provider.descriptor();
            if descriptor.id.0.trim().is_empty() {
                return Err(RegistryError {
                    detail: "provider ID must not be empty".into(),
                });
            }
            let id = descriptor.id.clone();
            if registered
                .insert(
                    id.clone(),
                    RegisteredProvider {
                        descriptor,
                        provider,
                    },
                )
                .is_some()
            {
                return Err(RegistryError {
                    detail: format!("duplicate provider ID {}", id.0),
                });
            }
        }
        Ok(Self {
            providers: registered,
        })
    }

    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        self.providers
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect()
    }

    /// Run admitted providers concurrently on the blocking pool. Closing the
    /// receiver cancels the shared request so filesystem work can stop early.
    pub async fn dispatch(&self, request: Request, sender: async_channel::Sender<Batch>) {
        if sender.is_closed() {
            request.cancellation.cancel();
            return;
        }
        let jobs: Vec<_> = request
            .providers
            .iter()
            .filter_map(|descriptor| {
                self.providers
                    .get(&descriptor.id)
                    .filter(|registered| registered.descriptor == *descriptor)
                    .map(|registered| (registered.descriptor.clone(), registered.provider.clone()))
            })
            .collect();

        stream::iter(jobs)
            .for_each_concurrent(None, |(descriptor, provider)| {
                let worker_request = request.clone();
                let cancellation = request.cancellation.clone();
                let generation = request.generation;
                let sender = sender.clone();
                async move {
                    if sender.is_closed() {
                        cancellation.cancel();
                        return;
                    }
                    let batch = blocking::unblock(move || {
                        rmac_launcher_providers::execute(&worker_request, provider.as_ref())
                    })
                    .await;
                    let batch = batch.or_else(|| {
                        (!cancellation.is_cancelled()).then_some(Batch {
                            generation,
                            provider: descriptor.id,
                            results: Err(ProviderError {
                                detail: "provider descriptor changed after registration".into(),
                            }),
                        })
                    });
                    if let Some(batch) = batch {
                        if sender.send(batch).await.is_err() {
                            cancellation.cancel();
                        }
                    }
                }
            })
            .await;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusTarget {
    Query,
}

#[derive(Clone, Debug)]
pub struct OpenEffect {
    pub request: Request,
    pub focus: FocusTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCommand {
    ArrowDown,
    ArrowUp,
    Return,
    AlternateReturn,
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Activation {
    pub id: rmac_launcher_system::ActivationId,
    pub action: rmac_launcher::Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyEffect {
    None,
    SelectionChanged,
    Activate(Activation),
    Dismissed,
}

#[derive(Clone, Debug)]
pub enum ShortcutEffect {
    None,
    Open(OpenEffect),
    Dismissed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Phase {
    Closed,
    Loading,
    Results {
        still_searching: bool,
        degraded: bool,
    },
    Empty,
    Unavailable,
    Activating,
    ActivationFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub id: ResultId,
    pub category: rmac_launcher::Category,
    pub category_label: &'static str,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<std::path::PathBuf>,
    pub selected: bool,
    pub has_alternate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub open: bool,
    pub query: String,
    pub phase: Phase,
    pub rows: Vec<Row>,
    pub pending_providers: usize,
    pub failed_providers: usize,
    pub application_catalog: CatalogHealth,
    /// A concise live-region message. It intentionally contains no provider
    /// error detail, file path, or action payload.
    pub announcement: Option<String>,
}

pub struct Coordinator {
    launcher: rmac_launcher::Launcher,
    descriptors: Vec<ProviderDescriptor>,
    policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    next_activation: u64,
    activation: Option<rmac_launcher_system::ActivationId>,
    activation_error: Option<String>,
    last_shortcut_timestamp_ms: Option<u64>,
    application_catalog: CatalogHealth,
    application_catalog_revision: u64,
}

impl Coordinator {
    pub fn new(
        descriptors: Vec<ProviderDescriptor>,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Self {
        Self {
            launcher: rmac_launcher::Launcher::default(),
            descriptors,
            policies,
            next_activation: 0,
            activation: None,
            activation_error: None,
            last_shortcut_timestamp_ms: None,
            application_catalog: CatalogHealth::Unmanaged,
            application_catalog_revision: 0,
        }
    }

    /// Open synchronously. The caller must apply `focus` before it schedules
    /// provider work, so an empty-query filesystem search cannot delay typing.
    pub fn open(&mut self) -> OpenEffect {
        self.activation = None;
        self.activation_error = None;
        OpenEffect {
            request: self.launcher.open(&self.descriptors, &self.policies),
            focus: FocusTarget::Query,
        }
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> Option<Request> {
        if self.activation.is_some() {
            return None;
        }
        self.activation_error = None;
        self.launcher
            .set_query(query, &self.descriptors, &self.policies)
    }

    /// Reapply provider privacy policy immediately. If the overlay is open,
    /// this cancels the old generation before newly admitted providers run.
    pub fn set_policies(
        &mut self,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        if self.policies == policies {
            return None;
        }
        self.policies = policies;
        if self.launcher.is_open() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        }
    }

    /// Replace the complete provider environment after an authoritative
    /// settings/scope reload. An open query is cancelled and reissued once.
    pub fn set_environment(
        &mut self,
        descriptors: Vec<ProviderDescriptor>,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        if self.descriptors == descriptors && self.policies == policies {
            return None;
        }
        self.descriptors = descriptors;
        self.policies = policies;
        if self.launcher.is_open() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        }
    }

    pub fn select(&mut self, id: &ResultId) -> bool {
        self.launcher.is_open() && self.launcher.session_mut().select(id)
    }

    pub fn activate_selected(&mut self, mode: ActivationMode) -> KeyEffect {
        if !self.launcher.is_open() {
            KeyEffect::None
        } else {
            self.activation(mode)
        }
    }

    pub fn apply(&mut self, batch: Batch) -> bool {
        self.launcher.is_open()
            && self
                .launcher
                .session_mut()
                .apply(batch.generation, batch.provider, batch.results)
    }

    pub fn apply_catalog(&mut self, update: CatalogUpdate) -> CatalogEffect {
        if update.revision < self.application_catalog_revision {
            return CatalogEffect {
                visible: false,
                request: None,
            };
        }
        let visible = self.application_catalog != update.health;
        let revision_changed =
            update.changed && update.revision > self.application_catalog_revision;
        self.application_catalog = update.health;
        self.application_catalog_revision = self.application_catalog_revision.max(update.revision);
        let request = if revision_changed && self.launcher.is_open() && self.activation.is_none() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        };
        CatalogEffect {
            visible: visible || request.is_some(),
            request,
        }
    }

    /// Handle only the stable launcher shortcut. Duplicate or older portal
    /// timestamps are ignored, and a fresh activation toggles the overlay.
    pub fn handle_shortcut(&mut self, event: &rmac_shortcuts::Event) -> ShortcutEffect {
        let rmac_shortcuts::Event::Activated { id, timestamp_ms } = event else {
            return ShortcutEffect::None;
        };
        if id.0 != "launcher"
            || self
                .last_shortcut_timestamp_ms
                .is_some_and(|last| *timestamp_ms <= last)
        {
            return ShortcutEffect::None;
        }
        self.last_shortcut_timestamp_ms = Some(*timestamp_ms);
        if self.launcher.is_open() {
            self.activation = None;
            self.activation_error = None;
            self.launcher.escape();
            ShortcutEffect::Dismissed
        } else {
            ShortcutEffect::Open(self.open())
        }
    }

    pub fn handle_key(&mut self, command: KeyCommand) -> KeyEffect {
        if !self.launcher.is_open() {
            return KeyEffect::None;
        }
        match command {
            KeyCommand::ArrowDown => {
                let before = self.launcher.session().selected().cloned();
                self.launcher
                    .session_mut()
                    .move_selection(MoveSelection::Next);
                if before != self.launcher.session().selected().cloned() {
                    KeyEffect::SelectionChanged
                } else {
                    KeyEffect::None
                }
            }
            KeyCommand::ArrowUp => {
                let before = self.launcher.session().selected().cloned();
                self.launcher
                    .session_mut()
                    .move_selection(MoveSelection::Previous);
                if before != self.launcher.session().selected().cloned() {
                    KeyEffect::SelectionChanged
                } else {
                    KeyEffect::None
                }
            }
            KeyCommand::Return => self.activation(ActivationMode::Primary),
            KeyCommand::AlternateReturn => self.activation(ActivationMode::Alternate),
            KeyCommand::Escape => {
                self.activation = None;
                self.activation_error = None;
                self.launcher.escape();
                KeyEffect::Dismissed
            }
        }
    }

    pub fn finish_activation(
        &mut self,
        result: Result<rmac_launcher_system::Receipt, rmac_launcher_system::Error>,
    ) -> bool {
        let id = match &result {
            Ok(receipt) => receipt.activation,
            Err(error) => error.activation,
        };
        if self.activation != Some(id) || !self.launcher.is_open() {
            return false;
        }
        self.activation = None;
        match result {
            Ok(_) => {
                self.activation_error = None;
                self.launcher.escape();
            }
            Err(error) => self.activation_error = Some(error.to_string()),
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        if !self.launcher.is_open() {
            return Snapshot {
                open: false,
                query: String::new(),
                phase: Phase::Closed,
                rows: Vec::new(),
                pending_providers: 0,
                failed_providers: 0,
                application_catalog: self.application_catalog.clone(),
                announcement: None,
            };
        }
        let session = self.launcher.session();
        let pending = session.pending().len();
        let failed = session.errors().len();
        let selected = session.selected();
        let rows: Vec<_> = session
            .results()
            .iter()
            .map(|ranked| Row {
                id: ranked.result.id.clone(),
                category: ranked.result.category,
                category_label: ranked.result.category.label(),
                title: ranked.result.title.clone(),
                subtitle: ranked.result.subtitle.clone(),
                icon: ranked.result.icon.clone(),
                selected: selected == Some(&ranked.result.id),
                has_alternate: ranked.result.alternate.is_some(),
            })
            .collect();
        let catalog_starting = self.application_catalog == CatalogHealth::Starting;
        let catalog_unavailable = self.application_catalog == CatalogHealth::Unavailable;
        let phase = if self.activation.is_some() {
            Phase::Activating
        } else if self.activation_error.is_some() {
            Phase::ActivationFailed
        } else if rows.is_empty() && (pending > 0 || catalog_starting) {
            Phase::Loading
        } else if !rows.is_empty() {
            Phase::Results {
                still_searching: pending > 0,
                degraded: failed > 0 || catalog_unavailable,
            }
        } else if failed > 0 || catalog_unavailable {
            Phase::Unavailable
        } else {
            Phase::Empty
        };
        let announcement = announcement(&phase, &rows, self.activation_error.as_deref());
        Snapshot {
            open: true,
            query: session.query().to_owned(),
            phase,
            rows,
            pending_providers: pending,
            failed_providers: failed,
            application_catalog: self.application_catalog.clone(),
            announcement,
        }
    }

    fn activation(&mut self, mode: ActivationMode) -> KeyEffect {
        if self.activation.is_some() {
            return KeyEffect::None;
        }
        let Some(action) = self.launcher.session().activation(mode) else {
            return KeyEffect::None;
        };
        self.next_activation = self.next_activation.wrapping_add(1).max(1);
        let id = rmac_launcher_system::ActivationId(self.next_activation);
        self.activation = Some(id);
        self.activation_error = None;
        KeyEffect::Activate(Activation { id, action })
    }
}

fn announcement(phase: &Phase, rows: &[Row], activation_error: Option<&str>) -> Option<String> {
    match phase {
        Phase::Closed => None,
        Phase::Loading => Some("Searching".into()),
        Phase::Results { .. } => {
            let selected = rows.iter().position(|row| row.selected)?;
            let row = &rows[selected];
            Some(format!(
                "{}, {}, {} of {} results",
                row.title,
                row.category_label,
                selected + 1,
                rows.len()
            ))
        }
        Phase::Empty => Some("No results".into()),
        Phase::Unavailable => Some("Search providers are unavailable".into()),
        Phase::Activating => Some("Opening selection".into()),
        Phase::ActivationFailed => activation_error.map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use rmac_launcher::{Action, Cancellation, Category, Privacy, ProviderError, SearchResult};

    fn provider_id(value: &str) -> rmac_shell_settings::ProviderId {
        rmac_shell_settings::ProviderId(value.into())
    }

    #[derive(Clone)]
    struct FakeProvider {
        descriptor: ProviderDescriptor,
        title: &'static str,
        calls: Arc<AtomicUsize>,
    }

    impl Provider for FakeProvider {
        fn descriptor(&self) -> ProviderDescriptor {
            self.descriptor.clone()
        }

        fn search(
            &self,
            _: &str,
            cancellation: &Cancellation,
        ) -> Result<Vec<SearchResult>, ProviderError> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            if cancellation.is_cancelled() {
                return Err(ProviderError {
                    detail: "cancelled".into(),
                });
            }
            Ok(vec![result(
                &self.descriptor.id.0,
                self.descriptor.category,
                self.title,
            )])
        }
    }

    struct ChangingProvider {
        descriptor_calls: AtomicUsize,
    }

    impl Provider for ChangingProvider {
        fn descriptor(&self) -> ProviderDescriptor {
            let id = if self.descriptor_calls.fetch_add(1, Ordering::AcqRel) == 0 {
                "stable"
            } else {
                "changed"
            };
            descriptor(id, Category::Settings, false)
        }

        fn search(&self, _: &str, _: &Cancellation) -> Result<Vec<SearchResult>, ProviderError> {
            panic!("a provider with changed identity must not receive a query")
        }
    }

    fn descriptor(id: &str, category: Category, private: bool) -> ProviderDescriptor {
        ProviderDescriptor {
            id: provider_id(id),
            category,
            privacy: Privacy {
                private_content: private,
                network: false,
            },
        }
    }

    fn result(id: &str, category: Category, title: &str) -> SearchResult {
        let primary = match category {
            Category::Applications => Action::LaunchApplication {
                app_id: title.to_lowercase(),
                spec: rmac_apps::LaunchSpec::OpenPath("/Applications/Test.app".into()),
            },
            Category::Settings => Action::OpenSetting {
                pane_id: title.to_lowercase(),
            },
            Category::Files => Action::OpenFile {
                path: "/home/alex/report.txt".into(),
            },
            Category::Calculator | Category::Other => Action::CopyText { text: title.into() },
        };
        SearchResult {
            id: ResultId {
                provider: provider_id(id),
                local: title.to_lowercase(),
            },
            category,
            title: title.into(),
            subtitle: None,
            icon: None,
            primary,
            alternate: (category == Category::Files)
                .then(|| Action::RevealFile {
                    path: "/home/alex/report.txt".into(),
                })
                .or_else(|| {
                    (category == Category::Applications).then(|| Action::RevealApplication {
                        source: "/Applications/Test.app".into(),
                    })
                }),
            recency_rank: 0,
        }
    }

    fn batch(
        request: &Request,
        id: &str,
        results: Result<Vec<SearchResult>, ProviderError>,
    ) -> Batch {
        Batch {
            generation: request.generation,
            provider: provider_id(id),
            results,
        }
    }

    fn allow_private(
        id: &str,
    ) -> BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy> {
        BTreeMap::from([(
            provider_id(id),
            rmac_shell_settings::ProviderPolicy {
                enabled: true,
                allow_private_content: true,
                allow_network: false,
            },
        )])
    }

    #[test]
    fn registry_rejects_duplicate_identity_and_dispatches_only_admitted_providers() {
        let public_calls = Arc::new(AtomicUsize::new(0));
        let private_calls = Arc::new(AtomicUsize::new(0));
        let public: Arc<dyn Provider> = Arc::new(FakeProvider {
            descriptor: descriptor("apps", Category::Applications, false),
            title: "Terminal",
            calls: public_calls.clone(),
        });
        let duplicate: Arc<dyn Provider> = Arc::new(FakeProvider {
            descriptor: descriptor("apps", Category::Settings, false),
            title: "Settings",
            calls: Arc::new(AtomicUsize::new(0)),
        });
        assert!(Registry::new(vec![public.clone(), duplicate]).is_err());

        let private: Arc<dyn Provider> = Arc::new(FakeProvider {
            descriptor: descriptor("files", Category::Files, true),
            title: "Report.txt",
            calls: private_calls.clone(),
        });
        let registry = Registry::new(vec![public, private]).expect("registry is valid");
        let mut coordinator = Coordinator::new(registry.descriptors(), BTreeMap::new());
        let request = coordinator.open().request;
        let (sender, receiver) = async_channel::unbounded();
        futures_lite::future::block_on(registry.dispatch(request, sender));
        let batches: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(public_calls.load(Ordering::Acquire), 1);
        assert_eq!(private_calls.load(Ordering::Acquire), 0);

        let closed_request = coordinator
            .set_query("terminal")
            .expect("overlay remains open");
        let cancellation = closed_request.cancellation.clone();
        let (sender, receiver) = async_channel::bounded(1);
        drop(receiver);
        futures_lite::future::block_on(registry.dispatch(closed_request, sender));
        assert!(cancellation.is_cancelled());
        assert_eq!(public_calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn changed_provider_identity_finishes_as_an_error_instead_of_loading_forever() {
        let registry = Registry::new(vec![Arc::new(ChangingProvider {
            descriptor_calls: AtomicUsize::new(0),
        })])
        .expect("initial descriptor is valid");
        let mut coordinator = Coordinator::new(registry.descriptors(), BTreeMap::new());
        let request = coordinator.open().request;
        let (sender, receiver) = async_channel::bounded(1);
        futures_lite::future::block_on(registry.dispatch(request, sender));
        let batch = receiver
            .try_recv()
            .expect("descriptor failure is published");
        assert!(coordinator.apply(batch));
        assert_eq!(coordinator.snapshot().phase, Phase::Unavailable);
    }

    #[test]
    fn open_focuses_before_search_and_new_queries_reject_stale_batches() {
        let descriptors = vec![descriptor("apps", Category::Applications, false)];
        let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
        let opened = coordinator.open();
        assert_eq!(opened.focus, FocusTarget::Query);
        assert_eq!(coordinator.snapshot().phase, Phase::Loading);
        assert_eq!(
            coordinator.handle_key(KeyCommand::ArrowDown),
            KeyEffect::None
        );
        let newer = coordinator.set_query("term").expect("overlay is open");
        assert!(!coordinator.apply(batch(
            &opened.request,
            "apps",
            Ok(vec![result("apps", Category::Applications, "Old")]),
        )));
        assert!(coordinator.apply(batch(
            &newer,
            "apps",
            Ok(vec![result("apps", Category::Applications, "Terminal")]),
        )));
        assert_eq!(coordinator.snapshot().rows[0].title, "Terminal");
    }

    #[test]
    fn launcher_shortcut_toggles_once_and_ignores_replays_and_other_ids() {
        let mut coordinator = Coordinator::new(
            vec![descriptor("apps", Category::Applications, false)],
            BTreeMap::new(),
        );
        let other = rmac_shortcuts::Event::Activated {
            id: rmac_shortcuts::ShortcutId("notification-center".into()),
            timestamp_ms: 9,
        };
        assert!(matches!(
            coordinator.handle_shortcut(&other),
            ShortcutEffect::None
        ));

        let open = rmac_shortcuts::Event::Activated {
            id: rmac_shortcuts::ShortcutId("launcher".into()),
            timestamp_ms: 10,
        };
        let ShortcutEffect::Open(effect) = coordinator.handle_shortcut(&open) else {
            panic!("fresh launcher shortcut opens");
        };
        assert_eq!(effect.focus, FocusTarget::Query);
        assert!(coordinator.snapshot().open);
        assert!(matches!(
            coordinator.handle_shortcut(&open),
            ShortcutEffect::None
        ));
        assert!(coordinator.snapshot().open);

        let close = rmac_shortcuts::Event::Activated {
            id: rmac_shortcuts::ShortcutId("launcher".into()),
            timestamp_ms: 11,
        };
        assert!(matches!(
            coordinator.handle_shortcut(&close),
            ShortcutEffect::Dismissed
        ));
        assert_eq!(coordinator.snapshot().phase, Phase::Closed);
        assert!(matches!(
            coordinator.handle_shortcut(&rmac_shortcuts::Event::Deactivated {
                id: rmac_shortcuts::ShortcutId("launcher".into()),
                timestamp_ms: 12,
            }),
            ShortcutEffect::None
        ));
    }

    #[test]
    fn keyboard_journey_announces_category_and_uses_exact_alternate_action() {
        let descriptors = vec![descriptor("files", Category::Files, true)];
        let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
        let request = coordinator.open().request;
        coordinator.apply(batch(
            &request,
            "files",
            Ok(vec![
                result("files", Category::Files, "Alpha.txt"),
                result("files", Category::Files, "Beta.txt"),
            ]),
        ));
        assert_eq!(
            coordinator.snapshot().announcement.as_deref(),
            Some("Alpha.txt, Files, 1 of 2 results")
        );
        assert_eq!(
            coordinator.handle_key(KeyCommand::ArrowDown),
            KeyEffect::SelectionChanged
        );
        assert_eq!(
            coordinator.snapshot().announcement.as_deref(),
            Some("Beta.txt, Files, 2 of 2 results")
        );
        let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::AlternateReturn)
        else {
            panic!("alternate return activates");
        };
        assert!(matches!(activation.action, Action::RevealFile { .. }));
        assert_eq!(coordinator.snapshot().phase, Phase::Activating);
        assert_eq!(coordinator.handle_key(KeyCommand::Return), KeyEffect::None);
        assert!(coordinator.set_query("ignored while opening").is_none());
        assert!(
            coordinator.finish_activation(Ok(rmac_launcher_system::Receipt {
                activation: activation.id,
                outcome: rmac_launcher_system::Outcome::FileRevealed,
            }))
        );
        assert_eq!(coordinator.snapshot().phase, Phase::Closed);
    }

    #[test]
    fn progressive_results_expose_searching_and_degraded_state_without_flicker() {
        let descriptors = vec![
            descriptor("apps", Category::Applications, false),
            descriptor("settings", Category::Settings, false),
        ];
        let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
        let request = coordinator.open().request;
        assert!(coordinator.apply(batch(
            &request,
            "apps",
            Ok(vec![result("apps", Category::Applications, "Terminal",)]),
        )));
        assert_eq!(
            coordinator.snapshot().phase,
            Phase::Results {
                still_searching: true,
                degraded: false,
            }
        );
        assert!(coordinator.apply(batch(
            &request,
            "settings",
            Err(ProviderError {
                detail: "settings service unavailable".into(),
            }),
        )));
        let snapshot = coordinator.snapshot();
        assert_eq!(
            snapshot.phase,
            Phase::Results {
                still_searching: false,
                degraded: true,
            }
        );
        assert_eq!(snapshot.rows[0].title, "Terminal");
        assert_eq!(snapshot.failed_providers, 1);
    }

    #[test]
    fn catalog_revisions_restart_open_search_once_and_health_retains_results() {
        let descriptors = vec![descriptor("apps", Category::Applications, false)];
        let mut coordinator = Coordinator::new(descriptors, BTreeMap::new());
        let original = coordinator.open().request;
        let starting = coordinator.apply_catalog(CatalogUpdate {
            health: CatalogHealth::Starting,
            revision: 0,
            changed: false,
            detail: None,
        });
        assert!(starting.visible);
        assert!(starting.request.is_none());

        let refreshed = coordinator.apply_catalog(CatalogUpdate {
            health: CatalogHealth::Healthy,
            revision: 1,
            changed: true,
            detail: None,
        });
        let refreshed = refreshed.request.expect("new revision reissues query");
        assert!(original.cancellation.is_cancelled());
        assert!(
            !coordinator
                .apply_catalog(CatalogUpdate {
                    health: CatalogHealth::Healthy,
                    revision: 1,
                    changed: true,
                    detail: None,
                })
                .visible
        );
        assert!(coordinator.apply(batch(
            &refreshed,
            "apps",
            Ok(vec![result("apps", Category::Applications, "Terminal",)]),
        )));

        let unavailable = coordinator.apply_catalog(CatalogUpdate {
            health: CatalogHealth::Unavailable,
            revision: 1,
            changed: false,
            detail: Some("private catalog path detail".into()),
        });
        assert!(unavailable.visible);
        assert!(unavailable.request.is_none());
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.application_catalog, CatalogHealth::Unavailable);
        assert_eq!(snapshot.rows[0].title, "Terminal");
        assert_eq!(
            snapshot.phase,
            Phase::Results {
                still_searching: false,
                degraded: true,
            }
        );
        assert!(!snapshot
            .announcement
            .expect("selection is announced")
            .contains("private catalog"));

        let stale = coordinator.apply_catalog(CatalogUpdate {
            health: CatalogHealth::Starting,
            revision: 0,
            changed: true,
            detail: None,
        });
        assert!(!stale.visible);
        assert_eq!(
            coordinator.snapshot().application_catalog,
            CatalogHealth::Unavailable
        );
    }

    #[test]
    fn provider_and_activation_failures_are_truthful_but_private_safe() {
        let descriptors = vec![descriptor("files", Category::Files, true)];
        let mut coordinator = Coordinator::new(descriptors, allow_private("files"));
        let request = coordinator.open().request;
        coordinator.apply(batch(
            &request,
            "files",
            Err(ProviderError {
                detail: "secret /home/alex/report.txt".into(),
            }),
        ));
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.phase, Phase::Unavailable);
        assert_eq!(
            snapshot.announcement.as_deref(),
            Some("Search providers are unavailable")
        );
        assert!(!snapshot.announcement.unwrap().contains("alex"));

        let request = coordinator.set_query("report").expect("overlay open");
        coordinator.apply(batch(
            &request,
            "files",
            Ok(vec![result("files", Category::Files, "Report.txt")]),
        ));
        let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
            panic!("return activates");
        };
        let failure = rmac_launcher_system::BackendError::new(
            rmac_launcher_system::FailureKind::Rejected,
            "secret /home/alex/report.txt",
        );
        let error = futures_lite::future::block_on(rmac_launcher_system::execute(
            activation.id,
            &activation.action,
            &RejectingBackend(failure),
        ))
        .expect_err("activation fails");
        assert!(coordinator.finish_activation(Err(error)));
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.phase, Phase::ActivationFailed);
        assert_eq!(
            snapshot.announcement.as_deref(),
            Some("Could not open the file")
        );
        assert!(!snapshot.announcement.unwrap().contains("alex"));
    }

    #[test]
    fn policy_change_cancels_private_work_and_escape_ignores_late_activation() {
        let files = descriptor("files", Category::Files, true);
        let mut policies = BTreeMap::new();
        policies.insert(
            files.id.clone(),
            rmac_shell_settings::ProviderPolicy {
                enabled: true,
                allow_private_content: true,
                allow_network: false,
            },
        );
        let mut coordinator = Coordinator::new(vec![files.clone()], policies);
        let original = coordinator.open().request;
        let replacement = coordinator
            .set_policies(BTreeMap::new())
            .expect("open overlay restarts search");
        assert!(original.cancellation.is_cancelled());
        assert!(replacement.providers.is_empty());
        assert_eq!(coordinator.snapshot().phase, Phase::Empty);

        let request = coordinator
            .set_policies({
                let mut policies = BTreeMap::new();
                policies.insert(
                    files.id,
                    rmac_shell_settings::ProviderPolicy {
                        enabled: true,
                        allow_private_content: true,
                        allow_network: false,
                    },
                );
                policies
            })
            .expect("provider is readmitted");
        coordinator.apply(batch(
            &request,
            "files",
            Ok(vec![result("files", Category::Files, "Report.txt")]),
        ));
        let KeyEffect::Activate(activation) = coordinator.handle_key(KeyCommand::Return) else {
            panic!("return activates");
        };
        assert_eq!(
            coordinator.handle_key(KeyCommand::Escape),
            KeyEffect::Dismissed
        );
        assert!(
            !coordinator.finish_activation(Ok(rmac_launcher_system::Receipt {
                activation: activation.id,
                outcome: rmac_launcher_system::Outcome::SettingOpened,
            }))
        );
        assert_eq!(coordinator.snapshot().phase, Phase::Closed);
    }

    #[test]
    fn environment_change_reissues_open_query_with_new_descriptors() {
        let apps = descriptor("apps", Category::Applications, false);
        let mut coordinator = Coordinator::new(vec![apps], BTreeMap::new());
        let original = coordinator.open().request;
        let settings = descriptor("settings", Category::Settings, false);
        let replacement = coordinator
            .set_environment(vec![settings.clone()], BTreeMap::new())
            .expect("changed provider environment restarts the open query");
        assert!(original.cancellation.is_cancelled());
        assert_eq!(replacement.providers, [settings]);
        assert!(coordinator
            .set_environment(replacement.providers.clone(), BTreeMap::new())
            .is_none());
    }

    struct RejectingBackend(rmac_launcher_system::BackendError);

    impl rmac_launcher_system::Backend for RejectingBackend {
        fn launch<'a>(
            &'a self,
            _: &'a rmac_apps::LaunchSpec,
        ) -> rmac_launcher_system::BackendFuture<'a, Result<u32, rmac_launcher_system::BackendError>>
        {
            Box::pin(async { Err(self.0.clone()) })
        }

        fn open_setting<'a>(
            &'a self,
            _: &'a str,
        ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
        {
            Box::pin(async { Err(self.0.clone()) })
        }

        fn open_file<'a>(
            &'a self,
            _: &'a std::path::Path,
        ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
        {
            Box::pin(async { Err(self.0.clone()) })
        }

        fn reveal_file<'a>(
            &'a self,
            _: &'a std::path::Path,
        ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
        {
            Box::pin(async { Err(self.0.clone()) })
        }

        fn copy_text<'a>(
            &'a self,
            _: &'a str,
        ) -> rmac_launcher_system::BackendFuture<'a, Result<(), rmac_launcher_system::BackendError>>
        {
            Box::pin(async { Err(self.0.clone()) })
        }
    }
}
