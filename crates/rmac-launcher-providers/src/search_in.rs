//! The "Search in Files" row that closes every query's list.
//!
//! macOS 26.2 ends each list with "Search iCloud Drive", "Search in
//! Finder", "Search the Web" and "Ask Siri" (and shows only those when
//! nothing matches). rmac has Files and none of the others, so it keeps the
//! one row: Return opens Files searching the home folder for the query.

use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct SearchInFilesProvider;

pub const SEARCH_IN_FILES_TITLE: &str = "Search in Files";

impl Provider for SearchInFilesProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(SEARCH_IN_PROVIDER, Category::SearchIn, Privacy::default())
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![SearchResult {
            id: ResultId {
                provider: provider_id(SEARCH_IN_PROVIDER),
                local: "files".into(),
            },
            category: Category::SearchIn,
            application_group: None,
            title: SEARCH_IN_FILES_TITLE.into(),
            subtitle: None,
            detail: None,
            icon: None,
            primary: Action::SearchFiles {
                query: query.to_owned(),
            },
            alternate: None,
            recency_rank: 0,
        }])
    }
}
