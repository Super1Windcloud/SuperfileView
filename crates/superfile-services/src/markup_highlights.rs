//! Theme-safe display projection. Source offsets stay recoverable for Markdown actions.
use quick_xml::{Reader, events::Event};
use std::{borrow::Cow, ops::Range};

/// GPUI 0.7 paints a mark's background with the inherited editor ink. Keep
/// the hue but cap its opacity so light and dark ink both stay legible.
fn highlight(value: &str) -> Option<String> {
    let hex = value.trim();
    let named = match hex.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "ffffff",
        "blue" => "3b82f6",
        "yellow" => "facc15",
        _ => hex.strip_prefix('#')?,
    };
    if !named.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let full = match named.len() {
        3 | 4 => named.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => named.to_owned(),
        _ => return None,
    };
    let alpha = full
        .get(6..8)
        .and_then(|a| u8::from_str_radix(a, 16).ok())
        .unwrap_or(255);
    Some(format!("#{}{:02x}", &full[..6], alpha.min(64)))
}

pub(crate) fn mark_color(explicit: Option<&str>, style: Option<&str>) -> String {
    explicit
        .and_then(highlight)
        .or_else(|| {
            style?
                .split(';')
                .filter_map(|decl| decl.split_once(':'))
                .find_map(|(key, value)| {
                    if key.trim().eq_ignore_ascii_case("background-color") {
                        highlight(value)
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "#facc1540".into())
}

fn contains_mark(text: &str) -> bool {
    text.match_indices('<').any(|(start, _)| {
        text.as_bytes()
            .get(start..start + 5)
            .is_some_and(|tag| tag.eq_ignore_ascii_case(b"<mark"))
    })
}

fn mark_insertions(fragment: &str, base: usize, insertions: &mut Vec<(usize, String)>) {
    if !contains_mark(fragment) {
        return;
    }
    let mut reader = Reader::from_str(fragment);
    reader.config_mut().check_end_names = false;
    reader.config_mut().allow_unmatched_ends = true;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(tag))
                if ["script", "style", "textarea", "title"]
                    .iter()
                    .any(|n| tag.name().as_ref().eq_ignore_ascii_case(n)) =>
            {
                if reader.read_to_end(tag.name()).is_err() {
                    break;
                }
            }
            Ok(Event::Start(tag) | Event::Empty(tag))
                if tag.name().as_ref().eq_ignore_ascii_case("mark") =>
            {
                let mut explicit = None;
                let mut styled = None;
                for attr in tag.html_attributes().flatten() {
                    if attr.key.as_ref().eq_ignore_ascii_case("color") {
                        explicit = Some(attr.value);
                    } else if attr.key.as_ref().eq_ignore_ascii_case("style") {
                        styled = Some(attr.value);
                    }
                }
                let color = mark_color(explicit.as_deref(), styled.as_deref());
                // HTML uses the first attribute if an author supplied another color.
                insertions.push((base + start + 5, format!(" color=\"{color}\"")));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
}

pub fn preview(text: &str, html: bool) -> (Cow<'_, str>, Vec<Range<usize>>) {
    if !contains_mark(text) {
        return (Cow::Borrowed(text), Vec::new());
    }
    let mut insertions = Vec::new();
    if html {
        mark_insertions(text, 0, &mut insertions);
    } else {
        let mut options = markdown::ParseOptions::gfm();
        options.constructs.frontmatter = true;
        options.constructs.math_text = true;
        options.constructs.math_flow = true;
        if let Ok(root) = markdown::to_mdast(text, &options) {
            let mut nodes = vec![&root];
            while let Some(node) = nodes.pop() {
                if let markdown::mdast::Node::Html(fragment) = node
                    && let Some(pos) = &fragment.position
                {
                    mark_insertions(
                        &text[pos.start.offset..pos.end.offset],
                        pos.start.offset,
                        &mut insertions,
                    );
                } else if let Some(children) = node.children() {
                    nodes.extend(children.iter().rev());
                }
            }
        }
    }
    if insertions.is_empty() {
        return (Cow::Borrowed(text), Vec::new());
    }
    let mut output = String::with_capacity(text.len());
    let mut ranges = Vec::with_capacity(insertions.len());
    let mut offset = 0;
    for (at, attr) in insertions {
        output.push_str(&text[offset..at]);
        let start = output.len();
        output.push_str(&attr);
        ranges.push(start..output.len());
        offset = at;
    }
    output.push_str(&text[offset..]);
    (Cow::Owned(output), ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marks_preserve_hue_and_bookkeeping_across_real_markup_only() {
        let html = r##"<p title="<mark>">x</p><!-- <mark> --><script>"<mark>"</script><MARK style='background-color: #f00'>红</MARK><mark color=#abcdef03>x</mark>"##;
        let (projected, added) = preview(html, true);
        assert_eq!(added.len(), 2);
        assert!(projected.contains(r##"<MARK color="#ff000040" style="##));
        assert!(projected.contains(r##"<mark color="#abcdef03" color="##));
        let md = "`<mark>`\n\n```html\n<mark>code</mark>\n```\n\n<div><mark>yes</mark></div>\n\n- [ ] do";
        let (projected, added) = preview(md, false);
        assert_eq!(added.len(), 1);
        assert_eq!(
            projected.find("- [ ]").unwrap() - added[0].len(),
            md.find("- [ ]").unwrap()
        );
    }
}
