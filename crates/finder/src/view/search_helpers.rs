use super::*;

pub(super) fn ranked_search_summary(
    report: &rmac_search::SearchReport,
    visible_count: usize,
) -> String {
    let mut summary = format!(
        "{visible_count} result{} · {} items checked",
        if visible_count == 1 { "" } else { "s" },
        report.scanned_entries
    );
    if report.results_truncated {
        summary.push_str(" · result limit reached");
    }
    if report.entry_limit_reached {
        summary.push_str(" · item limit reached");
    }
    if report.content_partially_scanned {
        summary.push_str(" · some content not searched");
    }
    if report.skipped_errors > 0 {
        summary.push_str(&format!(" · {} unavailable", report.skipped_errors));
    }
    summary
}

pub(super) fn ranked_search_error_message(error: &rmac_search::Error) -> String {
    match error {
        rmac_search::Error::InvalidQuery(_) => error.to_string(),
        _ => "Search could not safely read this folder".to_string(),
    }
}

pub(super) fn search_entry_for(
    root: &Path,
    search_match: rmac_search::SearchMatch,
) -> Option<Entry> {
    std::fs::symlink_metadata(&search_match.path).ok()?;
    let mut entry = entry_for(&search_match.path)?;
    let detail = match search_match.kind {
        rmac_search::MatchKind::ExactName => {
            format!(
                "Exact name · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::NamePrefix => {
            format!(
                "Name begins with · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::NameSubstring => {
            format!(
                "Name contains · {}",
                search_parent_label(root, &search_match.path)
            )
        }
        rmac_search::MatchKind::Content => format!(
            "Contents · {}",
            sanitize_dialog_name(search_match.excerpt.as_deref().unwrap_or("Matching text"))
        ),
    };
    entry.search_detail = Some(detail.into());
    Some(entry)
}

fn search_parent_label(root: &Path, path: &Path) -> String {
    let parent = path.parent().unwrap_or(root);
    match parent.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => "This folder".to_string(),
        Ok(relative) => sanitize_dialog_name(&relative.to_string_lossy()),
        Err(_) => "Current search scope".to_string(),
    }
}
