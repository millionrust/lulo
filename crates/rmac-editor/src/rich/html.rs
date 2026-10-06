//! A [`Document`] as HTML, for the clipboard's `text/html` flavour: what
//! browsers, mail and office apps paste when they cannot read RTF.

use std::fmt::Write as _;

use super::model::{Alignment, CharStyle, Document, ListKind};

fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{2028}' => out.push_str("<br>"),
            '\u{000C}' => out.push_str("<br style=\"page-break-after:always\">"),
            c => out.push(c),
        }
    }
}

fn css(style: &CharStyle) -> String {
    let mut css = String::new();
    if let Some(family) = &style.family {
        let family: String = family
            .chars()
            .filter(|c| !matches!(c, '"' | ';' | '<' | '>'))
            .collect();
        let _ = write!(css, "font-family:'{family}';");
    }
    let _ = write!(css, "font-size:{}pt;", style.size);
    if style.bold {
        css.push_str("font-weight:bold;");
    }
    if style.italic {
        css.push_str("font-style:italic;");
    }
    match (style.underline, style.strikethrough) {
        (true, true) => css.push_str("text-decoration:underline line-through;"),
        (true, false) => css.push_str("text-decoration:underline;"),
        (false, true) => css.push_str("text-decoration:line-through;"),
        (false, false) => {}
    }
    if let Some(color) = style.color {
        let _ = write!(css, "color:#{:06x};", color.to_u32());
    }
    if let Some(color) = style.highlight {
        let _ = write!(css, "background-color:#{:06x};", color.to_u32());
    }
    if style.outline {
        css.push_str("-webkit-text-stroke:1px;color:transparent;");
    }
    if let Some(kern) = style.kern.filter(|kern| *kern != 0.0) {
        let _ = write!(css, "letter-spacing:{kern}pt;");
    }
    if style.baseline_offset != 0.0 {
        let _ = write!(css, "vertical-align:{}pt;", style.baseline_offset);
    }
    css
}

fn list_tag(kind: ListKind) -> (&'static str, &'static str) {
    match kind {
        ListKind::Numbered => ("ol", "decimal"),
        ListKind::UpperRoman => ("ol", "upper-roman"),
        ListKind::LowerRoman => ("ol", "lower-roman"),
        ListKind::UpperAlpha => ("ol", "upper-alpha"),
        ListKind::LowerAlpha => ("ol", "lower-alpha"),
        ListKind::Circle => ("ul", "circle"),
        ListKind::Square => ("ul", "square"),
        ListKind::Bullet => ("ul", "disc"),
        ListKind::Diamond => ("ul", "'\\25C6  '"),
        ListKind::Hyphen => ("ul", "'\\2043  '"),
        ListKind::Check => ("ul", "'\\2713  '"),
    }
}

/// The document as an HTML fragment with inline styles.
pub fn write(document: &Document) -> String {
    let mut out = String::with_capacity(document.len() * 2 + 64);
    out.push_str("<meta charset=\"utf-8\">");
    // Open lists, innermost last: their tags.
    let mut open: Vec<&'static str> = Vec::new();
    for paragraph in document.paragraphs() {
        let style = paragraph.style();
        let depth = style.list.map_or(0, |_| usize::from(style.list_level) + 1);
        while open.len() > depth {
            let tag = open.pop().unwrap_or("ul");
            let _ = write!(out, "</{tag}>");
        }
        if let Some(kind) = style.list {
            let (tag, marker) = list_tag(kind);
            while open.len() < depth {
                let _ = write!(out, "<{tag} style=\"list-style-type:{marker}\">");
                open.push(tag);
            }
            out.push_str("<li");
        } else {
            out.push_str("<p");
        }
        let align = match style.alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justified => "justify",
        };
        let _ = write!(
            out,
            " style=\"margin:0;white-space:pre-wrap;text-align:{align};line-height:{}\">",
            style.line_spacing
        );
        if paragraph.is_empty() {
            out.push_str("<br>");
        }
        for (range, run) in paragraph.styled_ranges() {
            let text = &paragraph.text()[range];
            if text.is_empty() {
                continue;
            }
            if let Some(link) = &run.link {
                out.push_str("<a href=\"");
                push_escaped(&mut out, link);
                out.push_str("\">");
            }
            let script = match run.superscript {
                0 => None,
                level if level > 0 => Some("sup"),
                _ => Some("sub"),
            };
            if let Some(tag) = script {
                let _ = write!(out, "<{tag}>");
            }
            let _ = write!(out, "<span style=\"{}\">", css(run));
            push_escaped(&mut out, text);
            out.push_str("</span>");
            if let Some(tag) = script {
                let _ = write!(out, "</{tag}>");
            }
            if run.link.is_some() {
                out.push_str("</a>");
            }
        }
        out.push_str(if style.list.is_some() {
            "</li>"
        } else {
            "</p>"
        });
    }
    while let Some(tag) = open.pop() {
        let _ = write!(out, "</{tag}>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn styles_links_and_lists_become_inline_html() {
        let mut document = Document::from_plain_text("a <b> & c\none\ntwo", &CharStyle::default());
        document.update_char_style(0..1, |style| style.bold = true);
        document.update_char_style(2..5, |style| {
            style.link = Some(Arc::from("https://x.test/?a=\"1\""))
        });
        document.update_paragraph_style(10..17, |style| style.list = Some(ListKind::Numbered));
        document.update_paragraph_style(14..14, |style| style.list_level = 1);
        let html = write(&document);
        assert!(html.contains("font-weight:bold;"));
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("&amp; c"));
        assert!(html.contains("<a href=\"https://x.test/?a=&quot;1&quot;\">"));
        assert!(html.contains("<ol style=\"list-style-type:decimal\"><li"));
        // The nested item opens a second list inside the first.
        assert_eq!(html.matches("<ol").count(), 2);
        assert_eq!(html.matches("</ol>").count(), 2);
    }
}
