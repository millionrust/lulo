use std::ops::Range;

pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImeBuffer {
    pub(crate) text: String,
    pub(crate) selection_utf16: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImeEditError {
    InvalidRange,
    TooLarge,
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

fn byte_offset_for_utf16(text: &str, target: usize) -> Option<usize> {
    let mut utf16_offset = 0;
    for (byte_offset, character) in text.char_indices() {
        if utf16_offset == target {
            return Some(byte_offset);
        }
        utf16_offset += character.len_utf16();
        if utf16_offset > target {
            return None;
        }
    }
    (utf16_offset == target).then_some(text.len())
}

pub(crate) fn byte_range_for_utf16(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    if range.start > range.end {
        return None;
    }
    Some(byte_offset_for_utf16(text, range.start)?..byte_offset_for_utf16(text, range.end)?)
}

pub(crate) fn replace_buffer(
    current: Option<&ImeBuffer>,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    new_selection_utf16: Option<Range<usize>>,
) -> Result<ImeBuffer, ImeEditError> {
    let current_text = current.map_or("", |buffer| buffer.text.as_str());
    let current_utf16_len = utf16_len(current_text);
    let replacement_utf16 = range_utf16.unwrap_or(0..current_utf16_len);
    let replacement_bytes = byte_range_for_utf16(current_text, replacement_utf16.clone())
        .ok_or(ImeEditError::InvalidRange)?;
    let new_byte_len = current_text
        .len()
        .checked_sub(replacement_bytes.len())
        .and_then(|len| len.checked_add(new_text.len()))
        .ok_or(ImeEditError::TooLarge)?;
    if new_byte_len > MAX_TEXT_BYTES {
        return Err(ImeEditError::TooLarge);
    }

    let inserted_utf16_len = utf16_len(new_text);
    let relative_selection = new_selection_utf16.unwrap_or(inserted_utf16_len..inserted_utf16_len);
    if byte_range_for_utf16(new_text, relative_selection.clone()).is_none() {
        return Err(ImeEditError::InvalidRange);
    }

    let mut text = String::with_capacity(new_byte_len);
    text.push_str(&current_text[..replacement_bytes.start]);
    text.push_str(new_text);
    text.push_str(&current_text[replacement_bytes.end..]);
    let selection_start = replacement_utf16
        .start
        .checked_add(relative_selection.start)
        .ok_or(ImeEditError::InvalidRange)?;
    let selection_end = replacement_utf16
        .start
        .checked_add(relative_selection.end)
        .ok_or(ImeEditError::InvalidRange)?;

    Ok(ImeBuffer {
        text,
        selection_utf16: selection_start..selection_end,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        byte_offset_for_utf16, byte_range_for_utf16, replace_buffer, utf16_len, ImeBuffer,
        ImeEditError, MAX_TEXT_BYTES,
    };
    use std::ops::Range;

    #[test]
    fn ranges_use_utf16_scalar_boundaries() {
        let text = "a😀界";

        assert_eq!(utf16_len(text), 4);
        assert_eq!(byte_offset_for_utf16(text, 0), Some(0));
        assert_eq!(byte_offset_for_utf16(text, 1), Some(1));
        assert_eq!(byte_offset_for_utf16(text, 2), None);
        assert_eq!(byte_offset_for_utf16(text, 3), Some(5));
        assert_eq!(byte_offset_for_utf16(text, 4), Some(text.len()));
        assert_eq!(byte_offset_for_utf16(text, 5), None);
        assert_eq!(byte_range_for_utf16(text, 1..3), Some(1..5));
        assert_eq!(byte_range_for_utf16(text, 2..3), None);
        let reversed = Range { start: 3, end: 1 };
        assert_eq!(byte_range_for_utf16(text, reversed), None);
    }

    #[test]
    fn preedit_replacement_is_bounded_and_selection_safe() {
        assert_eq!(MAX_TEXT_BYTES, 16 * 1024);
        let initial = replace_buffer(None, Some(0..0), "ka", Some(2..2)).unwrap();
        assert_eq!(initial.text, "ka");
        assert_eq!(initial.selection_utf16, 2..2);

        let committed = replace_buffer(Some(&initial), None, "か", Some(1..1)).unwrap();
        assert_eq!(committed.text, "か");
        assert_eq!(committed.selection_utf16, 1..1);

        let emoji = ImeBuffer {
            text: "a😀界".into(),
            selection_utf16: 4..4,
        };
        let edited = replace_buffer(Some(&emoji), Some(1..3), "é", Some(1..1)).unwrap();
        assert_eq!(edited.text, "aé界");
        assert_eq!(edited.selection_utf16, 2..2);

        assert_eq!(
            replace_buffer(Some(&emoji), Some(2..3), "x", None),
            Err(ImeEditError::InvalidRange)
        );
        assert_eq!(
            replace_buffer(None, None, "😀", Some(1..1)),
            Err(ImeEditError::InvalidRange)
        );

        let exact_limit = format!("{}界", "x".repeat(MAX_TEXT_BYTES - "界".len()));
        assert_eq!(
            replace_buffer(None, None, &exact_limit, None)
                .unwrap()
                .text
                .len(),
            MAX_TEXT_BYTES
        );
        assert_eq!(
            replace_buffer(None, None, &format!("{exact_limit}x"), None),
            Err(ImeEditError::TooLarge)
        );
    }
}
