//! File names from the Windows Search index (Spotlight on Windows).
//!
//! Walking the user's folder for every typed letter took about 10 s on the
//! owner's Ryzen 3 laptop and kept a core busy; the index Windows already
//! keeps for the user's folders answers the same name query in 50 to 120 ms
//! (measured there through the same WinRT query). The index is read
//! through `StorageFolder` queries with `IndexerOption::OnlyUseIndexer`, so
//! nothing walks the disk. When the index cannot answer (the Windows
//! Search service is off, or the folder is not indexed) the caller falls
//! back to the walk.

use std::path::{Path, PathBuf};

use windows::core::HSTRING;
use windows::Storage::Search::{FolderDepth, IndexerOption, QueryOptions};
use windows::Storage::StorageFolder;

/// Advanced Query Syntax for "the file's name contains `query`", with the
/// characters AQS gives meaning to inside a quoted value left out.
pub(crate) fn name_filter(query: &str) -> Option<String> {
    let cleaned: String = query
        .chars()
        .filter(|character| *character != '"' && !character.is_control())
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| format!("System.FileName:~=\"{cleaned}\""))
}

/// Up to `limit` indexed items under `root` whose name contains `query`;
/// `None` when the index cannot answer.
pub(crate) fn filenames(root: &Path, query: &str, limit: usize) -> Option<Vec<PathBuf>> {
    let Some(filter) = name_filter(query) else {
        return Some(Vec::new());
    };
    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(root.as_os_str()))
        .ok()?
        .get()
        .ok()?;
    let options = QueryOptions::new().ok()?;
    options.SetFolderDepth(FolderDepth::Deep).ok()?;
    options
        .SetIndexerOption(IndexerOption::OnlyUseIndexer)
        .ok()?;
    options.SetUserSearchFilter(&HSTRING::from(filter)).ok()?;
    let items = folder
        .CreateItemQueryWithOptions(&options)
        .ok()?
        .GetItemsAsync(0, u32::try_from(limit).unwrap_or(u32::MAX))
        .ok()?
        .get()
        .ok()?;
    Some(
        items
            .into_iter()
            .filter_map(|item| item.Path().ok())
            .map(|path| PathBuf::from(path.to_os_string()))
            .collect(),
    )
}
