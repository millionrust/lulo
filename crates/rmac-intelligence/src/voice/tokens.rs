//! Moonshine's vocabulary (`tokens.txt`: one `<piece>\t<id>` per line, a
//! SentencePiece model with byte fallback) and turning token ids into text.

/// The decoder's start and end tokens.
pub const START: i32 = 1;
pub const END: i32 = 2;

#[derive(Clone, Debug, Default)]
pub struct Vocabulary {
    pieces: Vec<String>,
}

impl Vocabulary {
    /// Parse `tokens.txt`. Lines that do not end in a tab and an id are
    /// skipped; an id with no line reads as an empty piece.
    pub fn parse(text: &str) -> Self {
        let mut pieces: Vec<String> = Vec::new();
        for line in text.lines() {
            let Some((piece, id)) = line.rsplit_once('\t') else {
                continue;
            };
            let Ok(id) = id.trim().parse::<usize>() else {
                continue;
            };
            if id >= 1 << 20 {
                continue;
            }
            if pieces.len() <= id {
                pieces.resize(id + 1, String::new());
            }
            pieces[id] = piece.to_owned();
        }
        Self { pieces }
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// The text for `ids`: `▁` starts a word, `<0xNN>` pieces are raw
    /// bytes (joined into UTF-8), control pieces (`<s>`, `</s>`, `<unk>`)
    /// are dropped.
    pub fn decode(&self, ids: &[i32]) -> String {
        let mut bytes: Vec<u8> = Vec::new();
        for id in ids {
            let Some(piece) = usize::try_from(*id).ok().and_then(|id| self.pieces.get(id)) else {
                continue;
            };
            if let Some(byte) = piece
                .strip_prefix("<0x")
                .and_then(|rest| rest.strip_suffix('>'))
                .filter(|hex| hex.len() == 2)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                bytes.push(byte);
                continue;
            }
            if piece.starts_with('<') && piece.ends_with('>') && piece.len() > 2 {
                continue;
            }
            bytes.extend_from_slice(piece.replace('\u{2581}', " ").as_bytes());
        }
        String::from_utf8_lossy(&bytes)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_bytes_and_controls_decode_to_text() {
        let vocabulary = Vocabulary::parse(
            "<unk>\t0\n<s>\t1\n</s>\t2\n<0xC3>\t3\n<0xA9>\t4\n\u{2581}turn\t5\n\u{2581}on\t6\n\u{2581}dark\t7\n\u{2581}mode\t8\n\t9\n\u{2581}caf\t10\nbroken line\n",
        );
        assert_eq!(vocabulary.len(), 11);
        assert_eq!(vocabulary.decode(&[1, 5, 6, 7, 8, 2]), "turn on dark mode");
        // Byte fallback: "café".
        assert_eq!(vocabulary.decode(&[10, 3, 4]), "café");
        // Unknown and negative ids, and the empty piece, are skipped.
        assert_eq!(vocabulary.decode(&[5, 9, 999, -1, 6]), "turn on");
        assert_eq!(vocabulary.decode(&[]), "");
    }
}
