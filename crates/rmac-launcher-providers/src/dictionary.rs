//! "define serendipity": a definition from a dictionary installed on this
//! computer.
//!
//! macOS answers from the Dictionary app's Oxford dictionaries. Ubuntu has
//! no dictionary by default; rmac reads the dictd databases the `dict-wn`
//! (WordNet) and `dict-gcide` packages install under `/usr/share/dictd`,
//! directly from their `.index` and `.dict`/`.dict.dz` files, with no
//! server and no network. With none installed the provider stays silent.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};

use super::*;

pub const DICTD_ROOT: &str = "/usr/share/dictd";
/// Longest definition text kept for the one-line subtitle.
const MAX_DEFINITION_CHARS: usize = 240;
/// Largest entry read from a database.
const MAX_ENTRY_BYTES: u64 = 64 * 1024;
const MAX_WORD_CHARS: usize = 48;

/// One dictd database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictionary {
    /// Shown after the word: "serendipity — WordNet".
    pub name: String,
    pub index: PathBuf,
    pub data: PathBuf,
}

/// Databases under `root`, WordNet first, then GCIDE, then any other.
pub fn installed_dictionaries(root: &Path) -> Vec<Dictionary> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let index = entry.path();
            let stem = index
                .file_name()?
                .to_str()?
                .strip_suffix(".index")?
                .to_owned();
            let data = [
                root.join(format!("{stem}.dict.dz")),
                root.join(format!("{stem}.dict")),
            ]
            .into_iter()
            .find(|path| path.is_file())?;
            // dictd's own "short name" entries are skipped: only word lists.
            let name = match stem.as_str() {
                "wn" => "WordNet".to_owned(),
                "gcide" => "GCIDE".to_owned(),
                "moby-thesaurus" | "jargon" | "foldoc" | "vera" | "elements" | "devil" => {
                    return None
                }
                // Translation dictionaries answer in another language.
                other if other.starts_with("freedict") => return None,
                other => other.to_owned(),
            };
            Some(Dictionary { name, index, data })
        })
        .collect::<Vec<_>>();
    found.sort_by_key(|dictionary| {
        (
            match dictionary.name.as_str() {
                "WordNet" => 0,
                "GCIDE" => 1,
                _ => 2,
            },
            dictionary.name.clone(),
        )
    });
    found
}

/// The word in "define serendipity", "definition of x", "meaning of x",
/// "x meaning", "x definition".
pub fn word_to_define(query: &str) -> Option<String> {
    let query = query
        .trim()
        .trim_end_matches('?')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let word = [
        "define ",
        "definition of ",
        "meaning of ",
        "what does ",
        "dictionary ",
    ]
    .iter()
    .find_map(|prefix| query.strip_prefix(prefix))
    .map(|rest| rest.strip_suffix(" mean").unwrap_or(rest))
    .or_else(|| {
        query
            .strip_suffix(" meaning")
            .or_else(|| query.strip_suffix(" definition"))
    })?
    .trim();
    (!word.is_empty()
        && word.chars().count() <= MAX_WORD_CHARS
        && word
            .chars()
            .all(|character| character.is_alphabetic() || matches!(character, ' ' | '-' | '\'')))
    .then(|| word.to_owned())
}

/// The first definition of `word` in `dictionary`.
pub fn lookup(dictionary: &Dictionary, word: &str, cancellation: &Cancellation) -> Option<String> {
    let (offset, length) = find_in_index(&dictionary.index, word, cancellation)?;
    let entry = read_entry(&dictionary.data, offset, length.min(MAX_ENTRY_BYTES))?;
    first_definition(&entry)
}

/// Scan a dictd `.index` (`headword\tOFFSET\tLENGTH`, numbers in dictd's
/// base 64) for `word`, ignoring case.
fn find_in_index(index: &Path, word: &str, cancellation: &Cancellation) -> Option<(u64, u64)> {
    let reader = BufReader::new(File::open(index).ok()?);
    let word = word.to_lowercase();
    for (number, line) in reader.split(b'\n').enumerate() {
        if number % 4_096 == 0 && cancellation.is_cancelled() {
            return None;
        }
        let line = line.ok()?;
        let mut fields = line.split(|byte| *byte == b'\t');
        let (Some(headword), Some(offset), Some(length)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(headword) = std::str::from_utf8(headword) else {
            continue;
        };
        if headword.to_lowercase() == word {
            return Some((decode_b64(offset)?, decode_b64(length)?));
        }
    }
    None
}

/// dictd's base-64 numbers: most significant digit first, alphabet
/// `A–Z a–z 0–9 + /`.
pub fn decode_b64(digits: &[u8]) -> Option<u64> {
    let digits = digits.strip_suffix(b"\r").unwrap_or(digits);
    if digits.is_empty() || digits.len() > 10 {
        return None;
    }
    digits.iter().try_fold(0_u64, |value, digit| {
        let digit = match digit {
            b'A'..=b'Z' => digit - b'A',
            b'a'..=b'z' => digit - b'a' + 26,
            b'0'..=b'9' => digit - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        Some(value * 64 + u64::from(digit))
    })
}

fn read_entry(data: &Path, offset: u64, length: u64) -> Option<String> {
    let bytes = if data.extension().is_some_and(|extension| extension == "dz") {
        Dictzip::open(data)?.read(offset, length)?
    } else {
        let mut file = File::open(data).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut bytes = Vec::new();
        file.take(length).read_to_end(&mut bytes).ok()?;
        bytes
    };
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A dictzip file: gzip whose `RA` extra field lists independently
/// inflatable chunks, so one entry is read without the whole database.
pub(crate) struct Dictzip {
    file: File,
    chunk_length: u64,
    /// (compressed offset in the file, compressed size) per chunk.
    chunks: Vec<(u64, u64)>,
}

impl Dictzip {
    pub(crate) fn open(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        let mut header = [0_u8; 10];
        file.read_exact(&mut header).ok()?;
        if header[0] != 0x1f || header[1] != 0x8b || header[2] != 8 {
            return None;
        }
        let flags = header[3];
        if flags & 0x04 == 0 {
            return None;
        }
        let mut length = [0_u8; 2];
        file.read_exact(&mut length).ok()?;
        let extra_length = usize::from(u16::from_le_bytes(length));
        let mut extra = vec![0_u8; extra_length];
        file.read_exact(&mut extra).ok()?;
        let (chunk_length, sizes) = parse_ra(&extra)?;
        let mut position = 12 + extra_length as u64;
        // FNAME and FCOMMENT are zero-terminated; FHCRC is two bytes.
        for flag in [0x08, 0x10] {
            if flags & flag != 0 {
                let mut byte = [0_u8; 1];
                loop {
                    file.read_exact(&mut byte).ok()?;
                    position += 1;
                    if byte[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 0x02 != 0 {
            position += 2;
        }
        let mut chunks = Vec::with_capacity(sizes.len());
        for size in sizes {
            chunks.push((position, u64::from(size)));
            position += u64::from(size);
        }
        Some(Self {
            file,
            chunk_length: u64::from(chunk_length),
            chunks,
        })
    }

    pub(crate) fn read(&mut self, offset: u64, length: u64) -> Option<Vec<u8>> {
        if length == 0 || self.chunk_length == 0 {
            return Some(Vec::new());
        }
        let first = offset / self.chunk_length;
        let last = (offset + length - 1) / self.chunk_length;
        let mut plain = Vec::new();
        for chunk in first..=last {
            let (start, size) = *self.chunks.get(usize::try_from(chunk).ok()?)?;
            self.file.seek(SeekFrom::Start(start)).ok()?;
            let mut compressed = vec![0_u8; usize::try_from(size).ok()?];
            self.file.read_exact(&mut compressed).ok()?;
            let mut inflater = flate2::Decompress::new(false);
            let mut output = Vec::with_capacity(usize::try_from(self.chunk_length).ok()?);
            inflater
                .decompress_vec(&compressed, &mut output, flate2::FlushDecompress::Sync)
                .ok()?;
            plain.extend_from_slice(&output);
        }
        let skip = usize::try_from(offset - first * self.chunk_length).ok()?;
        let end = skip
            .checked_add(usize::try_from(length).ok()?)?
            .min(plain.len());
        plain.get(skip..end).map(<[u8]>::to_vec)
    }
}

/// The `RA` subfield: version, chunk length, chunk count, then each
/// chunk's compressed size (all little-endian u16).
pub(crate) fn parse_ra(extra: &[u8]) -> Option<(u16, Vec<u16>)> {
    let mut rest = extra;
    while rest.len() >= 4 {
        let (id, length) = (
            &rest[..2],
            usize::from(u16::from_le_bytes([rest[2], rest[3]])),
        );
        let body = rest.get(4..4 + length)?;
        if id == b"RA" {
            let word = |index: usize| {
                body.get(index * 2..index * 2 + 2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            };
            let (version, chunk_length, count) = (word(0)?, word(1)?, word(2)?);
            if version != 1 || chunk_length == 0 {
                return None;
            }
            let sizes = (0..usize::from(count))
                .map(|index| word(3 + index))
                .collect::<Option<Vec<_>>>()?;
            return Some((chunk_length, sizes));
        }
        rest = &rest[4 + length..];
    }
    None
}

/// The first sense of an entry, on one line. WordNet entries read
/// "serendipity\n     n 1: good luck in making unexpected and fortunate
/// discoveries"; other databases fall back to the text after the headword.
pub fn first_definition(entry: &str) -> Option<String> {
    let mut lines = entry.lines();
    lines.next()?;
    let body = lines.collect::<Vec<_>>().join("\n");
    let sense = sense_text(&body).unwrap_or_else(|| body.clone());
    let sense = sense
        .split("[syn:")
        .next()
        .unwrap_or_default()
        .split('\n')
        .map(str::trim)
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = sense.split_whitespace().collect::<Vec<_>>().join(" ");
    // WordNet appends examples in quotes after a semicolon.
    if let Some(index) = text.find("; \"") {
        text.truncate(index);
    }
    let text = text.trim().trim_end_matches(';').trim().to_owned();
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > MAX_DEFINITION_CHARS {
        text.chars().take(MAX_DEFINITION_CHARS).collect()
    } else {
        text
    })
}

/// The text after "1:" (WordNet's first numbered sense) or after "n :" /
/// "adj :" for a single-sense entry, up to the next sense.
fn sense_text(body: &str) -> Option<String> {
    let start = body.find(" 1: ").map(|index| index + 4).or_else(|| {
        [
            " n : ", " v : ", " adj : ", " adv : ", "n : ", "v : ", "adj : ", "adv : ",
        ]
        .iter()
        .find_map(|marker| body.find(marker).map(|index| index + marker.len()))
    })?;
    let rest = &body[start..];
    let end = rest
        .find(" 2: ")
        .into_iter()
        .chain(
            ["\n     n ", "\n     v ", "\n     adj ", "\n     adv "]
                .iter()
                .filter_map(|marker| rest.find(marker)),
        )
        .min()
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

pub struct DictionaryProvider {
    dictionaries: Vec<Dictionary>,
}

impl DictionaryProvider {
    pub fn new(dictionaries: Vec<Dictionary>) -> Self {
        Self { dictionaries }
    }

    /// The dictd databases installed under `/usr/share/dictd`.
    pub fn system() -> Self {
        Self::new(installed_dictionaries(Path::new(DICTD_ROOT)))
    }

    pub fn is_available(&self) -> bool {
        !self.dictionaries.is_empty()
    }
}

impl Provider for DictionaryProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(
            DICTIONARY_PROVIDER,
            Category::Dictionary,
            Privacy::default(),
        )
    }

    fn search(
        &self,
        query: &str,
        cancellation: &Cancellation,
    ) -> Result<Vec<SearchResult>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let Some(word) = word_to_define(query) else {
            return Ok(Vec::new());
        };
        for dictionary in &self.dictionaries {
            if cancellation.is_cancelled() {
                return Err(cancelled());
            }
            if let Some(definition) = lookup(dictionary, &word, cancellation) {
                return Ok(vec![SearchResult {
                    id: ResultId {
                        provider: provider_id(DICTIONARY_PROVIDER),
                        local: format!("{}|{word}", dictionary.name),
                    },
                    category: Category::Dictionary,
                    application_group: None,
                    title: word.clone(),
                    subtitle: Some(definition.clone()),
                    detail: Some(dictionary.name.clone()),
                    icon: None,
                    primary: Action::CopyText { text: definition },
                    alternate: None,
                    recency_rank: 0,
                }]);
            }
        }
        Ok(Vec::new())
    }
}
