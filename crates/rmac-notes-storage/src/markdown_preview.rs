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
    Paragraph,
    Heading(u8),
    BlockQuote,
    ListItem {
        depth: u8,
        ordered_index: Option<u32>,
        checked: Option<bool>,
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
}

impl fmt::Debug for MarkdownPreviewBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MarkdownPreviewBlock")
            .field("kind", &self.kind)
            .field("text_bytes", &self.text.len())
            .field("runs", &self.runs.len())
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
    let mut builder = PreviewBuilder::new(source.len());
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
    },
    Footnote,
}

struct PreviewBuilder {
    source_bytes: usize,
    remaining_text_bytes: usize,
    remaining_runs: usize,
    blocks: Vec<MarkdownPreviewBlock>,
    truncated: bool,
}

impl PreviewBuilder {
    fn new(source_bytes: usize) -> Self {
        Self {
            source_bytes,
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
            Node::Paragraph(paragraph) => {
                self.push_inline_block(context_kind(context), &paragraph.children, depth + 1)
            }
            Node::Heading(heading) => self.push_inline_block(
                MarkdownPreviewBlockKind::Heading(heading.depth),
                &heading.children,
                depth + 1,
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
            _ => self.push_inline_block(context_kind(context), std::slice::from_ref(node), depth),
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
        let context = BlockContext::ListItem {
            depth: list_depth,
            ordered_index,
            checked: item.checked,
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

    fn push_inline_block(&mut self, kind: MarkdownPreviewBlockKind, nodes: &[Node], depth: usize) {
        let mut block = BlockBuilder::new(kind);
        render_inline_nodes(
            nodes,
            &mut block,
            MarkdownPreviewTextStyle::default(),
            depth,
            &mut self.remaining_text_bytes,
            &mut self.truncated,
        );
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
        BlockContext::Paragraph => MarkdownPreviewBlockKind::Paragraph,
        BlockContext::BlockQuote => MarkdownPreviewBlockKind::BlockQuote,
        BlockContext::ListItem {
            depth,
            ordered_index,
            checked,
        } => MarkdownPreviewBlockKind::ListItem {
            depth,
            ordered_index,
            checked,
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
            Node::Text(text) => block.append(&text.value, style, remaining_text_bytes),
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

struct BlockBuilder {
    kind: MarkdownPreviewBlockKind,
    text: String,
    runs: Vec<MarkdownPreviewRun>,
    truncated: bool,
}

impl BlockBuilder {
    fn new(kind: MarkdownPreviewBlockKind) -> Self {
        Self {
            kind,
            text: String::new(),
            runs: Vec::new(),
            truncated: false,
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

    fn finish(self) -> MarkdownPreviewBlock {
        MarkdownPreviewBlock {
            kind: self.kind,
            text: self.text,
            runs: self.runs,
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
    fn excessive_style_runs_are_truncated_without_exceeding_the_run_cap() {
        let mut block = BlockBuilder::new(MarkdownPreviewBlockKind::Paragraph);
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
