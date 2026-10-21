//! Tile-name line breaking: macOS 26 Apps wraps a name at word boundaries
//! onto at most two centred lines and ends the second with an ellipsis.
//! Nothing is ever clipped at the start of a line.

/// Break `name` into at most two lines that each measure within `max`.
/// `measure` returns the width of a string in the same unit as `max`.
pub(crate) fn wrap_name(name: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> [String; 2] {
    let name = name.trim();
    if measure(name) <= max {
        return [name.to_string(), String::new()];
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    // Greedy first line at word boundaries.
    let mut first = String::new();
    let mut used = 0;
    for word in &words {
        let candidate = if first.is_empty() {
            (*word).to_string()
        } else {
            format!("{first} {word}")
        };
        if measure(&candidate) > max {
            break;
        }
        first = candidate;
        used += 1;
    }
    let mut rest = words[used..].join(" ");
    if first.is_empty() {
        // One word wider than the tile: break it by characters.
        let mut cut = rest.chars().next().map_or(0, char::len_utf8);
        for (index, ch) in rest.char_indices() {
            let end = index + ch.len_utf8();
            if measure(&rest[..end]) > max {
                break;
            }
            cut = end;
        }
        first = rest[..cut].to_string();
        rest = rest[cut..].to_string();
    }
    [first, truncate(&rest, max, measure)]
}

fn truncate(text: &str, max: f32, measure: &dyn Fn(&str) -> f32) -> String {
    if measure(text) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        while chars.last() == Some(&' ') {
            chars.pop();
        }
        let candidate: String = chars.iter().collect::<String>() + "\u{2026}";
        if measure(&candidate) <= max {
            return candidate;
        }
    }
    "\u{2026}".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 12 pt UI text averages about 6 pt a character; the tile name box is
    // TILE_W - 8 = 99 pt wide.
    fn width(text: &str) -> f32 {
        text.chars().count() as f32 * 6.0
    }

    #[test]
    fn long_names_wrap_at_words_and_end_with_an_ellipsis() {
        let a = wrap_name("Advanced Network Configuration", 99.0, &width);
        assert_eq!(a, ["Advanced Network", "Configuration"]);
        let b = wrap_name("Openbox Configuration Manager", 99.0, &width);
        assert_eq!(b[0], "Openbox");
        assert!(b[1].starts_with("Configuration") && b[1].ends_with('\u{2026}'));
        for line in a.iter().chain(b.iter()) {
            assert!(width(line) <= 99.0, "{line:?} too wide");
        }
    }

    #[test]
    fn short_and_unbreakable_names_never_clip() {
        assert_eq!(wrap_name("Files", 99.0, &width), ["Files", ""]);
        let w = wrap_name("Supercalifragilisticexpialidocious", 99.0, &width);
        assert!(w[0].starts_with("Supercali"));
        assert!(width(&w[0]) <= 99.0 && width(&w[1]) <= 99.0);
        assert!(w[1].ends_with('\u{2026}'));
    }
}
