use std::fmt;
use std::ops::Range;

use markdown::mdast::Node;
use rmac_notes_store::MAX_BODY_BYTES;

pub const MAX_MARKDOWN_PREVIEW_BLOCKS: usize = 4_096;
pub const MAX_MARKDOWN_PREVIEW_RUNS: usize = 32_768;
pub const MAX_MARKDOWN_PREVIEW_DEPTH: usize = 64;
pub const MAX_MARKDOWN_PREVIEW_OUTPUT_BYTES: usize = MAX_BODY_BYTES + 64 * 1_024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkdownPreviewTextStyle {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link_label: bool,
    pub inert_placeholder: bool,
    /// Format ▸ Font ▸ Underline (`notes::ToggleUnderline`): a `++marker++`
    /// span. Not a CommonMark/GFM construct, so it is recognised by
    /// [`scan_custom_marker_spans`] rather than the `markdown` crate's AST.
    pub underline: bool,
    /// Format ▸ Font ▸ Highlight (`notes::ToggleHighlight`): a `==marker==`
    /// span, scanned the same way as `underline`.
    pub highlight: bool,
    /// Format ▸ Font ▸ Baseline ▸ Superscript: a `^marker^` span.
    pub superscript: bool,
    /// Format ▸ Font ▸ Baseline ▸ Subscript: a `::marker::` span. Not a
    /// lone `~marker~`: this crate's GFM strikethrough accepts a single
    /// `~` the same as a doubled `~~`, so that span is already claimed
    /// before it ever reaches the AST as literal text.
    pub subscript: bool,
}

/// Format ▸ Text ▸ Align Left/Centre/Align Right: a trailing
/// ` :center:`/` :right:` marker on a paragraph or heading line, stripped
/// and recorded here by [`strip_trailing_alignment_marker`] rather than
/// rendered as visible text. There is no "Justify" variant: GPUI's text
/// layout has no justified line-breaking API, so Format ▸ Text ▸ Justify is
/// intentionally left out of the menu rather than faked (docs/parity.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MarkdownPreviewRun {
    range: Range<usize>,
    style: MarkdownPreviewTextStyle,
}

impl MarkdownPreviewRun {
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn style(&self) -> MarkdownPreviewTextStyle {
        self.style
    }
}

impl fmt::Debug for MarkdownPreviewRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkdownPreviewRun")
            .field("range", &self.range)
            .field("style", &self.style)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewBlockKind {
    Paragraph(TextAlign),
    Heading(u8, TextAlign),
    BlockQuote,
    ListItem {
        depth: u8,
        ordered_index: Option<u32>,
        checked: Option<bool>,
        unordered_marker: Option<char>,
    },
    CodeBlock,
    TableRow {
        header: bool,
    },
    Footnote,
    InertNotice,
    ThematicBreak,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MarkdownPreviewBlock {
    kind: MarkdownPreviewBlockKind,
    text: String,
    runs: Vec<MarkdownPreviewRun>,
    /// For a checklist/list item, the byte range of the *entire item* in the
    /// original Markdown source (its `- [ ] `/`- [x] ` marker included), so a
    /// click on the rendered checkbox can flip the marker back in the stored
    /// body without re-parsing or guessing at line numbers. `None` for every
    /// other block kind, and for list items the source parser could not
    /// place (never expected in practice).
    source_range: Option<Range<usize>>,
}

impl MarkdownPreviewBlock {
    pub fn kind(&self) -> MarkdownPreviewBlockKind {
        self.kind
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn runs(&self) -> &[MarkdownPreviewRun] {
        &self.runs
    }

    /// The byte range of this list item in the Markdown source, when known.
    pub fn source_range(&self) -> Option<Range<usize>> {
        self.source_range.clone()
    }
}

impl fmt::Debug for MarkdownPreviewBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkdownPreviewBlock")
            .field("kind", &self.kind)
            .field("text_bytes", &self.text.len())
            .field("runs", &self.runs.len())
            .field("source_range", &self.source_range)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MarkdownPreviewDocument {
    source_bytes: usize,
    blocks: Vec<MarkdownPreviewBlock>,
    truncated: bool,
}

impl MarkdownPreviewDocument {
    pub fn source_bytes(&self) -> usize {
        self.source_bytes
    }

    pub fn blocks(&self) -> &[MarkdownPreviewBlock] {
        &self.blocks
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl fmt::Debug for MarkdownPreviewDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkdownPreviewDocument")
            .field("source_bytes", &self.source_bytes)
            .field("blocks", &self.blocks.len())
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownPreviewError {
    SourceTooLarge,
    Parse,
}

impl fmt::Display for MarkdownPreviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SourceTooLarge => "The note exceeds the Markdown preview safety limit",
            Self::Parse => "Notes could not format this Markdown safely",
        })
    }
}

impl std::error::Error for MarkdownPreviewError {}

pub fn parse_inert_markdown_preview(
    source: &str,
) -> Result<MarkdownPreviewDocument, MarkdownPreviewError> {
    if source.len() > MAX_BODY_BYTES {
        return Err(MarkdownPreviewError::SourceTooLarge);
    }
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let tree = markdown::to_mdast(source, &options).map_err(|_| MarkdownPreviewError::Parse)?;
    let mut builder = PreviewBuilder::new(source);
    builder.render_node(&tree, BlockContext::Paragraph, 0);
    Ok(builder.finish())
}

#[derive(Clone, Copy)]
enum BlockContext {
    Paragraph,
    BlockQuote,
    ListItem {
        depth: u8,
        ordered_index: Option<u32>,
        checked: Option<bool>,
        unordered_marker: Option<char>,
        /// The whole item's byte range in the source, as a `(start, end)`
        /// pair so the context stays `Copy` (unlike `Range<usize>`).
        source_range: Option<(usize, usize)>,
    },
    Footnote,
}

/// The current list item's source byte range, if `context` is inside one.
fn list_item_source_range(context: BlockContext) -> Option<Range<usize>> {
    match context {
        BlockContext::ListItem {
            source_range: Some((start, end)),
            ..
        } => Some(start..end),
        _ => None,
    }
}

struct PreviewBuilder<'a> {
    source: &'a str,
    source_bytes: usize,
    remaining_text_bytes: usize,
    remaining_runs: usize,
    blocks: Vec<MarkdownPreviewBlock>,
    truncated: bool,
}

impl<'a> PreviewBuilder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            source_bytes: source.len(),
            remaining_text_bytes: MAX_MARKDOWN_PREVIEW_OUTPUT_BYTES,
            remaining_runs: MAX_MARKDOWN_PREVIEW_RUNS,
            blocks: Vec::new(),
            truncated: false,
        }
    }

    fn finish(self) -> MarkdownPreviewDocument {
        MarkdownPreviewDocument {
            source_bytes: self.source_bytes,
            blocks: self.blocks,
            truncated: self.truncated,
        }
    }

    fn render_nodes(&mut self, nodes: &[Node], context: BlockContext, depth: usize) {
        for node in nodes {
            if self.truncated {
                break;
            }
            self.render_node(node, context, depth);
        }
    }

    fn render_node(&mut self, node: &Node, context: BlockContext, depth: usize) {
        if depth > MAX_MARKDOWN_PREVIEW_DEPTH {
            self.truncated = true;
            return;
        }
        match node {
            Node::Root(root) => self.render_nodes(&root.children, context, depth + 1),
            Node::Paragraph(paragraph) => self.push_inline_block(
                context_kind(context),
                &paragraph.children,
                depth + 1,
                list_item_source_range(context),
            ),
            Node::Heading(heading) => self.push_inline_block(
                MarkdownPreviewBlockKind::Heading(heading.depth, TextAlign::Left),
                &heading.children,
                depth + 1,
                None,
            ),
            Node::Blockquote(quote) => {
                self.render_nodes(&quote.children, BlockContext::BlockQuote, depth + 1)
            }
            Node::List(list) => self.render_list(list, depth + 1),
            Node::ListItem(item) => self.render_list_item(item, None, 0, depth + 1),
            Node::Code(code) => {
                let mut block = BlockBuilder::new(MarkdownPreviewBlockKind::CodeBlock);
                block.append(
                    &code.value,
                    MarkdownPreviewTextStyle {
                        code: true,
                        ..MarkdownPreviewTextStyle::default()
                    },
                    &mut self.remaining_text_bytes,
                );
                self.push_block(block);
            }
            Node::Table(table) => {
                for (index, row) in table.children.iter().enumerate() {
                    if let Node::TableRow(row) = row {
                        let mut block = BlockBuilder::new(MarkdownPreviewBlockKind::TableRow {
                            header: index == 0,
                        });
                        for (cell_index, cell) in row.children.iter().enumerate() {
                            if cell_index != 0 {
                                block.append(
                                    "  |  ",
                                    MarkdownPreviewTextStyle::default(),
                                    &mut self.remaining_text_bytes,
                                );
                            }
                            if let Node::TableCell(cell) = cell {
                                render_inline_nodes(
                                    &cell.children,
                                    &mut block,
                                    MarkdownPreviewTextStyle::default(),
                                    depth + 1,
                                    &mut self.remaining_text_bytes,
                                    &mut self.truncated,
                                );
                            }
                        }
                        self.push_block(block);
                    }
                }
            }
            Node::FootnoteDefinition(footnote) => {
                self.render_nodes(&footnote.children, BlockContext::Footnote, depth + 1)
            }
            Node::Html(_) => self.push_notice("Raw HTML is inert in Notes preview"),
            Node::Yaml(_) | Node::Toml(_) => {
                self.push_notice("Frontmatter remains editable source and is not Notes metadata")
            }
            Node::ThematicBreak(_) => {
                self.push_block(BlockBuilder::new(MarkdownPreviewBlockKind::ThematicBreak))
            }
            Node::Definition(_) => {}
            _ => self.push_inline_block(
                context_kind(context),
                std::slice::from_ref(node),
                depth,
                list_item_source_range(context),
            ),
        }
    }

    fn render_list(&mut self, list: &markdown::mdast::List, depth: usize) {
        let ordered_start = list.start;
        for (index, child) in list.children.iter().enumerate() {
            let Node::ListItem(item) = child else {
                continue;
            };
            let ordered_index = ordered_start
                .map(|start| start.saturating_add(u32::try_from(index).unwrap_or(u32::MAX)));
            self.render_list_item(item, ordered_index, 0, depth + 1);
        }
    }

    fn render_list_item(
        &mut self,
        item: &markdown::mdast::ListItem,
        ordered_index: Option<u32>,
        list_depth: u8,
        depth: usize,
    ) {
        let unordered_marker = if ordered_index.is_none() {
            item.position
                .as_ref()
                .and_then(|position| self.source.get(position.start.offset..))
                .and_then(|source| source.trim_start().chars().next())
                .filter(|marker| matches!(marker, '-' | '*' | '+'))
        } else {
            None
        };
        let context = BlockContext::ListItem {
            depth: list_depth,
            ordered_index,
            checked: item.checked,
            unordered_marker,
            source_range: item
                .position
                .as_ref()
                .map(|position| (position.start.offset, position.end.offset)),
        };
        for child in &item.children {
            match child {
                Node::List(list) => {
                    let nested_depth = list_depth.saturating_add(1);
                    for (index, nested) in list.children.iter().enumerate() {
                        if let Node::ListItem(nested) = nested {
                            let nested_index = list.start.map(|start| {
                                start.saturating_add(u32::try_from(index).unwrap_or(u32::MAX))
                            });
                            self.render_list_item(nested, nested_index, nested_depth, depth + 1);
                        }
                    }
                }
                _ => self.render_node(child, context, depth + 1),
            }
        }
    }

    fn push_inline_block(
        &mut self,
        kind: MarkdownPreviewBlockKind,
        nodes: &[Node],
        depth: usize,
        source_range: Option<Range<usize>>,
    ) {
        let mut block = BlockBuilder::new(kind).with_source_range(source_range);
        render_inline_nodes(
            nodes,
            &mut block,
            MarkdownPreviewTextStyle::default(),
            depth,
            &mut self.remaining_text_bytes,
            &mut self.truncated,
        );
        apply_trailing_alignment(&mut block);
        self.push_block(block);
    }

    fn push_notice(&mut self, message: &str) {
        let mut block = BlockBuilder::new(MarkdownPreviewBlockKind::InertNotice);
        block.append(
            message,
            MarkdownPreviewTextStyle {
                inert_placeholder: true,
                ..MarkdownPreviewTextStyle::default()
            },
            &mut self.remaining_text_bytes,
        );
        self.push_block(block);
    }

    fn push_block(&mut self, block: BlockBuilder) {
        if block.truncated {
            self.truncated = true;
        }
        if self.blocks.len() >= MAX_MARKDOWN_PREVIEW_BLOCKS
            || block.runs.len() > self.remaining_runs
        {
            self.truncated = true;
            return;
        }
        self.remaining_runs -= block.runs.len();
        if !block.text.is_empty() || matches!(block.kind, MarkdownPreviewBlockKind::ThematicBreak) {
            self.blocks.push(block.finish());
        }
    }
}

fn context_kind(context: BlockContext) -> MarkdownPreviewBlockKind {
    match context {
        BlockContext::Paragraph => MarkdownPreviewBlockKind::Paragraph(TextAlign::Left),
        BlockContext::BlockQuote => MarkdownPreviewBlockKind::BlockQuote,
        BlockContext::ListItem {
            depth,
            ordered_index,
            checked,
            unordered_marker,
            ..
        } => MarkdownPreviewBlockKind::ListItem {
            depth,
            ordered_index,
            checked,
            unordered_marker,
        },
        BlockContext::Footnote => MarkdownPreviewBlockKind::Footnote,
    }
}

fn render_inline_nodes(
    nodes: &[Node],
    block: &mut BlockBuilder,
    style: MarkdownPreviewTextStyle,
    depth: usize,
    remaining_text_bytes: &mut usize,
    truncated: &mut bool,
) {
    if depth > MAX_MARKDOWN_PREVIEW_DEPTH {
        *truncated = true;
        return;
    }
    for node in nodes {
        if *truncated {
            return;
        }
        match node {
            Node::Text(text) => {
                scan_custom_marker_spans(&text.value, style, block, remaining_text_bytes)
            }
            Node::Break(_) => block.append("\n", style, remaining_text_bytes),
            Node::InlineCode(code) => {
                block.append(
                    &code.value,
                    MarkdownPreviewTextStyle {
                        code: true,
                        ..style
                    },
                    remaining_text_bytes,
                );
            }
            Node::InlineMath(math) => {
                block.append(
                    &math.value,
                    MarkdownPreviewTextStyle {
                        code: true,
                        ..style
                    },
                    remaining_text_bytes,
                );
            }
            Node::Emphasis(emphasis) => render_inline_nodes(
                &emphasis.children,
                block,
                MarkdownPreviewTextStyle {
                    italic: true,
                    ..style
                },
                depth + 1,
                remaining_text_bytes,
                truncated,
            ),
            Node::Strong(strong) => render_inline_nodes(
                &strong.children,
                block,
                MarkdownPreviewTextStyle {
                    bold: true,
                    ..style
                },
                depth + 1,
                remaining_text_bytes,
                truncated,
            ),
            Node::Delete(deleted) => render_inline_nodes(
                &deleted.children,
                block,
                MarkdownPreviewTextStyle {
                    strikethrough: true,
                    ..style
                },
                depth + 1,
                remaining_text_bytes,
                truncated,
            ),
            Node::Link(link) => render_inline_nodes(
                &link.children,
                block,
                MarkdownPreviewTextStyle {
                    link_label: true,
                    ..style
                },
                depth + 1,
                remaining_text_bytes,
                truncated,
            ),
            Node::LinkReference(link) => render_inline_nodes(
                &link.children,
                block,
                MarkdownPreviewTextStyle {
                    link_label: true,
                    ..style
                },
                depth + 1,
                remaining_text_bytes,
                truncated,
            ),
            Node::Image(image) => {
                let label = if image.alt.trim().is_empty() {
                    "Linked image".to_string()
                } else {
                    format!("Linked image: {}", image.alt.trim())
                };
                block.append(
                    &label,
                    MarkdownPreviewTextStyle {
                        inert_placeholder: true,
                        ..style
                    },
                    remaining_text_bytes,
                );
            }
            Node::ImageReference(image) => {
                let label = if image.alt.trim().is_empty() {
                    "Linked image".to_string()
                } else {
                    format!("Linked image: {}", image.alt.trim())
                };
                block.append(
                    &label,
                    MarkdownPreviewTextStyle {
                        inert_placeholder: true,
                        ..style
                    },
                    remaining_text_bytes,
                );
            }
            Node::Html(_) => block.append(
                "Raw HTML omitted",
                MarkdownPreviewTextStyle {
                    inert_placeholder: true,
                    ..style
                },
                remaining_text_bytes,
            ),
            Node::FootnoteReference(_) => block.append(
                "Footnote",
                MarkdownPreviewTextStyle {
                    link_label: true,
                    ..style
                },
                remaining_text_bytes,
            ),
            _ => {
                if let Some(children) = node.children() {
                    render_inline_nodes(
                        children,
                        block,
                        style,
                        depth + 1,
                        remaining_text_bytes,
                        truncated,
                    );
                }
            }
        }
        if *remaining_text_bytes == 0 {
            *truncated = true;
        }
    }
}

/// The trailing line markers Format ▸ Text ▸ Align Left/Centre/Align Right
/// write (`note_format_controller::set_text_alignment`). Checked longest
/// first is unnecessary since the three are mutually exclusive by their `:`
/// delimiters, but kept in a fixed, obvious order.
const ALIGNMENT_MARKERS: [(&str, TextAlign); 2] = [
    (" :center:", TextAlign::Center),
    (" :right:", TextAlign::Right),
];

/// If `block`'s kind is a Paragraph or Heading whose rendered text ends with
/// one of [`ALIGNMENT_MARKERS`], strip the marker from the visible text (and
/// shrink/drop its trailing style run to match) and rewrite `block.kind` to
/// carry the detected [`TextAlign`]. A line with no marker is left as Left.
fn apply_trailing_alignment(block: &mut BlockBuilder) {
    let align = match block.kind {
        MarkdownPreviewBlockKind::Paragraph(_) | MarkdownPreviewBlockKind::Heading(_, _) => {
            strip_trailing_alignment_marker(&mut block.text, &mut block.runs)
        }
        _ => return,
    };
    block.kind = match block.kind {
        MarkdownPreviewBlockKind::Paragraph(_) => MarkdownPreviewBlockKind::Paragraph(align),
        MarkdownPreviewBlockKind::Heading(depth, _) => {
            MarkdownPreviewBlockKind::Heading(depth, align)
        }
        other => other,
    };
}

fn strip_trailing_alignment_marker(
    text: &mut String,
    runs: &mut Vec<MarkdownPreviewRun>,
) -> TextAlign {
    for (marker, align) in ALIGNMENT_MARKERS {
        let Some(kept_len) = text.len().checked_sub(marker.len()) else {
            continue;
        };
        if text.get(kept_len..) != Some(marker) {
            continue;
        }
        text.truncate(kept_len);
        while let Some(last) = runs.last().cloned() {
            if last.range.start >= text.len() {
                runs.pop();
            } else if last.range.end > text.len() {
                runs.last_mut().expect("just checked non-empty").range.end = text.len();
                break;
            } else {
                break;
            }
        }
        return align;
    }
    TextAlign::Left
}

/// One of Format ▸ Font's custom, non-CommonMark inline markers: not
/// produced by the `markdown` crate's parser, so plain [`Node::Text`]
/// content is re-scanned for them here. `++marker++` is Underline,
/// `==marker==` is Highlight, `^marker^` is Superscript, and `::marker::`
/// is Subscript. A lone `~marker~` cannot be used for Subscript: this
/// crate's GFM strikethrough accepts a single `~` the same as a doubled
/// `~~`, so any `~...~` text is already consumed as a `Node::Delete`
/// before it ever reaches this scan. Markers do not nest with each other;
/// an opener with no matching closer is left as plain text for the
/// remainder of this text node, the same bounded, best-effort approach
/// `find_checkbox_marker` in `note_format_controller.rs` uses for its own
/// marker search.
type MarkerStyleFn = fn(MarkdownPreviewTextStyle) -> MarkdownPreviewTextStyle;

const CUSTOM_MARKERS: [(&str, MarkerStyleFn); 4] = [
    ("++", |style| MarkdownPreviewTextStyle {
        underline: true,
        ..style
    }),
    ("==", |style| MarkdownPreviewTextStyle {
        highlight: true,
        ..style
    }),
    ("^", |style| MarkdownPreviewTextStyle {
        superscript: true,
        ..style
    }),
    ("::", |style| MarkdownPreviewTextStyle {
        subscript: true,
        ..style
    }),
];

fn scan_custom_marker_spans(
    mut text: &str,
    style: MarkdownPreviewTextStyle,
    block: &mut BlockBuilder,
    remaining_text_bytes: &mut usize,
) {
    while !text.is_empty() {
        if *remaining_text_bytes == 0 {
            return;
        }
        // The earliest-opening marker wins, not the first entry in
        // `CUSTOM_MARKERS` that happens to appear anywhere in `text` — two
        // different marker spans can both be present, in either order.
        let found = CUSTOM_MARKERS
            .iter()
            .filter_map(|&(marker, apply)| {
                let open = text.find(marker)?;
                let after_open = open + marker.len();
                let close_relative = text[after_open..].find(marker)?;
                // A zero-length span (adjacent markers, "++++") is not a
                // styled run: skip it rather than emit an empty one.
                if close_relative == 0 {
                    return None;
                }
                Some((open, marker.len(), after_open + close_relative, apply))
            })
            .min_by_key(|&(open, ..)| open);
        let Some((open, marker_len, close, apply)) = found else {
            block.append(text, style, remaining_text_bytes);
            return;
        };
        if open > 0 {
            block.append(&text[..open], style, remaining_text_bytes);
        }
        let inner_start = open + marker_len;
        block.append(
            &text[inner_start..close],
            apply(style),
            remaining_text_bytes,
        );
        text = &text[close + marker_len..];
    }
}

struct BlockBuilder {
    kind: MarkdownPreviewBlockKind,
    text: String,
    runs: Vec<MarkdownPreviewRun>,
    truncated: bool,
    source_range: Option<Range<usize>>,
}

impl BlockBuilder {
    fn new(kind: MarkdownPreviewBlockKind) -> Self {
        Self {
            kind,
            text: String::new(),
            runs: Vec::new(),
            truncated: false,
            source_range: None,
        }
    }

    fn append(
        &mut self,
        value: &str,
        style: MarkdownPreviewTextStyle,
        remaining_text_bytes: &mut usize,
    ) {
        if value.is_empty() || *remaining_text_bytes == 0 {
            return;
        }
        let mut end = value.len().min(*remaining_text_bytes);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return;
        }
        let can_merge = self.runs.last().is_some_and(|previous| {
            previous.style == style && previous.range.end == self.text.len()
        });
        if !can_merge && self.runs.len() >= MAX_MARKDOWN_PREVIEW_RUNS {
            self.truncated = true;
            return;
        }
        let start = self.text.len();
        self.text.push_str(&value[..end]);
        let range = start..self.text.len();
        *remaining_text_bytes -= end;
        if can_merge {
            self.runs
                .last_mut()
                .expect("a mergeable run exists")
                .range
                .end = range.end;
            return;
        }
        self.runs.push(MarkdownPreviewRun { range, style });
    }

    fn with_source_range(mut self, source_range: Option<Range<usize>>) -> Self {
        self.source_range = source_range;
        self
    }

    fn finish(self) -> MarkdownPreviewBlock {
        MarkdownPreviewBlock {
            kind: self.kind,
            text: self.text,
            runs: self.runs,
            source_range: self.source_range,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inert_preview_discards_urls_and_active_html_but_keeps_readable_labels() {
        let source = concat!(
            "---\nprivate: metadata\n---\n",
            "# Heading\n\n",
            "A **bold** [private link](https://private.invalid/path).\n\n",
            "![private alt](https://private.invalid/image.png)\n\n",
            "<script>private()</script>\n\n",
            "- [x] Done\n",
        );
        let document = parse_inert_markdown_preview(source).unwrap();
        let visible = document
            .blocks()
            .iter()
            .map(MarkdownPreviewBlock::text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(visible.contains("Heading"));
        assert!(visible.contains("private link"));
        assert!(visible.contains("Linked image: private alt"));
        assert!(visible.contains("Raw HTML is inert"));
        assert!(visible.contains("Frontmatter remains editable source"));
        assert!(!visible.contains("https://"));
        assert!(!visible.contains("private.invalid"));
        assert!(!visible.contains("private()"));
        assert!(!document.truncated());
        let debug = format!("{document:?}");
        assert!(!debug.contains("private"));
        assert!(!debug.contains("Heading"));
    }

    #[test]
    fn checklist_items_report_a_source_range_covering_their_marker() {
        let source = "- [ ] Buy milk\n- [x] Walk the dog\n- Not a checklist\n";
        let document = parse_inert_markdown_preview(source).unwrap();
        let checklist_blocks: Vec<_> = document
            .blocks()
            .iter()
            .filter(|block| {
                matches!(
                    block.kind(),
                    MarkdownPreviewBlockKind::ListItem {
                        checked: Some(_),
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(checklist_blocks.len(), 2);

        let unchecked = checklist_blocks[0];
        assert_eq!(unchecked.text(), "Buy milk");
        let range = unchecked
            .source_range()
            .expect("unchecked item has a source range");
        // The parser's reported end offset may or may not include the
        // item's trailing newline; only the marker's position matters here.
        assert_eq!(
            source[range.clone()].trim_end_matches('\n'),
            "- [ ] Buy milk"
        );

        // Flipping just the marker inside the reported range reproduces the
        // toggle the checkbox click handler (`toggle_checklist_range`)
        // performs on the stored body: no other byte offset moves.
        let mut toggled = source.to_string();
        let marker_at = range.start + toggled[range.clone()].find("[ ]").unwrap();
        toggled.replace_range(marker_at..marker_at + 3, "[x]");
        assert_eq!(
            toggled,
            "- [x] Buy milk\n- [x] Walk the dog\n- Not a checklist\n"
        );

        let checked = checklist_blocks[1];
        assert_eq!(checked.text(), "Walk the dog");
        let range = checked
            .source_range()
            .expect("checked item has a source range");
        assert_eq!(source[range].trim_end_matches('\n'), "- [x] Walk the dog");

        let plain_item = document
            .blocks()
            .iter()
            .find(|block| block.text() == "Not a checklist")
            .expect("plain list item is present");
        assert!(matches!(
            plain_item.kind(),
            MarkdownPreviewBlockKind::ListItem { checked: None, .. }
        ));
        // A plain (non-checklist) bullet still reports its range — it is
        // simply unused by the click handler since `checked` is `None`.
        assert!(plain_item.source_range().is_some());

        // Headings and paragraphs outside any list never carry a range.
        assert_eq!(
            parse_inert_markdown_preview("# Heading\n\nParagraph.\n")
                .unwrap()
                .blocks()
                .iter()
                .filter(|block| block.source_range().is_some())
                .count(),
            0
        );
    }

    #[test]
    fn preview_retains_bullet_and_dash_markers() {
        let document = parse_inert_markdown_preview("* Bullet\n- Dash\n").unwrap();
        let markers = document
            .blocks()
            .iter()
            .filter_map(|block| match block.kind() {
                MarkdownPreviewBlockKind::ListItem {
                    unordered_marker, ..
                } => unordered_marker,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(markers, vec!['*', '-']);
    }

    #[test]
    fn preview_styles_are_non_overlapping_and_cover_visible_text() {
        let document = parse_inert_markdown_preview("**bold and *italic*** `code`").unwrap();
        let block = &document.blocks()[0];
        let mut cursor = 0;
        for run in block.runs() {
            assert_eq!(run.range().start, cursor);
            assert!(run.range().end > run.range().start);
            cursor = run.range().end;
        }
        assert_eq!(cursor, block.text().len());
        assert!(block.runs().iter().any(|run| run.style().bold));
        assert!(block.runs().iter().any(|run| run.style().italic));
        assert!(block.runs().iter().any(|run| run.style().code));
    }

    #[test]
    fn excessive_block_count_is_truncated_without_unbounded_output() {
        let source = "# heading\n".repeat(MAX_MARKDOWN_PREVIEW_BLOCKS + 50);
        let document = parse_inert_markdown_preview(&source).unwrap();
        assert!(document.truncated());
        assert_eq!(document.blocks().len(), MAX_MARKDOWN_PREVIEW_BLOCKS);
        assert!(
            document
                .blocks()
                .iter()
                .map(|block| block.text().len())
                .sum::<usize>()
                <= MAX_MARKDOWN_PREVIEW_OUTPUT_BYTES
        );
    }

    #[test]
    fn custom_markers_style_underline_highlight_superscript_and_subscript() {
        let document =
            parse_inert_markdown_preview("++under++ ==mark== x^2^ H::2::O plain").unwrap();
        let block = &document.blocks()[0];
        assert_eq!(block.text(), "under mark x2 H2O plain");
        assert!(block.runs().iter().any(|run| run.style().underline));
        assert!(block.runs().iter().any(|run| run.style().highlight));
        assert!(block.runs().iter().any(|run| run.style().superscript));
        assert!(block.runs().iter().any(|run| run.style().subscript));
        // Every byte is covered by exactly one run, like the existing
        // bold/italic/code coverage test.
        let mut cursor = 0;
        for run in block.runs() {
            assert_eq!(run.range().start, cursor);
            cursor = run.range().end;
        }
        assert_eq!(cursor, block.text().len());
    }

    #[test]
    fn an_unmatched_custom_marker_is_left_as_plain_text() {
        let document = parse_inert_markdown_preview("a ++b not closed").unwrap();
        let block = &document.blocks()[0];
        assert_eq!(block.text(), "a ++b not closed");
        assert!(block.runs().iter().all(|run| !run.style().underline));
    }

    #[test]
    fn a_lone_tilde_span_is_gfm_strikethrough_not_subscript() {
        // This crate's GFM strikethrough accepts a single `~` the same as
        // a doubled `~~` (see `CUSTOM_MARKERS`'s doc comment), so there is
        // no way to spell Subscript with `~`; both become strikethrough.
        // Subscript's own `::marker::` still works, checked separately.
        let document = parse_inert_markdown_preview("~~gone~~ H~2~O").unwrap();
        let block = &document.blocks()[0];
        assert!(block.runs().iter().any(|run| run.style().strikethrough));
        assert!(!block.runs().iter().any(|run| run.style().subscript));
        let with_subscript = parse_inert_markdown_preview("H::2::O").unwrap();
        assert!(with_subscript.blocks()[0]
            .runs()
            .iter()
            .any(|run| run.style().subscript));
    }

    #[test]
    fn a_trailing_alignment_marker_is_stripped_and_recorded_on_the_block() {
        let centered = parse_inert_markdown_preview("Hello :center:").unwrap();
        assert_eq!(centered.blocks()[0].text(), "Hello");
        assert_eq!(
            centered.blocks()[0].kind(),
            MarkdownPreviewBlockKind::Paragraph(TextAlign::Center)
        );

        let right = parse_inert_markdown_preview("## Heading :right:").unwrap();
        assert_eq!(right.blocks()[0].text(), "Heading");
        assert_eq!(
            right.blocks()[0].kind(),
            MarkdownPreviewBlockKind::Heading(2, TextAlign::Right)
        );

        let left = parse_inert_markdown_preview("No marker here").unwrap();
        assert_eq!(left.blocks()[0].text(), "No marker here");
        assert_eq!(
            left.blocks()[0].kind(),
            MarkdownPreviewBlockKind::Paragraph(TextAlign::Left)
        );
    }

    #[test]
    fn excessive_style_runs_are_truncated_without_exceeding_the_run_cap() {
        let mut block = BlockBuilder::new(MarkdownPreviewBlockKind::Paragraph(TextAlign::Left));
        let mut remaining = MAX_MARKDOWN_PREVIEW_OUTPUT_BYTES;
        for index in 0..=MAX_MARKDOWN_PREVIEW_RUNS {
            block.append(
                "x",
                MarkdownPreviewTextStyle {
                    bold: index % 2 == 0,
                    ..MarkdownPreviewTextStyle::default()
                },
                &mut remaining,
            );
        }
        assert!(block.truncated);
        assert_eq!(block.runs.len(), MAX_MARKDOWN_PREVIEW_RUNS);
    }
}
