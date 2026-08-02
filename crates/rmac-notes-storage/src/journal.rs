use rmac_notes_store::{decode, CodecError, MAX_LIBRARY_BYTES};
use sha2::{Digest as _, Sha256};

use crate::{Baseline, ErrorKind, Operation, StoreError};

pub(crate) const JOURNAL_MAGIC: &[u8; 8] = b"RMNJRN\0\0";
pub(crate) const JOURNAL_VERSION: u16 = 1;
pub(crate) const MAX_JOURNAL_BYTES: usize = MAX_LIBRARY_BYTES + 128;

#[derive(Clone, Debug)]
pub(crate) struct Journal {
    expected: ExpectedPrimary,
    pub(crate) candidate: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
enum ExpectedPrimary {
    Missing,
    Exact { length: u64, sha256: [u8; 32] },
}

impl Journal {
    pub(crate) fn new(baseline: &Baseline, candidate: Vec<u8>) -> Self {
        let expected = match baseline {
            Baseline::Missing => ExpectedPrimary::Missing,
            Baseline::Exact(bytes) => ExpectedPrimary::Exact {
                length: bytes.len() as u64,
                sha256: digest(bytes),
            },
        };
        Self {
            expected,
            candidate,
        }
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, StoreError> {
        if self.candidate.len() > MAX_LIBRARY_BYTES {
            return Err(StoreError::new(
                Operation::ValidateCandidate,
                ErrorKind::Codec(CodecError::TooLarge),
            ));
        }
        let mut bytes = Vec::with_capacity(self.candidate.len() + 64);
        bytes.extend_from_slice(JOURNAL_MAGIC);
        bytes.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        match self.expected {
            ExpectedPrimary::Missing => {
                bytes.push(0);
                bytes.extend_from_slice(&0_u64.to_le_bytes());
                bytes.extend_from_slice(&[0; 32]);
            }
            ExpectedPrimary::Exact { length, sha256 } => {
                bytes.push(1);
                bytes.extend_from_slice(&length.to_le_bytes());
                bytes.extend_from_slice(&sha256);
            }
        }
        bytes.extend_from_slice(&(self.candidate.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&self.candidate);
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        if bytes.len() > MAX_JOURNAL_BYTES {
            return Err(StoreError::new(
                Operation::ParseJournal,
                ErrorKind::Codec(CodecError::TooLarge),
            ));
        }
        let mut reader = Reader::new(bytes);
        if reader.take(8)? != JOURNAL_MAGIC || reader.u16()? != JOURNAL_VERSION {
            return Err(StoreError::new(
                Operation::ParseJournal,
                ErrorKind::Codec(CodecError::Malformed),
            ));
        }
        let present = reader.byte()?;
        let length = reader.u64()?;
        let sha256: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_| malformed_journal())?;
        let expected = match present {
            0 if length == 0 && sha256 == [0; 32] => ExpectedPrimary::Missing,
            1 => ExpectedPrimary::Exact { length, sha256 },
            _ => return Err(malformed_journal()),
        };
        let candidate_length = usize::try_from(reader.u64()?).map_err(|_| malformed_journal())?;
        if candidate_length > MAX_LIBRARY_BYTES {
            return Err(malformed_journal());
        }
        let candidate = reader.take(candidate_length)?.to_vec();
        if !reader.is_empty() {
            return Err(malformed_journal());
        }
        decode(&candidate).map_err(|error| StoreError::codec(Operation::ParseJournal, error))?;
        Ok(Self {
            expected,
            candidate,
        })
    }

    pub(crate) fn matches_expected(&self, current: Option<&[u8]>) -> bool {
        match (self.expected, current) {
            (ExpectedPrimary::Missing, None) => true,
            (ExpectedPrimary::Exact { length, sha256 }, Some(bytes)) => {
                bytes.len() as u64 == length && digest(bytes) == sha256
            }
            _ => false,
        }
    }
}

pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn malformed_journal() -> StoreError {
    StoreError::new(
        Operation::ParseJournal,
        ErrorKind::Codec(CodecError::Malformed),
    )
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], StoreError> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or_else(malformed_journal)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(malformed_journal)?;
        self.cursor = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, StoreError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, StoreError> {
        self.take(2)?
            .try_into()
            .map(u16::from_le_bytes)
            .map_err(|_| malformed_journal())
    }

    fn u64(&mut self) -> Result<u64, StoreError> {
        self.take(8)?
            .try_into()
            .map(u64::from_le_bytes)
            .map_err(|_| malformed_journal())
    }

    fn is_empty(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}
