use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmac_notes_store::{AttachmentId, LibrarySnapshot, NoteId, ValidationError, MAX_LIBRARY_BYTES};

pub const SEARCH_INDEX_VERSION: u16 = 1;
pub const MAX_SEARCH_QUERY_BYTES: usize = 1024;
pub const MAX_SEARCH_RESULTS: usize = 500;
pub const MAX_SEARCH_MATCHES_PER_RESULT: usize = 16;
pub const MAX_SEARCH_INDEX_TEXT_BYTES: usize = MAX_LIBRARY_BYTES * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SearchGeneration(u64);

impl SearchGeneration {
    pub fn new(value: u64) -> Option<Self> {
        (value != 0).then_some(Self(value))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Default)]
pub struct SearchCancellation {
    cancelled: Arc<AtomicBool>,
}

impl SearchCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl fmt::Debug for SearchCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone)]
pub struct SearchRequest {
    generation: SearchGeneration,
    library_revision: u64,
    query: String,
    limit: usize,
    cancellation: SearchCancellation,
}

impl SearchRequest {
    pub fn new(
        generation: SearchGeneration,
        library_revision: u64,
        query: impl Into<String>,
        limit: usize,
        cancellation: SearchCancellation,
    ) -> Result<Self, SearchError> {
        let query = query.into();
        let query = query.trim();
        if query.is_empty()
            || query.len() > MAX_SEARCH_QUERY_BYTES
            || query.chars().any(char::is_control)
            || limit == 0
            || limit > MAX_SEARCH_RESULTS
        {
            return Err(SearchError::InvalidRequest);
        }
        Ok(Self {
            generation,
            library_revision,
            query: query.to_string(),
            limit,
            cancellation,
        })
    }

    pub fn generation(&self) -> SearchGeneration {
        self.generation
    }

    pub fn library_revision(&self) -> u64 {
        self.library_revision
    }

    pub fn cancellation(&self) -> &SearchCancellation {
        &self.cancellation
    }
}

impl fmt::Debug for SearchRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchRequest")
            .field("generation", &self.generation)
            .field("library_revision", &self.library_revision)
            .field("query", &"[private]")
            .field("limit", &self.limit)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SearchRank {
    ExactTitle,
    TitlePrefix,
    Tag,
    TitleContains,
    Body,
    AttachmentName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchField {
    Title,
    Tag { index: usize },
    Body,
    AttachmentName { attachment_id: AttachmentId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSpan {
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub field: SearchField,
    pub span: TextSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub note_id: NoteId,
    pub rank: SearchRank,
    pub matches: Vec<SearchMatch>,
    pub matches_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchBatch {
    pub generation: SearchGeneration,
    pub library_revision: u64,
    pub hits: Vec<SearchHit>,
    pub results_truncated: bool,
    pub work_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchError {
    InvalidRequest,
    InvalidSnapshot(ValidationError),
    IndexTooLarge,
    StaleIndex,
    Cancelled,
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "the Notes search request is invalid",
            Self::InvalidSnapshot(_) => "the Notes search source is invalid",
            Self::IndexTooLarge => "the Notes search index exceeds its memory safety bound",
            Self::StaleIndex => "the Notes search index is stale",
            Self::Cancelled => "the Notes search was cancelled",
        })
    }
}

impl std::error::Error for SearchError {}

struct AttachmentText {
    id: AttachmentId,
    snapshot_index: usize,
    normalized_name: String,
}

struct IndexEntry {
    note_index: usize,
    note_id: NoteId,
    modified_unix_ms: u64,
    normalized_title: String,
    normalized_body: String,
    normalized_tags: Vec<String>,
    attachments: Vec<AttachmentText>,
}

pub struct NotesSearchIndex {
    version: u16,
    snapshot: Arc<LibrarySnapshot>,
    entries: Vec<IndexEntry>,
    indexed_text_bytes: usize,
}

impl NotesSearchIndex {
    pub fn build(snapshot: Arc<LibrarySnapshot>) -> Result<Self, SearchError> {
        Self::build_cancellable(snapshot, &SearchCancellation::default())
    }

    pub fn build_cancellable(
        snapshot: Arc<LibrarySnapshot>,
        cancellation: &SearchCancellation,
    ) -> Result<Self, SearchError> {
        require_active(cancellation)?;
        snapshot.validate().map_err(SearchError::InvalidSnapshot)?;
        let live_note_ids = snapshot
            .notes
            .iter()
            .filter(|note| !note.deleted)
            .map(|note| note.id)
            .collect::<BTreeSet<_>>();
        let mut attachments = BTreeMap::<NoteId, Vec<AttachmentText>>::new();
        let mut indexed_text_bytes = 0_usize;
        for (snapshot_index, attachment) in snapshot.attachments.iter().enumerate() {
            require_active(cancellation)?;
            if attachment.deleted || !live_note_ids.contains(&attachment.note_id) {
                continue;
            }
            let normalized_name = normalize_cancellable(
                &attachment.display_name,
                cancellation,
                remaining_index_capacity(indexed_text_bytes),
            )?;
            add_index_bytes(&mut indexed_text_bytes, normalized_name.len())?;
            attachments
                .entry(attachment.note_id)
                .or_default()
                .push(AttachmentText {
                    id: attachment.id,
                    snapshot_index,
                    normalized_name,
                });
        }
        for values in attachments.values_mut() {
            values.sort_by_key(|attachment| attachment.id);
        }

        let mut entries = Vec::new();
        for (note_index, note) in snapshot.notes.iter().enumerate() {
            require_active(cancellation)?;
            if note.deleted {
                continue;
            }
            let normalized_title = normalize_cancellable(
                &note.title,
                cancellation,
                remaining_index_capacity(indexed_text_bytes),
            )?;
            add_index_bytes(&mut indexed_text_bytes, normalized_title.len())?;
            let normalized_body = normalize_cancellable(
                &note.body,
                cancellation,
                remaining_index_capacity(indexed_text_bytes),
            )?;
            add_index_bytes(&mut indexed_text_bytes, normalized_body.len())?;
            let mut normalized_tags = Vec::with_capacity(note.tags.len());
            for tag in &note.tags {
                let normalized_tag = normalize_cancellable(
                    tag,
                    cancellation,
                    remaining_index_capacity(indexed_text_bytes),
                )?;
                add_index_bytes(&mut indexed_text_bytes, normalized_tag.len())?;
                normalized_tags.push(normalized_tag);
            }
            entries.push(IndexEntry {
                note_index,
                note_id: note.id,
                modified_unix_ms: note.modified_unix_ms,
                normalized_title,
                normalized_body,
                normalized_tags,
                attachments: attachments.remove(&note.id).unwrap_or_default(),
            });
        }
        require_active(cancellation)?;
        entries.sort_by_key(|entry| entry.note_id);
        Ok(Self {
            version: SEARCH_INDEX_VERSION,
            snapshot,
            entries,
            indexed_text_bytes,
        })
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn library_revision(&self) -> u64 {
        self.snapshot.revision
    }

    pub fn indexed_text_bytes(&self) -> usize {
        self.indexed_text_bytes
    }

    pub fn search(&self, request: &SearchRequest) -> Result<SearchBatch, SearchError> {
        require_active(&request.cancellation)?;
        if request.library_revision != self.snapshot.revision {
            return Err(SearchError::StaleIndex);
        }
        let query = normalize(&request.query);
        if query.is_empty() || query.len() > MAX_SEARCH_QUERY_BYTES.saturating_mul(3) {
            return Err(SearchError::InvalidRequest);
        }
        let mut ranked = Vec::<(SearchHit, u64)>::with_capacity(request.limit);
        let mut total_matches = 0_usize;
        let mut work_bytes = 0_usize;
        for entry in &self.entries {
            require_active(&request.cancellation)?;
            let note = &self.snapshot.notes[entry.note_index];
            let mut matches = Vec::new();
            let mut matches_truncated = false;
            let mut best = None;

            add_work(&mut work_bytes, entry.normalized_title.len())?;
            if let Some(position) =
                cancellable_find(&entry.normalized_title, &query, &request.cancellation)?
            {
                let rank = if entry.normalized_title == query {
                    SearchRank::ExactTitle
                } else if position == 0 {
                    SearchRank::TitlePrefix
                } else {
                    SearchRank::TitleContains
                };
                best = Some(rank);
                push_match(
                    &mut matches,
                    &mut matches_truncated,
                    SearchField::Title,
                    mapped_span(&note.title, position, query.len()),
                );
            }

            for (index, normalized_tag) in entry.normalized_tags.iter().enumerate() {
                require_active(&request.cancellation)?;
                add_work(&mut work_bytes, normalized_tag.len())?;
                if let Some(position) =
                    cancellable_find(normalized_tag, &query, &request.cancellation)?
                {
                    best = Some(best.map_or(SearchRank::Tag, |rank| rank.min(SearchRank::Tag)));
                    push_match(
                        &mut matches,
                        &mut matches_truncated,
                        SearchField::Tag { index },
                        mapped_span(&note.tags[index], position, query.len()),
                    );
                }
            }

            add_work(&mut work_bytes, entry.normalized_body.len())?;
            if let Some(position) =
                cancellable_find(&entry.normalized_body, &query, &request.cancellation)?
            {
                best = Some(best.map_or(SearchRank::Body, |rank| rank.min(SearchRank::Body)));
                push_match(
                    &mut matches,
                    &mut matches_truncated,
                    SearchField::Body,
                    mapped_span(&note.body, position, query.len()),
                );
            }

            for attachment in &entry.attachments {
                require_active(&request.cancellation)?;
                add_work(&mut work_bytes, attachment.normalized_name.len())?;
                if let Some(position) =
                    cancellable_find(&attachment.normalized_name, &query, &request.cancellation)?
                {
                    best = Some(best.map_or(SearchRank::AttachmentName, |rank| {
                        rank.min(SearchRank::AttachmentName)
                    }));
                    let original =
                        &self.snapshot.attachments[attachment.snapshot_index].display_name;
                    push_match(
                        &mut matches,
                        &mut matches_truncated,
                        SearchField::AttachmentName {
                            attachment_id: attachment.id,
                        },
                        mapped_span(original, position, query.len()),
                    );
                }
            }

            if let Some(rank) = best {
                total_matches = total_matches.saturating_add(1);
                let candidate = (
                    SearchHit {
                        note_id: entry.note_id,
                        rank,
                        matches,
                        matches_truncated,
                    },
                    entry.modified_unix_ms,
                );
                let position = ranked
                    .binary_search_by(|existing| compare_ranked(existing, &candidate))
                    .unwrap_or_else(|position| position);
                if ranked.len() < request.limit {
                    ranked.insert(position, candidate);
                } else if position < request.limit {
                    ranked.pop();
                    ranked.insert(position, candidate);
                }
            }
        }
        require_active(&request.cancellation)?;
        let results_truncated = total_matches > request.limit;
        let hits = ranked.into_iter().map(|(hit, _)| hit).collect();
        Ok(SearchBatch {
            generation: request.generation,
            library_revision: self.snapshot.revision,
            hits,
            results_truncated,
            work_bytes,
        })
    }
}

impl fmt::Debug for NotesSearchIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesSearchIndex")
            .field("version", &self.version)
            .field("library_revision", &self.snapshot.revision)
            .field("entries", &self.entries.len())
            .field("indexed_text_bytes", &self.indexed_text_bytes)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchState {
    #[default]
    Empty,
    Indexing,
    Results,
    NoMatches,
    Unavailable,
}

pub struct NotesSearchSession {
    generation: u64,
    expected_library_revision: Option<u64>,
    state: SearchState,
    cancellation: Option<SearchCancellation>,
    hits: Vec<SearchHit>,
    selected: Option<NoteId>,
    selection_anchor: Option<NoteId>,
    results_truncated: bool,
    failure: Option<SearchError>,
}

impl NotesSearchSession {
    pub fn new() -> Self {
        Self {
            generation: 0,
            expected_library_revision: None,
            state: SearchState::Empty,
            cancellation: None,
            hits: Vec::new(),
            selected: None,
            selection_anchor: None,
            results_truncated: false,
            failure: None,
        }
    }

    pub fn begin(
        &mut self,
        query: impl Into<String>,
        limit: usize,
        library_revision: u64,
    ) -> Result<Option<SearchRequest>, SearchError> {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        self.expected_library_revision = Some(library_revision);
        self.hits.clear();
        self.selected = None;
        self.results_truncated = false;
        self.failure = None;
        let query = query.into();
        if query.trim().is_empty() {
            self.state = SearchState::Empty;
            self.expected_library_revision = None;
            self.selection_anchor = None;
            return Ok(None);
        }
        let cancellation = SearchCancellation::default();
        let request = match SearchRequest::new(
            SearchGeneration::new(self.generation).expect("generation is nonzero"),
            library_revision,
            query,
            limit,
            cancellation.clone(),
        ) {
            Ok(request) => request,
            Err(error) => {
                self.expected_library_revision = None;
                self.selection_anchor = None;
                self.failure = Some(error);
                self.state = SearchState::Unavailable;
                return Err(error);
            }
        };
        self.cancellation = Some(cancellation);
        self.state = SearchState::Indexing;
        Ok(Some(request))
    }

    pub fn apply(&mut self, batch: SearchBatch) -> bool {
        if batch.generation.get() != self.generation
            || Some(batch.library_revision) != self.expected_library_revision
            || self
                .cancellation
                .as_ref()
                .is_none_or(SearchCancellation::is_cancelled)
        {
            return false;
        }
        self.hits = batch.hits;
        self.results_truncated = batch.results_truncated;
        self.selected = self
            .selection_anchor
            .filter(|note_id| self.hits.iter().any(|hit| hit.note_id == *note_id))
            .or_else(|| self.hits.first().map(|hit| hit.note_id));
        self.selection_anchor = self.selected;
        self.cancellation.take();
        self.expected_library_revision = None;
        self.state = if self.hits.is_empty() {
            SearchState::NoMatches
        } else {
            SearchState::Results
        };
        true
    }

    pub fn fail(
        &mut self,
        generation: SearchGeneration,
        library_revision: u64,
        error: SearchError,
    ) -> bool {
        if generation.get() != self.generation
            || Some(library_revision) != self.expected_library_revision
            || self
                .cancellation
                .as_ref()
                .is_none_or(SearchCancellation::is_cancelled)
        {
            return false;
        }
        self.cancellation.take();
        self.hits.clear();
        self.selected = None;
        self.results_truncated = false;
        self.failure = Some(error);
        self.expected_library_revision = None;
        self.selection_anchor = None;
        self.state = SearchState::Unavailable;
        true
    }

    pub(crate) fn is_pending(&self, generation: SearchGeneration, library_revision: u64) -> bool {
        generation.get() == self.generation
            && Some(library_revision) == self.expected_library_revision
            && self
                .cancellation
                .as_ref()
                .is_some_and(|cancellation| !cancellation.is_cancelled())
    }

    pub fn cancel(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.state = SearchState::Empty;
        self.expected_library_revision = None;
        self.hits.clear();
        self.selected = None;
        self.selection_anchor = None;
        self.results_truncated = false;
        self.failure = None;
    }

    pub fn state(&self) -> SearchState {
        self.state
    }

    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    pub fn selected(&self) -> Option<NoteId> {
        self.selected
    }

    pub fn select(&mut self, note_id: NoteId) -> bool {
        if !self.hits.iter().any(|hit| hit.note_id == note_id) {
            return false;
        }
        self.selected = Some(note_id);
        self.selection_anchor = Some(note_id);
        true
    }

    pub fn results_truncated(&self) -> bool {
        self.results_truncated
    }

    pub fn failure(&self) -> Option<SearchError> {
        self.failure
    }
}

impl Default for NotesSearchSession {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NotesSearchSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotesSearchSession")
            .field("generation", &self.generation)
            .field("state", &self.state)
            .field("hit_count", &self.hits.len())
            .field("selected", &self.selected)
            .field("results_truncated", &self.results_truncated)
            .field("failure", &self.failure)
            .finish()
    }
}

fn normalize(value: &str) -> String {
    value.to_lowercase()
}

fn normalize_cancellable(
    value: &str,
    cancellation: &SearchCancellation,
    max_bytes: usize,
) -> Result<String, SearchError> {
    const CHUNK_BYTES: usize = 64 * 1024;
    let mut normalized = String::with_capacity(value.len().min(max_bytes));
    let mut since_check = 0_usize;
    for character in value.chars() {
        if since_check >= CHUNK_BYTES {
            require_active(cancellation)?;
            since_check = 0;
        }
        since_check = since_check.saturating_add(character.len_utf8());
        for lowercase in character.to_lowercase() {
            if normalized
                .len()
                .checked_add(lowercase.len_utf8())
                .is_none_or(|length| length > max_bytes)
            {
                return Err(SearchError::IndexTooLarge);
            }
            normalized.push(lowercase);
        }
    }
    require_active(cancellation)?;
    Ok(normalized)
}

fn remaining_index_capacity(indexed_text_bytes: usize) -> usize {
    MAX_SEARCH_INDEX_TEXT_BYTES.saturating_sub(indexed_text_bytes)
}

fn compare_ranked(left: &(SearchHit, u64), right: &(SearchHit, u64)) -> std::cmp::Ordering {
    left.0
        .rank
        .cmp(&right.0.rank)
        .then_with(|| right.1.cmp(&left.1))
        .then_with(|| left.0.note_id.cmp(&right.0.note_id))
}

fn add_index_bytes(total: &mut usize, length: usize) -> Result<(), SearchError> {
    *total = total
        .checked_add(length)
        .ok_or(SearchError::IndexTooLarge)?;
    if *total > MAX_SEARCH_INDEX_TEXT_BYTES {
        return Err(SearchError::IndexTooLarge);
    }
    Ok(())
}

fn add_work(total: &mut usize, length: usize) -> Result<(), SearchError> {
    *total = total
        .checked_add(length)
        .ok_or(SearchError::IndexTooLarge)?;
    if *total > MAX_SEARCH_INDEX_TEXT_BYTES {
        return Err(SearchError::IndexTooLarge);
    }
    Ok(())
}

fn require_active(cancellation: &SearchCancellation) -> Result<(), SearchError> {
    if cancellation.is_cancelled() {
        Err(SearchError::Cancelled)
    } else {
        Ok(())
    }
}

fn cancellable_find(
    haystack: &str,
    needle: &str,
    cancellation: &SearchCancellation,
) -> Result<Option<usize>, SearchError> {
    const CHUNK_BYTES: usize = 64 * 1024;
    if needle.is_empty() {
        return Ok(Some(0));
    }
    let mut start = 0_usize;
    while start < haystack.len() {
        require_active(cancellation)?;
        let mut end = start.saturating_add(CHUNK_BYTES).min(haystack.len());
        while end < haystack.len() && !haystack.is_char_boundary(end) {
            end += 1;
        }
        let mut search_end = end
            .saturating_add(needle.len().saturating_sub(1))
            .min(haystack.len());
        while search_end < haystack.len() && !haystack.is_char_boundary(search_end) {
            search_end += 1;
        }
        if let Some(relative) = haystack[start..search_end].find(needle) {
            return Ok(Some(start + relative));
        }
        start = end;
    }
    Ok(None)
}

fn mapped_span(original: &str, normalized_start: usize, normalized_length: usize) -> TextSpan {
    let normalized_end = normalized_start.saturating_add(normalized_length);
    let mut normalized_offset = 0_usize;
    let mut original_start = original.len();
    let mut original_end = original.len();
    for (byte_start, character) in original.char_indices() {
        let byte_end = byte_start + character.len_utf8();
        let normalized_character_bytes = character
            .to_lowercase()
            .map(|character| character.len_utf8())
            .sum::<usize>();
        let next = normalized_offset.saturating_add(normalized_character_bytes);
        if original_start == original.len()
            && normalized_start >= normalized_offset
            && normalized_start < next
        {
            original_start = byte_start;
        }
        if normalized_end <= next {
            original_end = byte_end;
            break;
        }
        normalized_offset = next;
    }
    if original_start == original.len() {
        original_start = 0;
    }
    if original_end < original_start {
        original_end = original_start;
    }
    TextSpan {
        start_byte: original_start,
        end_byte: original_end,
    }
}

fn push_match(
    matches: &mut Vec<SearchMatch>,
    truncated: &mut bool,
    field: SearchField,
    span: TextSpan,
) {
    if matches.len() < MAX_SEARCH_MATCHES_PER_RESULT {
        matches.push(SearchMatch { field, span });
    } else {
        *truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_notes_store::{AttachmentKind, AttachmentRecord, FolderRecord, NoteRecord, SortOrder};

    fn note(id: u64, title: &str, body: &str, tags: &[&str], modified: u64) -> NoteRecord {
        NoteRecord {
            id: NoteId::new(id).unwrap(),
            revision: 1,
            created_unix_ms: 1,
            modified_unix_ms: modified,
            title: title.into(),
            body: body.into(),
            tags: tags.iter().map(|tag| (*tag).into()).collect(),
            folder_id: None,
            pinned: false,
            deleted: false,
            attachments: Vec::new(),
        }
    }

    fn snapshot() -> Arc<LibrarySnapshot> {
        let mut attachment_note = note(6, "Image", "Nothing here", &[], 5);
        let attachment_id = AttachmentId::new(1).unwrap();
        attachment_note.attachments.push(attachment_id);
        Arc::new(LibrarySnapshot {
            revision: 7,
            sort_order: SortOrder::Edited,
            next_note_id: 8,
            next_folder_id: 1,
            next_attachment_id: 2,
            folders: Vec::<FolderRecord>::new(),
            notes: vec![
                note(1, "Roadmap", "body", &[], 10),
                note(2, "Roadmap next", "body", &[], 20),
                note(3, "Planning", "body", &["roadmap"], 30),
                note(4, "Reference", "the roadmap appears here", &[], 40),
                note(5, "My roadmap archive", "body", &[], 50),
                attachment_note,
                {
                    let mut deleted = note(7, "Roadmap deleted", "body", &[], 100);
                    deleted.deleted = true;
                    deleted
                },
            ],
            attachments: vec![AttachmentRecord {
                id: attachment_id,
                revision: 1,
                note_id: NoteId::new(6).unwrap(),
                display_name: "roadmap.png".into(),
                kind: AttachmentKind::Png,
                byte_len: 10,
                sha256: [1; 32],
                deleted: false,
            }],
        })
    }

    fn request(query: &str, limit: usize) -> SearchRequest {
        SearchRequest::new(
            SearchGeneration::new(1).unwrap(),
            7,
            query,
            limit,
            SearchCancellation::default(),
        )
        .unwrap()
    }

    #[test]
    fn ranking_covers_title_tag_body_and_attachment_deterministically() {
        let index = NotesSearchIndex::build(snapshot()).unwrap();

        let batch = index.search(&request("ROADMAP", 10)).unwrap();

        assert_eq!(
            batch
                .hits
                .iter()
                .map(|hit| (hit.note_id.get(), hit.rank))
                .collect::<Vec<_>>(),
            vec![
                (1, SearchRank::ExactTitle),
                (2, SearchRank::TitlePrefix),
                (3, SearchRank::Tag),
                (5, SearchRank::TitleContains),
                (4, SearchRank::Body),
                (6, SearchRank::AttachmentName),
            ]
        );
        assert!(!batch.results_truncated);
        assert!(batch.work_bytes > 0);
        assert!(batch.hits.iter().all(|hit| hit.note_id.get() != 7));

        let bounded = index.search(&request("roadmap", 2)).unwrap();
        assert_eq!(
            bounded
                .hits
                .iter()
                .map(|hit| hit.note_id.get())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(bounded.results_truncated);
    }

    #[test]
    fn unicode_case_expansion_maps_highlight_to_original_bytes() {
        let mut value = (*snapshot()).clone();
        value.notes[0].title = "İstanbul Notes".into();
        let index = NotesSearchIndex::build(Arc::new(value)).unwrap();

        let batch = index.search(&request("i\u{307}stanbul", 10)).unwrap();
        let title_match = batch.hits[0]
            .matches
            .iter()
            .find(|matched| matched.field == SearchField::Title)
            .unwrap();
        let original = "İstanbul Notes";

        assert_eq!(
            &original[title_match.span.start_byte..title_match.span.end_byte],
            "İstanbul"
        );
    }

    #[test]
    fn cancellation_and_generation_reject_late_private_results() {
        let index = NotesSearchIndex::build(snapshot()).unwrap();
        let mut session = NotesSearchSession::new();
        let first = session.begin("roadmap", 10, 7).unwrap().unwrap();
        let first_batch = index.search(&first).unwrap();
        let second = session.begin("nothing", 10, 7).unwrap().unwrap();

        assert!(first.cancellation().is_cancelled());
        assert!(!session.apply(first_batch));
        assert!(session.apply(index.search(&second).unwrap()));
        assert_eq!(session.state(), SearchState::Results);
        assert_eq!(session.selected(), NoteId::new(6));

        second.cancellation().cancel();
        assert_eq!(index.search(&second), Err(SearchError::Cancelled));
        session.cancel();
        assert_eq!(session.state(), SearchState::Empty);
        assert!(session.hits().is_empty());
        assert!(!session.fail(second.generation(), 7, SearchError::Cancelled));
    }

    #[test]
    fn empty_no_match_and_unavailable_states_are_distinct() {
        let index = NotesSearchIndex::build(snapshot()).unwrap();
        let mut session = NotesSearchSession::new();
        assert!(session.begin("   ", 10, 7).unwrap().is_none());
        assert_eq!(session.state(), SearchState::Empty);

        let request = session.begin("absent phrase", 10, 7).unwrap().unwrap();
        assert_eq!(session.state(), SearchState::Indexing);
        assert!(session.apply(index.search(&request).unwrap()));
        assert_eq!(session.state(), SearchState::NoMatches);

        let request = session.begin("roadmap", 10, 7).unwrap().unwrap();
        assert!(session.fail(request.generation(), 7, SearchError::IndexTooLarge));
        assert_eq!(session.state(), SearchState::Unavailable);
        assert_eq!(session.failure(), Some(SearchError::IndexTooLarge));
    }

    #[test]
    fn requests_and_index_debug_output_redact_private_text() {
        let index = NotesSearchIndex::build(snapshot()).unwrap();
        let request = request("secret roadmap query", 10);
        let request_debug = format!("{request:?}");
        let index_debug = format!("{index:?}");

        assert!(!request_debug.contains("secret roadmap query"));
        assert!(request_debug.contains("[private]"));
        assert!(!index_debug.contains("Roadmap"));
        assert!(!index_debug.contains("roadmap.png"));
        assert!(index_debug.contains("indexed_text_bytes"));
        assert_eq!(
            SearchRequest::new(
                SearchGeneration::new(1).unwrap(),
                7,
                "query",
                MAX_SEARCH_RESULTS + 1,
                SearchCancellation::default(),
            )
            .err(),
            Some(SearchError::InvalidRequest)
        );
    }

    #[test]
    fn stale_library_results_and_invalid_requests_cannot_replace_current_state() {
        let index = NotesSearchIndex::build(snapshot()).unwrap();
        let mut session = NotesSearchSession::new();
        let stale = session.begin("roadmap", 10, 6).unwrap().unwrap();

        assert_eq!(index.search(&stale), Err(SearchError::StaleIndex));
        assert!(!session.apply(SearchBatch {
            generation: stale.generation(),
            library_revision: 5,
            hits: Vec::new(),
            results_truncated: false,
            work_bytes: 0,
        }));
        assert!(!session.fail(stale.generation(), 5, SearchError::StaleIndex));
        assert_eq!(session.state(), SearchState::Indexing);

        let error = session
            .begin("query", MAX_SEARCH_RESULTS + 1, 7)
            .unwrap_err();
        assert_eq!(error, SearchError::InvalidRequest);
        assert_eq!(session.state(), SearchState::Unavailable);
        assert_eq!(session.failure(), Some(SearchError::InvalidRequest));
    }

    #[test]
    fn cancelled_index_build_and_selection_retention_are_explicit() {
        let cancellation = SearchCancellation::default();
        cancellation.cancel();
        assert_eq!(
            NotesSearchIndex::build_cancellable(snapshot(), &cancellation).err(),
            Some(SearchError::Cancelled)
        );

        let active = SearchCancellation::default();
        assert_eq!(
            normalize_cancellable("İ", &active, 2),
            Err(SearchError::IndexTooLarge)
        );

        let index = NotesSearchIndex::build(snapshot()).unwrap();
        let mut session = NotesSearchSession::new();
        let first = session.begin("roadmap", 10, 7).unwrap().unwrap();
        assert!(session.apply(index.search(&first).unwrap()));
        let selected = NoteId::new(3).unwrap();
        assert!(session.select(selected));

        let second = session.begin("roadmap", 10, 7).unwrap().unwrap();
        assert!(session.apply(index.search(&second).unwrap()));
        assert_eq!(session.selected(), Some(selected));
        assert!(!session.select(NoteId::new(7).unwrap()));
    }

    #[test]
    fn highlight_spans_preserve_leading_whitespace_coordinates() {
        let mut value = (*snapshot()).clone();
        value.notes[0].title = "  Roadmap  ".into();
        let index = NotesSearchIndex::build(Arc::new(value)).unwrap();

        let batch = index.search(&request("roadmap", 10)).unwrap();
        let title_match = batch
            .hits
            .iter()
            .find(|hit| hit.note_id == NoteId::new(1).unwrap())
            .unwrap()
            .matches
            .iter()
            .find(|matched| matched.field == SearchField::Title)
            .unwrap();

        assert_eq!(
            title_match.span,
            TextSpan {
                start_byte: 2,
                end_byte: 9
            }
        );
    }
}
