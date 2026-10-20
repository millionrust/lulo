//! What Format ▸ Font ▸ Show Fonts lists and accepts, shared by every
//! rich-text window (Text Editor, Notes).

/// A size typed in the Fonts panel, in points.
pub fn parse_font_size(text: &str) -> Option<f32> {
    let size = text
        .trim()
        .trim_end_matches("pt")
        .trim()
        .parse::<f32>()
        .ok()?;
    (size.is_finite() && (4.0..=288.0).contains(&size)).then_some(size)
}

/// Installed font families, from fontconfig (`fc-list`), sorted and
/// deduplicated. Call it off the UI thread; it is empty when fontconfig's
/// tool is unavailable.
pub fn installed_font_families() -> Vec<String> {
    let Ok(output) = std::process::Command::new("fc-list")
        .args([":", "family"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_font_families(&String::from_utf8_lossy(&output.stdout))
}

/// `fc-list : family` lines list a family's localized names separated by
/// commas; the first is its own name. Hidden faces start with a dot.
pub fn parse_font_families(listing: &str) -> Vec<String> {
    let mut families: Vec<String> = listing
        .lines()
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim().replace("\\-", "-"))
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fontconfig_family_lines_become_one_sorted_name_each() {
        let listing =
            "Noto Sans,Noto Sans Regular\nDejaVu Serif\n.Hidden\nnoto sans mono\nDejaVu Serif\n";
        assert_eq!(
            parse_font_families(listing),
            ["DejaVu Serif", "Noto Sans", "noto sans mono"]
        );
    }

    #[test]
    fn font_sizes_must_be_sane_points() {
        assert_eq!(parse_font_size(" 14 "), Some(14.0));
        assert_eq!(parse_font_size("10.5pt"), Some(10.5));
        assert_eq!(parse_font_size("0"), None);
        assert_eq!(parse_font_size("big"), None);
    }
}
