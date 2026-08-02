//! Stable launcher provider execution contract and batch model.

use super::*;

pub trait Provider: Send + Sync + 'static {
    fn descriptor(&self) -> ProviderDescriptor;
    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Batch {
    pub generation: u64,
    pub provider: rmac_shell_settings::ProviderId,
    pub results: Result<Vec<SearchResult>, ProviderError>,
}

/// Run one provider only if its exact descriptor was admitted into this
/// privacy-filtered request. Call this function on a background executor.
pub fn execute(request: &Request, provider: &(impl Provider + ?Sized)) -> Option<Batch> {
    let descriptor = provider.descriptor();
    if request.cancellation.is_cancelled()
        || !request
            .providers
            .iter()
            .any(|admitted| admitted == &descriptor)
    {
        return None;
    }
    let results = provider.search(&request.query, &request.cancellation);
    (!request.cancellation.is_cancelled()).then_some(Batch {
        generation: request.generation,
        provider: descriptor.id,
        results,
    })
}
