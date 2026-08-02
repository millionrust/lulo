//! Shared provider descriptors and cancellation errors.

use super::*;

pub(super) fn descriptor(id: &str, category: Category, privacy: Privacy) -> ProviderDescriptor {
    ProviderDescriptor {
        id: provider_id(id),
        category,
        privacy,
    }
}

pub(super) fn provider_id(id: &str) -> rmac_shell_settings::ProviderId {
    rmac_shell_settings::ProviderId(id.into())
}

pub(super) fn cancelled() -> ProviderError {
    ProviderError {
        detail: "provider cancelled".into(),
    }
}
