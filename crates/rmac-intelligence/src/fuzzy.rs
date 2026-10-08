//! Light typo correction for "open <app>" requests (ADR 0024 "Phase 1.1").
//!
//! "opn notse" should still open Notes. Spotlight checks the request
//! against the installed apps' names before it asks the model, and again
//! when the model names an app that is not installed:
//!
//! - [`open_request`]: the request is an open verb ("open", "launch",
//!   "start", "run", one typo allowed) followed by what to open;
//! - [`closest`]: the one installed name within a small edit distance of
//!   it (one typo up to seven letters, two beyond). A tie between two
//!   different names is no answer.
//!
//! Distances are optimal string alignment (Levenshtein plus swapping two
//! neighbouring letters), on lowercase text.

/// Edit distance with adjacent transpositions, in characters.
pub fn distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let width = right.len() + 1;
    let mut table = vec![0usize; (left.len() + 1) * width];
    for (row, slot) in table.iter_mut().step_by(width).enumerate() {
        *slot = row;
    }
    for (column, slot) in table.iter_mut().take(width).enumerate() {
        *slot = column;
    }
    for row in 1..=left.len() {
        for column in 1..=right.len() {
            let cost = usize::from(left[row - 1] != right[column - 1]);
            let mut best = (table[(row - 1) * width + column] + 1)
                .min(table[row * width + column - 1] + 1)
                .min(table[(row - 1) * width + column - 1] + cost);
            if row > 1
                && column > 1
                && left[row - 1] == right[column - 2]
                && left[row - 2] == right[column - 1]
            {
                best = best.min(table[(row - 2) * width + column - 2] + 1);
            }
            table[row * width + column] = best;
        }
    }
    table[left.len() * width + right.len()]
}

/// How many typos a word of this many characters may carry.
fn allowed(length: usize) -> usize {
    match length {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

const OPEN_VERBS: [&str; 4] = ["open", "launch", "start", "run"];
const FILLER: [&str; 6] = ["the", "my", "app", "application", "up", "please"];

/// What an "open …" request asks to open, lowercase, without filler words
/// ("opn the notse app" → "notse"). `None` when it does not start with an
/// open verb.
pub fn open_request(request: &str) -> Option<String> {
    let lower = request.to_lowercase();
    let mut words = lower
        .split(|character: char| !(character.is_alphanumeric() || character == '-'))
        .filter(|word| !word.is_empty());
    let verb = words.next()?;
    let is_verb = OPEN_VERBS.iter().any(|candidate| {
        verb == *candidate
            || (verb.chars().count() >= 3
                && distance(verb, candidate) <= 1
                // "ran", "rum": a three-letter verb is spelled exactly.
                && candidate.len() > 3)
    });
    if !is_verb {
        return None;
    }
    let rest: Vec<&str> = words.filter(|word| !FILLER.contains(word)).collect();
    (!rest.is_empty()).then(|| rest.join(" "))
}

/// The one name in `names` that `wanted` spells, allowing a light typo:
/// an exact match (ignoring case) first, then the closest name within
/// [`allowed`] typos. `None` when nothing is that close, or when two
/// different names are equally close.
pub fn closest<'a>(wanted: &str, names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let wanted = wanted.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }
    let limit = allowed(wanted.chars().count());
    let mut best: Option<(usize, &'a str)> = None;
    let mut tied = false;
    for name in names {
        let lower = name.to_lowercase();
        if lower == wanted {
            return Some(name);
        }
        let score = distance(&wanted, &lower);
        if score > limit || score > allowed(lower.chars().count()) {
            continue;
        }
        match best {
            Some((top, top_name)) if score == top => {
                tied |= top_name.to_lowercase() != lower;
            }
            Some((top, _)) if score > top => {}
            _ => {
                best = Some((score, name));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPS: [&str; 9] = [
        "Notes",
        "Terminal",
        "Calculator",
        "Clock",
        "Files",
        "Text Editor",
        "Settings",
        "Mail",
        "Maps",
    ];

    #[test]
    fn distances() {
        assert_eq!(distance("notse", "notes"), 1);
        assert_eq!(distance("opn", "open"), 1);
        assert_eq!(distance("opne", "open"), 1);
        assert_eq!(distance("calculater", "calculator"), 1);
        assert_eq!(distance("termnal", "terminal"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }

    #[test]
    fn open_requests_are_recognised_with_a_typo() {
        assert_eq!(open_request("opn notse").as_deref(), Some("notse"));
        assert_eq!(
            open_request("open the calculator app").as_deref(),
            Some("calculator")
        );
        assert_eq!(open_request("lanch Terminal").as_deref(), Some("terminal"));
        assert_eq!(
            open_request("start text editor").as_deref(),
            Some("text editor")
        );
        assert_eq!(open_request("opne my notes").as_deref(), Some("notes"));
        assert_eq!(open_request("turn on dark mode"), None);
        assert_eq!(open_request("ran notes"), None);
        assert_eq!(open_request("open"), None);
        assert_eq!(open_request("open the"), None);
    }

    #[test]
    fn the_closest_installed_name_wins() {
        assert_eq!(closest("notse", APPS), Some("Notes"));
        assert_eq!(closest("notes", APPS), Some("Notes"));
        assert_eq!(closest("calculater", APPS), Some("Calculator"));
        assert_eq!(closest("termnal", APPS), Some("Terminal"));
        assert_eq!(closest("text editr", APPS), Some("Text Editor"));
        assert_eq!(closest("CLOCK", APPS), Some("Clock"));
    }

    #[test]
    fn far_or_ambiguous_names_are_no_answer() {
        // Too far.
        assert_eq!(closest("a 25 minute timer", APPS), None);
        assert_eq!(closest("bluetooth settings", APPS), None);
        assert_eq!(closest("nts", APPS), None);
        // Short words must be exact: "map" is not "Mail" or "Maps".
        assert_eq!(closest("map", APPS), None);
        // "mapl" is one typo from both Mail and Maps.
        assert_eq!(closest("mapl", APPS), None);
        assert_eq!(closest("mals", APPS), Some("Maps"));
        assert_eq!(closest("", APPS), None);
    }
}
