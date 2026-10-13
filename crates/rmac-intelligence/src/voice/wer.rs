//! Word error rate, for the voice evaluation (`rmac-voice-bench`).
//!
//! Both sides are normalised first, as speech evaluations usually are
//! (Whisper's English normaliser does the same, more thoroughly): lower
//! case, no punctuation, "wi-fi" as "wifi", number words as digits
//! ("ten" and "10" are the same request) and "percent" as "%".

/// Words of `text` after normalisation.
pub fn normalise(text: &str) -> Vec<String> {
    let lowered = text
        .to_lowercase()
        .replace("wi-fi", "wifi")
        .replace("wi fi", "wifi")
        .replace('%', " % ")
        .replace('-', " ");
    let cleaned: String = lowered
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '%' || character.is_whitespace() {
                character
            } else if character == '\'' || character == '\u{2019}' {
                '\0'
            } else {
                ' '
            }
        })
        .filter(|character| *character != '\0')
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if word == "percent" || (word == "per" && words.get(index + 1) == Some(&"cent")) {
            out.push("%".into());
            index += if word == "per" { 2 } else { 1 };
            continue;
        }
        if let Some(tens) = tens_value(word) {
            if let Some(units) = words.get(index + 1).and_then(|next| unit_value(next)) {
                if units > 0 && units < 10 {
                    out.push((tens + units).to_string());
                    index += 2;
                    continue;
                }
            }
            out.push(tens.to_string());
        } else if let Some(units) = unit_value(word) {
            out.push(units.to_string());
        } else {
            out.push(word.to_owned());
        }
        index += 1;
    }
    out
}

fn unit_value(word: &str) -> Option<u32> {
    const UNITS: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    UNITS
        .iter()
        .position(|unit| *unit == word)
        .map(|value| value as u32)
}

fn tens_value(word: &str) -> Option<u32> {
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    TENS.iter()
        .position(|tens| *tens == word)
        .map(|value| (value as u32 + 2) * 10)
}

/// Word-level edit distance between `reference` and `hypothesis`
/// (substitutions, deletions and insertions).
pub fn errors(reference: &[String], hypothesis: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    for (row, reference_word) in reference.iter().enumerate() {
        let mut current = Vec::with_capacity(hypothesis.len() + 1);
        current.push(row + 1);
        for (column, hypothesis_word) in hypothesis.iter().enumerate() {
            let substitution = previous[column] + usize::from(reference_word != hypothesis_word);
            let deletion = previous[column + 1] + 1;
            let insertion = current[column] + 1;
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous[hypothesis.len()]
}

/// Word error rate over a set of (reference, hypothesis) pairs: total
/// errors over total reference words.
pub fn word_error_rate<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> f64 {
    let (mut wrong, mut total) = (0usize, 0usize);
    for (reference, hypothesis) in pairs {
        let reference = normalise(reference);
        wrong += errors(&reference, &normalise(hypothesis));
        total += reference.len();
    }
    if total == 0 {
        0.0
    } else {
        wrong as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_makes_spoken_and_written_forms_agree() {
        assert_eq!(
            normalise("Turn on Wi-Fi, please."),
            ["turn", "on", "wifi", "please"]
        );
        assert_eq!(
            normalise("Set the volume to thirty percent"),
            normalise("set the volume to 30%")
        );
        assert_eq!(
            normalise("a timer for twenty five minutes"),
            ["a", "timer", "for", "25", "minutes"]
        );
        assert_eq!(normalise("Don't"), ["dont"]);
        assert_eq!(normalise("ten per cent"), ["10", "%"]);
    }

    #[test]
    fn errors_count_edits() {
        let words = |text: &str| normalise(text);
        assert_eq!(
            errors(&words("turn on dark mode"), &words("turn on dark mode")),
            0
        );
        assert_eq!(
            errors(&words("turn on dark mode"), &words("turn on the dark mode")),
            1
        );
        assert_eq!(
            errors(&words("turn on dark mode"), &words("turn of dark")),
            2
        );
        assert_eq!(errors(&words("open notes"), &words("")), 2);
        let rate = word_error_rate([
            ("turn on dark mode", "Turn on dark mode."),
            ("open notes", "open nodes"),
        ]);
        assert!((rate - 1.0 / 6.0).abs() < 1e-9);
        assert_eq!(word_error_rate([]), 0.0);
    }
}
