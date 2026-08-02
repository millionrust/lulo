//! Bounded launcher provider registry and concurrent query authority.

use super::*;

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

pub(super) struct RegisteredProvider {
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
