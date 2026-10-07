//! Spotlight's matching and ranking: apps first, then files, each ranked
//! by how well the name matches, as Lulo OS's Spotlight ranks them
//! (`rmac_launcher::query_matches`' tiers).

/// Something Spotlight can open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub name: String,
    /// `name` in lower case, so a search allocates nothing per entry.
    pub folded: String,
    /// What `ShellExecute` opens, or a Lulo app's executable name.
    pub target: Target,
    /// The folder a file is in; empty for an app.
    pub location: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    /// A Lulo app's executable, beside `lulo-shell.exe`.
    Lulo(&'static str),
    /// A path, a `shell:AppsFolder\…` item or a URI.
    Shell(String),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    Application,
    Folder,
    Document,
}

impl Entry {
    pub fn new(
        name: impl Into<String>,
        target: Target,
        location: impl Into<String>,
        kind: Kind,
    ) -> Self {
        let name = name.into();
        Self {
            folded: name.to_lowercase(),
            name,
            target,
            location: location.into(),
            kind,
        }
    }
}

impl Kind {
    pub fn section(self) -> &'static str {
        match self {
            Kind::Application => "Applications",
            Kind::Folder => "Folders",
            Kind::Document => "Documents",
        }
    }
}

/// How well `query` matches `name`, both already lower case: whole name,
/// prefix, word prefix, substring, then letters in order.
pub fn match_quality(query: &str, name: &str) -> Option<u16> {
    if query.is_empty() {
        return None;
    }
    if name == query {
        Some(1_000)
    } else if name.starts_with(query) {
        Some(850)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        Some(700)
    } else if name.contains(query) {
        Some(550)
    } else if is_subsequence(query, name) {
        Some(300)
    } else {
        None
    }
}

fn is_subsequence(query: &str, value: &str) -> bool {
    let mut wanted = query.chars().filter(|c| !c.is_whitespace());
    let mut next = wanted.next();
    for c in value.chars() {
        if Some(c) == next {
            next = wanted.next();
            if next.is_none() {
                return true;
            }
        }
    }
    next.is_none()
}

pub fn normalize(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Up to `limit` results for `query`: apps before folders before
/// documents, then the better match, then the shorter name. Files only
/// match by name from the third letter on (or when nothing else does), as
/// one or two letters would match most of a disk.
pub fn search<'a>(
    query: &str,
    entries: impl IntoIterator<Item = &'a Entry>,
    limit: usize,
) -> Vec<&'a Entry> {
    let query = normalize(query);
    if query.is_empty() {
        return Vec::new();
    }
    let short = query.chars().count() < 3;
    let mut hits = entries
        .into_iter()
        .filter_map(|entry| {
            let quality = match_quality(&query, &entry.folded)?;
            // A short query only finds files whose name starts with it.
            if short && entry.kind != Kind::Application && quality < 850 {
                return None;
            }
            // Letters in order is too loose for files.
            if entry.kind != Kind::Application && quality <= 300 {
                return None;
            }
            Some((
                entry.kind,
                std::cmp::Reverse(quality),
                entry.name.len(),
                entry,
            ))
        })
        .collect::<Vec<_>>();
    hits.sort_by_key(|hit| (hit.0, hit.1, hit.2));
    let mut seen = std::collections::HashSet::new();
    hits.into_iter()
        .map(|(_, _, _, entry)| entry)
        // The Start menu often lists an app twice (for the user and for
        // everyone); one row per name and kind.
        .filter(|entry| seen.insert((entry.kind, entry.folded.as_str())))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> Entry {
        Entry::new(
            name,
            Target::Shell(format!("shell:AppsFolder\\{name}")),
            "",
            Kind::Application,
        )
    }

    fn file(name: &str) -> Entry {
        Entry::new(
            name,
            Target::Shell(format!(r"C:\Users\ada\Documents\{name}")),
            "Documents",
            Kind::Document,
        )
    }

    #[test]
    fn match_quality_ranks_like_lulo_spotlight() {
        assert_eq!(match_quality("notepad", "notepad"), Some(1_000));
        assert_eq!(match_quality("note", "notepad"), Some(850));
        assert_eq!(match_quality("edit", "text editor"), Some(700));
        assert_eq!(match_quality("pad", "notepad"), Some(550));
        assert_eq!(match_quality("ntpd", "notepad"), Some(300));
        assert_eq!(match_quality("xyz", "notepad"), None);
        assert_eq!(match_quality("", "notepad"), None);
    }

    #[test]
    fn apps_come_first_then_files_and_duplicates_collapse() {
        let entries = vec![
            file("notes for tuesday.txt"),
            app("Notepad"),
            app("Notes"),
            app("Notepad"),
            app("Sticky Notes"),
            file("calc.xlsx"),
        ];
        let names = search("note", &entries, 10)
            .into_iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["Notes", "Notepad", "Sticky Notes", "notes for tuesday.txt"]
        );
        assert!(search("  ", &entries, 10).is_empty());
        assert_eq!(search("note", &entries, 2).len(), 2);
    }

    #[test]
    fn short_queries_find_files_only_by_their_start() {
        let entries = vec![file("ab.txt"), file("crab.txt"), app("Crab Game")];
        let names = search("ab", &entries, 10)
            .into_iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Crab Game", "ab.txt"]);
    }
}
