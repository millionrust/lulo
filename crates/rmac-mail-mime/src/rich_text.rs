use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use url::Url;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RichText {
    pub blocks: Vec<Block>,
    /// The viewer may show a per-message action; these URLs are never fetched
    /// by parsing or rendering the model.
    pub blocked_remote_images: Vec<String>,
    pub inline_images: Vec<Image>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub content_id: String,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockKind {
    #[default]
    Paragraph,
    ListItem,
    Quote,
    TableRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Only http, https and mailto links survive sanitisation.
    pub link: Option<String>,
}

impl RichText {
    pub fn from_plain(text: &str) -> Self {
        Self {
            blocks: text
                .split('\n')
                .map(|line| Block {
                    kind: BlockKind::Paragraph,
                    spans: vec![Span {
                        text: line.to_owned(),
                        ..Span::default()
                    }],
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| {
                block
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Default)]
struct Collector {
    result: RichText,
    current: Vec<Span>,
    kind: BlockKind,
}

impl Collector {
    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.result.blocks.push(Block {
                kind: self.kind,
                spans: std::mem::take(&mut self.current),
            });
        }
        self.kind = BlockKind::Paragraph;
    }
    fn text(&mut self, text: &str, style: &Span) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.current.last_mut() {
            if last.bold == style.bold
                && last.italic == style.italic
                && last.underline == style.underline
                && last.link == style.link
            {
                last.text.push_str(text);
                return;
            }
        }
        self.current.push(Span {
            text: text.to_owned(),
            ..style.clone()
        });
    }
}

fn safe_link(value: &str) -> Option<String> {
    let url = Url::parse(value.trim()).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.to_string()),
        "mailto" if !url.path().is_empty() => Some(url.to_string()),
        _ => None,
    }
}

fn visit(node: &Handle, collector: &mut Collector, style: &Span, depth: usize) {
    if depth > 64 {
        return;
    }
    match &node.data {
        NodeData::Text { contents } => collector.text(&contents.borrow(), style),
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.as_ref();
            if matches!(
                tag,
                "script"
                    | "style"
                    | "template"
                    | "noscript"
                    | "iframe"
                    | "frame"
                    | "object"
                    | "embed"
                    | "form"
                    | "input"
                    | "button"
                    | "textarea"
                    | "select"
                    | "svg"
                    | "math"
                    | "head"
                    | "meta"
                    | "link"
            ) {
                return;
            }
            if tag == "img" {
                let attrs = attrs.borrow();
                let source = attrs
                    .iter()
                    .find(|attr| attr.name.local.as_ref() == "src")
                    .map(|attr| attr.value.to_string());
                let alt = attrs
                    .iter()
                    .find(|attr| attr.name.local.as_ref() == "alt")
                    .map(|attr| attr.value.to_string())
                    .unwrap_or_default();
                if let Some(source) = source {
                    if let Some(id) = source.strip_prefix("cid:") {
                        if !id.is_empty() {
                            collector.result.inline_images.push(Image {
                                content_id: id.to_owned(),
                                alt: alt.clone(),
                            });
                        }
                    } else if let Some(url) = safe_link(&source) {
                        if url.starts_with("http") {
                            collector.result.blocked_remote_images.push(url);
                        }
                    }
                }
                collector.text(&alt, style);
                return;
            }
            if tag == "br" {
                collector.flush();
                return;
            }
            let block = match tag {
                "p" | "div" | "section" | "article" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    Some(BlockKind::Paragraph)
                }
                "li" => Some(BlockKind::ListItem),
                "blockquote" => Some(BlockKind::Quote),
                "tr" => Some(BlockKind::TableRow),
                _ => None,
            };
            if let Some(kind) = block {
                collector.flush();
                collector.kind = kind;
            }
            let mut child_style = style.clone();
            match tag {
                "b" | "strong" => child_style.bold = true,
                "i" | "em" => child_style.italic = true,
                "u" => child_style.underline = true,
                "a" => {
                    child_style.link = attrs
                        .borrow()
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "href")
                        .and_then(|attr| safe_link(&attr.value))
                }
                _ => {}
            }
            let children = node.children.borrow();
            for child in children.iter() {
                visit(child, collector, &child_style, depth + 1);
            }
            if tag == "td" || tag == "th" {
                collector.text("  ", style);
            }
            if block.is_some() {
                collector.flush();
            }
        }
        _ => {
            let children = node.children.borrow();
            for child in children.iter() {
                visit(child, collector, style, depth + 1);
            }
        }
    }
}

/// Parse hostile HTML into an inert GPUI view model. CSS and arbitrary HTML
/// attributes are discarded, so `url()` and event handlers have no rendering
/// or networking path. Remote images are recorded but never requested.
pub fn sanitize_html(html: &str) -> RichText {
    let dom: RcDom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut collector = Collector::default();
    visit(&dom.document, &mut collector, &Span::default(), 0);
    collector.flush();
    collector.result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_corpus_is_inert() {
        for html in [
            "<script>steal()</script><p>Hello</p>",
            "<iframe src='https://evil.test/x'></iframe><p>Hello</p>",
            "<style>body{background:url(https://evil.test/t)}</style><p>Hello</p>",
            "<form action='https://evil.test'><input value='secret'></form><p>Hello</p>",
            "<p onclick='steal()' style='background:url(https://evil.test/t)'>Hello</p>",
        ] {
            let rich = sanitize_html(html);
            assert_eq!(rich.plain_text(), "Hello");
            assert!(rich.blocked_remote_images.is_empty());
        }
    }

    #[test]
    fn trackers_are_blocked_and_cid_is_local() {
        let rich = sanitize_html("<p>Hi<img src='https://tracker.test/pixel?x=1' width='1' height='1'><img src='cid:photo@local' alt='Photo'></p>");
        assert_eq!(
            rich.blocked_remote_images,
            vec!["https://tracker.test/pixel?x=1"]
        );
        assert_eq!(rich.inline_images[0].content_id, "photo@local");
        assert_eq!(rich.plain_text(), "HiPhoto");
    }

    #[test]
    fn safe_styles_links_lists_quotes_tables() {
        let rich = sanitize_html("<p><strong>Bold</strong> <a href='javascript:alert(1)'>bad</a> <a href='https://example.test/a'>good</a></p><ul><li>Item</li></ul><blockquote>Quote</blockquote><table><tr><td>A</td><td>B</td></tr></table>");
        assert!(rich.blocks[0].spans[0].bold);
        assert!(rich.blocks[0]
            .spans
            .iter()
            .any(|span| span.text == "bad" && span.link.is_none()));
        assert!(rich.blocks[0]
            .spans
            .iter()
            .any(|span| span.text == "good"
                && span.link.as_deref() == Some("https://example.test/a")));
        assert!(rich
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::ListItem));
        assert!(rich
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::Quote));
        assert!(rich
            .blocks
            .iter()
            .any(|block| block.kind == BlockKind::TableRow));
    }
}
