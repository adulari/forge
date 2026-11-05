use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct MdSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MdBlock {
    Paragraph { spans: Vec<MdSpan> },
    Heading { level: u8, spans: Vec<MdSpan> },
    Code { lang: String, code: String },
    ListItem { depth: u8, marker: String, spans: Vec<MdSpan> },
    Quote { spans: Vec<MdSpan> },
    Rule,
}

#[derive(PartialEq)]
enum Ctx {
    Para,
    Heading(u8),
    Item,
    Quote,
}

pub fn parse(src: &str) -> Vec<MdBlock> {
    let mut out = Vec::new();
    let mut spans: Vec<MdSpan> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut link: Option<String> = None;
    let mut ctx: Vec<Ctx> = Vec::new();
    // (ordered counter or None for bullet)
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut markers: Vec<String> = Vec::new();
    let mut code: Option<(String, String)> = None;

    let flush = |spans: &mut Vec<MdSpan>, out: &mut Vec<MdBlock>, kind: &Ctx, depth: u8, marker: &str| {
        if spans.is_empty() {
            return;
        }
        let s = std::mem::take(spans);
        out.push(match kind {
            Ctx::Para => MdBlock::Paragraph { spans: s },
            Ctx::Heading(l) => MdBlock::Heading { level: *l, spans: s },
            Ctx::Item => MdBlock::ListItem { depth, marker: marker.to_string(), spans: s },
            Ctx::Quote => MdBlock::Quote { spans: s },
        });
    };

    for ev in Parser::new_ext(src, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Start(Tag::Paragraph) => ctx.push(match ctx.last() {
                Some(Ctx::Quote) => Ctx::Quote,
                Some(Ctx::Item) => Ctx::Item,
                _ => Ctx::Para,
            }),
            Event::End(TagEnd::Paragraph) => {
                let k = ctx.pop().unwrap_or(Ctx::Para);
                let (d, m) = (lists.len().saturating_sub(1) as u8, markers.last().cloned().unwrap_or_default());
                flush(&mut spans, &mut out, &k, d, &m);
            }
            Event::Start(Tag::Heading { level, .. }) => ctx.push(Ctx::Heading(match level {
                HeadingLevel::H1 => 1,
                HeadingLevel::H2 => 2,
                _ => 3,
            })),
            Event::End(TagEnd::Heading(_)) => {
                let k = ctx.pop().unwrap_or(Ctx::Para);
                flush(&mut spans, &mut out, &k, 0, "");
            }
            Event::Start(Tag::BlockQuote(_)) => ctx.push(Ctx::Quote),
            Event::End(TagEnd::BlockQuote(_)) => {
                let k = ctx.pop().unwrap_or(Ctx::Quote);
                flush(&mut spans, &mut out, &k, 0, "");
            }
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                let m = match lists.last_mut() {
                    Some(Some(n)) => {
                        let s = format!("{n}.");
                        *n += 1;
                        s
                    }
                    _ => "•".to_string(),
                };
                markers.push(m);
                ctx.push(Ctx::Item);
            }
            Event::End(TagEnd::Item) => {
                let k = ctx.pop().unwrap_or(Ctx::Item);
                let m = markers.pop().unwrap_or_default();
                flush(&mut spans, &mut out, &k, lists.len().saturating_sub(1) as u8, &m);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, c)) = code.take() {
                    out.push(MdBlock::Code { lang, code: c.trim_end_matches('\n').to_string() });
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => link = None,
            Event::Text(t) => {
                if let Some((_, c)) = code.as_mut() {
                    c.push_str(&t);
                } else {
                    spans.push(MdSpan { text: t.to_string(), bold: bold > 0, italic: italic > 0, code: false, link: link.clone() });
                }
            }
            Event::Code(t) => spans.push(MdSpan { text: t.to_string(), bold: bold > 0, italic: italic > 0, code: true, link: link.clone() }),
            Event::SoftBreak | Event::HardBreak => spans.push(MdSpan { text: "\n".into(), ..Default::default() }),
            Event::Rule => out.push(MdBlock::Rule),
            _ => {}
        }
    }
    if let Some((lang, c)) = code.take() {
        // Unterminated fence while streaming: show what we have as code.
        out.push(MdBlock::Code { lang, code: c.trim_end_matches('\n').to_string() });
    }
    if !spans.is_empty() {
        out.push(MdBlock::Paragraph { spans });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_in_order() {
        let b = parse("## T\n\nhello **bold** `c`\n\n- a\n- b\n\n```rust\nfn x(){}\n```\n\n> q");
        assert!(matches!(b[0], MdBlock::Heading { level: 2, .. }));
        assert!(matches!(&b[1], MdBlock::Paragraph { spans } if spans.iter().any(|s| s.bold && s.text == "bold") && spans.iter().any(|s| s.code)));
        assert!(matches!(&b[2], MdBlock::ListItem { marker, .. } if marker == "•"));
        assert!(matches!(&b[4], MdBlock::Code { lang, code } if lang == "rust" && code == "fn x(){}"));
        assert!(matches!(b[5], MdBlock::Quote { .. }));
    }

    #[test]
    fn unterminated_fence_streams_as_code() {
        let b = parse("text\n\n```py\nprint(1)");
        assert!(matches!(b.last(), Some(MdBlock::Code { code, .. }) if code == "print(1)"));
    }
}
