//! Width-driven Text Editor document and find-bar geometry.

const COMPACT_MAX_WIDTH: f32 = 720.0;
const REGULAR_CONTENT_PADDING: f32 = 48.0;
const COMPACT_CONTENT_PADDING: f32 = 24.0;
const REGULAR_FIND_WIDTH: f32 = 220.0;
const COMPACT_FIND_WIDTH: f32 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EditorLayout {
    pub(super) compact: bool,
    pub(super) content_padding: f32,
    pub(super) find_input_width: f32,
}

pub(super) fn editor_layout(window_width: f32) -> EditorLayout {
    let compact =
        !window_width.is_finite() || window_width <= 0.0 || window_width < COMPACT_MAX_WIDTH;
    if compact {
        EditorLayout {
            compact: true,
            content_padding: COMPACT_CONTENT_PADDING,
            find_input_width: COMPACT_FIND_WIDTH,
        }
    } else {
        EditorLayout {
            compact: false,
            content_padding: REGULAR_CONTENT_PADDING,
            find_input_width: REGULAR_FIND_WIDTH,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_editor_returns_space_to_document_and_find_content() {
        let compact = editor_layout(640.0);
        assert!(compact.compact);
        assert_eq!(compact.content_padding, COMPACT_CONTENT_PADDING);
        assert_eq!(compact.find_input_width, COMPACT_FIND_WIDTH);

        let regular = editor_layout(860.0);
        assert!(!regular.compact);
        assert_eq!(regular.content_padding, REGULAR_CONTENT_PADDING);
        assert_eq!(regular.find_input_width, REGULAR_FIND_WIDTH);

        assert_eq!(editor_layout(f32::NAN), compact);
    }
}
