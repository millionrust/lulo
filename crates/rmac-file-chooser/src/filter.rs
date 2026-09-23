//! Portal filter matching against the system shared-mime-info database.
//!
//! A filter is compiled once into name globs: glob rules are used as given
//! and every MIME rule expands to the globs of that type, its subclasses,
//! and (for `type/*`) its whole family. Matching is case-insensitive, like
//! GTK 4 and the Mac.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::request::{Filter, Rule};

const MAX_DATABASE_BYTES: u64 = 8 * 1024 * 1024;

/// `mime → globs` and `parent → children` from shared-mime-info.
#[derive(Clone, Debug, Default)]
pub struct MimeDatabase {
    globs: HashMap<String, Vec<String>>,
    children: HashMap<String, Vec<String>>,
}

impl MimeDatabase {
    /// Load `mime/globs2` and `mime/subclasses` from the XDG data directories,
    /// user data first. A missing database simply leaves MIME rules empty.
    pub fn load() -> Self {
        let mut database = Self::default();
        for directory in data_directories() {
            let mime = directory.join("mime");
            if let Some(text) = read_bounded(&mime.join("globs2")) {
                database.add_globs2(&text);
            }
            if let Some(text) = read_bounded(&mime.join("subclasses")) {
                database.add_subclasses(&text);
            }
        }
        database
    }

    pub fn add_globs2(&mut self, text: &str) {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut fields = line.splitn(4, ':');
            let (Some(_weight), Some(mime), Some(glob)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if mime.is_empty() || glob.is_empty() {
                continue;
            }
            let globs = self.globs.entry(mime.to_owned()).or_default();
            if !globs.iter().any(|known| known == glob) {
                globs.push(glob.to_owned());
            }
        }
    }

    pub fn add_subclasses(&mut self, text: &str) {
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            if let (Some(child), Some(parent)) = (fields.next(), fields.next()) {
                let children = self.children.entry(parent.to_owned()).or_default();
                if !children.iter().any(|known| known == child) {
                    children.push(child.to_owned());
                }
            }
        }
    }

    /// `mime` and every type that inherits from it (or, for `type/*`, every
    /// type of that family).
    fn family(&self, mime: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        if let Some(prefix) = mime.strip_suffix("/*") {
            let prefix = format!("{prefix}/");
            queue.extend(
                self.globs
                    .keys()
                    .filter(|known| known.starts_with(&prefix))
                    .cloned(),
            );
        } else {
            queue.push_back(mime.to_owned());
        }
        while let Some(next) = queue.pop_front() {
            if seen.len() >= 4096 || !seen.insert(next.clone()) {
                continue;
            }
            if let Some(children) = self.children.get(&next) {
                queue.extend(children.iter().cloned());
            }
        }
        seen
    }
}

fn data_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(home) => directories.push(PathBuf::from(home)),
        None => {
            if let Some(home) = std::env::var_os("HOME") {
                directories.push(PathBuf::from(home).join(".local/share"));
            }
        }
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    directories.extend(
        system
            .split(':')
            .filter(|path| path.starts_with('/'))
            .map(PathBuf::from),
    );
    directories
}

fn read_bounded(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_DATABASE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// A filter reduced to lowercase name patterns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompiledFilter {
    accept_all: bool,
    extensions: HashSet<String>,
    globs: Vec<Vec<char>>,
}

impl CompiledFilter {
    /// A filter that shows every file (no filter selected).
    pub fn everything() -> Self {
        Self {
            accept_all: true,
            ..Self::default()
        }
    }

    pub fn compile(filter: &Filter, database: &MimeDatabase) -> Self {
        let mut compiled = Self::default();
        for rule in &filter.rules {
            match rule {
                Rule::Glob(glob) => compiled.add_glob(glob),
                Rule::Mime(mime) => {
                    let mime = mime.to_ascii_lowercase();
                    if mime == "application/octet-stream" || mime == "*/*" || mime == "*" {
                        compiled.accept_all = true;
                        continue;
                    }
                    for member in database.family(&mime) {
                        if let Some(globs) = database.globs.get(&member) {
                            for glob in globs {
                                compiled.add_glob(glob);
                            }
                        }
                    }
                }
            }
        }
        compiled
    }

    fn add_glob(&mut self, glob: &str) {
        let glob = glob.to_lowercase();
        if glob == "*" {
            self.accept_all = true;
            return;
        }
        if let Some(extension) = glob.strip_prefix("*.") {
            if !extension.is_empty() && !extension.contains(['*', '?', '[']) {
                self.extensions.insert(extension.to_owned());
                return;
            }
        }
        let pattern: Vec<char> = glob.chars().collect();
        if !self.globs.contains(&pattern) {
            self.globs.push(pattern);
        }
    }

    pub fn accepts(&self, file_name: &str) -> bool {
        if self.accept_all {
            return true;
        }
        let name = file_name.to_lowercase();
        // `*.tar.gz` style rules are stored as the full suffix after `*.`.
        let mut rest = name.as_str();
        while let Some(dot) = rest.find('.') {
            rest = &rest[dot + 1..];
            if self.extensions.contains(rest) {
                return true;
            }
        }
        let name: Vec<char> = name.chars().collect();
        self.globs.iter().any(|glob| glob_match(glob, &name))
    }
}

/// Shell-style glob over characters: `*`, `?`, `[set]`, `[!set]`, `[a-z]`.
pub fn glob_match(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() {
            match pattern[p] {
                '*' => {
                    star = Some((p, t));
                    p += 1;
                    continue;
                }
                '?' => {
                    p += 1;
                    t += 1;
                    continue;
                }
                '[' => {
                    if let Some((matched, next)) = class_match(pattern, p, text[t]) {
                        if matched {
                            p = next;
                            t += 1;
                            continue;
                        }
                    } else if text[t] == '[' {
                        p += 1;
                        t += 1;
                        continue;
                    }
                }
                literal => {
                    if literal == text[t] {
                        p += 1;
                        t += 1;
                        continue;
                    }
                }
            }
        }
        match star {
            Some((star_p, star_t)) => {
                p = star_p + 1;
                t = star_t + 1;
                star = Some((star_p, star_t + 1));
            }
            None => return false,
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

/// Match `character` against the class opening at `pattern[start]`.
/// Returns `(matched, index after the class)`, or `None` if unterminated.
fn class_match(pattern: &[char], start: usize, character: char) -> Option<(bool, usize)> {
    let mut index = start + 1;
    let negated = matches!(pattern.get(index), Some('!') | Some('^'));
    if negated {
        index += 1;
    }
    let mut matched = false;
    let mut first = true;
    while index < pattern.len() {
        let current = pattern[index];
        if current == ']' && !first {
            return Some((matched != negated, index + 1));
        }
        first = false;
        if pattern.get(index + 1) == Some(&'-') && pattern.get(index + 2).is_some_and(|c| *c != ']')
        {
            let high = pattern[index + 2];
            if current <= character && character <= high {
                matched = true;
            }
            index += 3;
        } else {
            if current == character {
                matched = true;
            }
            index += 1;
        }
    }
    None
}
