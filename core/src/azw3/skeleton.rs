// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/skeleton.py
// (calibre 9.16, commit 0b8c0c2).

//! Splitting each HTML file into a skeleton and chunks, as KF8 stores text.
//!
//! The body of each file is broken into chunks of at most [`CHUNK_SIZE`]
//! bytes; what is left (the `<head>`, and the tags of large elements that had
//! to be stepped into) is the skeleton. The text of the book is each
//! skeleton followed by its chunks, and the chunk table says where each chunk
//! goes back into its skeleton.

use std::collections::HashMap;

use super::dom::{Element, Node, escape_text, local};
use super::index::{ChunkEntry, Skel};
use super::oeb::XHTML_NS;
use super::utils::to_base32;
use crate::{Error, Result};

pub const CHUNK_SIZE: usize = 8192;

/// Tags to which kindlegen adds the aid attribute.
pub const AID_ABLE_TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "bdo",
    "blockquote",
    "body",
    "button",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "i",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "map",
    "mark",
    "meter",
    "nav",
    "ol",
    "output",
    "p",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "samp",
    "section",
    "select",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "textarea",
    "time",
    "ul",
    "var",
    "video",
];

/// Link targets are stored as 10 base-32 digits.
pub fn to_href(num: usize) -> String {
    to_base32(num, 10)
}

#[derive(Debug, Clone)]
struct Chunk {
    raw: Vec<u8>,
    starts_tags: Vec<String>,
    ends_tags: Vec<String>,
    insert_pos: usize,
    selector: String,
}

impl Chunk {
    fn new(raw: Vec<u8>, selector: &(char, String)) -> Chunk {
        Chunk {
            raw,
            starts_tags: Vec::new(),
            ends_tags: Vec::new(),
            insert_pos: 0,
            selector: format!("{}-//*[@aid='{}']", selector.0, selector.1),
        }
    }
}

/// The byte lengths a skeleton tag adds before its first chunk (its start
/// tag and leading whitespace) and after its last (its end tag and
/// following whitespace).
#[derive(Debug, Clone, Copy)]
struct Metric {
    start: usize,
    end: usize,
}

struct Skeleton {
    file_number: usize,
    skeleton: Vec<u8>,
    chunks: Vec<Chunk>,
    start_pos: usize,
}

impl Skeleton {
    fn new(file_number: usize, root: &Element, chunks: Vec<Chunk>) -> Skeleton {
        let skeleton = render(root);
        let body_offset = find(&skeleton, b"<body").expect("the skeleton has a body");
        let mut metrics = HashMap::new();
        calculate_metrics(root, &mut metrics);
        let mut s = Skeleton {
            file_number,
            skeleton,
            chunks,
            start_pos: 0,
        };
        let mut pos = body_offset;
        for chunk in &mut s.chunks {
            for tag in &chunk.starts_tags {
                pos += metrics[tag].start;
            }
            chunk.insert_pos = pos;
            pos += chunk.raw.len();
            for tag in &chunk.ends_tags {
                pos += metrics[tag].end;
            }
        }
        s
    }

    fn len(&self) -> usize {
        self.skeleton.len() + self.chunks.iter().map(|c| c.raw.len()).sum::<usize>()
    }

    /// The file as it was, with every chunk put back.
    fn rebuild(&self) -> Vec<u8> {
        // Each insert position counts the chunks inserted before it, so the
        // skeleton is copied up to it less what those chunks added.
        let mut ans = Vec::with_capacity(self.len());
        let mut from = 0;
        for chunk in &self.chunks {
            let to = from + (chunk.insert_pos - ans.len());
            ans.extend(&self.skeleton[from..to]);
            ans.extend(&chunk.raw);
            from = to;
        }
        ans.extend(&self.skeleton[from..]);
        ans
    }

    /// The skeleton followed by its chunks: how the text stores it.
    fn raw_text(&self) -> Vec<u8> {
        let mut ans = self.skeleton.clone();
        for c in &self.chunks {
            ans.extend(&c.raw);
        }
        ans
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The whole document, with an XML declaration and the XHTML namespace.
fn render(root: &Element) -> Vec<u8> {
    let mut out = br#"<?xml version="1.0" encoding="UTF-8"?>"#.to_vec();
    out.push(b'\n');
    let mut root = root.clone();
    root.attrs.insert(0, ("xmlns".into(), XHTML_NS.into()));
    root.write(&mut out);
    out.push(b'\n');
    out
}

fn text_len(s: &str) -> usize {
    let mut out = Vec::new();
    escape_text(s, &mut out);
    out.len()
}

fn calculate_metrics(parent: &Element, metrics: &mut HashMap<String, Metric>) {
    for (k, node) in parent.children.iter().enumerate() {
        let Node::Element(tag) = node else { continue };
        if let Some(aid) = tag.get("aid") {
            let mut open = Vec::new();
            tag.write_start(&mut open);
            let leading = match tag.children.first() {
                Some(Node::Text(t)) => text_len(t),
                _ => 0,
            };
            let mut close = Vec::new();
            tag.write_end(&mut close);
            let tail = match parent.children.get(k + 1) {
                Some(Node::Text(t)) => text_len(t),
                _ => 0,
            };
            metrics.insert(
                aid.to_owned(),
                Metric {
                    start: open.len() + leading,
                    end: close.len() + tail,
                },
            );
        }
        calculate_metrics(tag, metrics);
    }
}

pub struct Chunker {
    pub skel_table: Vec<Skel>,
    pub chunk_table: Vec<ChunkEntry>,
    /// aid → (chunk sequence number, offset in chunk, offset in text).
    pub aid_offset_map: HashMap<String, (usize, usize, usize)>,
    pub text: Vec<u8>,
}

/// Where the next chunks go: 'P' for inside the tag with the given aid, 'S'
/// for after it.
type Selector = (char, String);

impl Chunker {
    /// `docs` are the spine's documents with aids inserted;
    /// `placeholder_map` maps link placeholders to the aid they point to.
    pub fn new(docs: &[Element], placeholder_map: &HashMap<String, String>) -> Result<Chunker> {
        let mut skeletons = Vec::new();
        for (i, doc) in docs.iter().enumerate() {
            let mut root = remove_namespaces(doc);
            set_body_tail(&mut root);
            let mut chunks = Vec::new();
            let mut selector: Selector = ('P', String::new());
            let body = root.find_mut("body").expect("documents have a body");
            // First pass: break the body up into pieces of at most CHUNK_SIZE.
            step_into_tag(body, &mut chunks, &mut selector)?;
            // Second pass: merge neighbouring small chunks within the same
            // skeleton tag, to get chunks as close to CHUNK_SIZE as possible.
            let chunks = merge_small_chunks(chunks);
            // Third pass: the skeleton and each chunk's insert position.
            skeletons.push(Skeleton::new(i, &root, chunks));
        }

        let mut sp = 0;
        for s in &mut skeletons {
            s.start_pos = sp;
            sp += s.len();
        }
        let skel_table = skeletons
            .iter()
            .map(|s| Skel {
                file_number: s.file_number,
                chunk_count: s.chunks.len(),
                start_pos: s.start_pos,
                length: s.skeleton.len(),
            })
            .collect();
        let mut chunk_table = Vec::new();
        for s in &skeletons {
            let mut cp = 0;
            for c in &s.chunks {
                chunk_table.push(ChunkEntry {
                    insert_pos: c.insert_pos + s.start_pos,
                    selector: c.selector.clone(),
                    file_number: s.file_number,
                    sequence_number: chunk_table.len(),
                    start_pos: cp,
                    length: c.raw.len(),
                });
                cp += c.raw.len();
            }
        }

        let text: Vec<u8> = skeletons.iter().flat_map(Skeleton::raw_text).collect();
        let rebuilt: Vec<u8> = skeletons.iter().flat_map(Skeleton::rebuild).collect();
        let mut chunker = Chunker {
            skel_table,
            chunk_table,
            aid_offset_map: HashMap::new(),
            text: Vec::new(),
        };
        chunker.text = chunker.set_internal_links(text, &rebuilt, placeholder_map)?;
        Ok(chunker)
    }

    /// Points the link placeholders at their targets: a chunk number and an
    /// offset from the start of that chunk to the target tag.
    fn set_internal_links(
        &mut self,
        mut text: Vec<u8>,
        rebuilt: &[u8],
        placeholder_map: &HashMap<String, String>,
    ) -> Result<Vec<u8>> {
        for (offset, aid) in aids_in(rebuilt) {
            let mut pos_fid = None;
            for (n, chunk) in self.chunk_table.iter().enumerate() {
                if chunk.insert_pos <= offset && offset < chunk.insert_pos + chunk.length {
                    pos_fid = Some((chunk.sequence_number, offset - chunk.insert_pos, offset));
                    break;
                }
                if chunk.insert_pos > offset {
                    // The aid is in the skeleton, so use the chunk right after.
                    pos_fid = Some((chunk.sequence_number, 0, offset));
                    break;
                }
                if n == self.chunk_table.len() - 1 {
                    // An aid very close to the end of the text.
                    pos_fid = Some((chunk.sequence_number, offset - chunk.insert_pos, offset));
                }
            }
            let pos_fid =
                pos_fid.ok_or_else(|| Error::Unreadable(format!("no chunk for aid {aid}")))?;
            self.aid_offset_map.insert(aid, pos_fid);
        }

        let mut replacements = HashMap::new();
        for (placeholder, aid) in placeholder_map {
            if let Some(&(pos, fid, _)) = self.aid_offset_map.get(aid) {
                replacements.insert(
                    placeholder.as_bytes().to_vec(),
                    format!("{}:off:{}", to_base32(pos, 4), to_href(fid)),
                );
            }
        }
        // Placeholders and their replacements are the same length, so no
        // offset changes.
        const PREFIX: &[u8] = b"kindle:pos:fid:";
        let mut i = 0;
        while let Some(found) = find(&text[i..], PREFIX) {
            let start = i + found;
            let end = start + PREFIX.len() + 19;
            if end <= text.len()
                && let Some(new) = replacements.get(&text[start..end])
            {
                text[start + PREFIX.len()..end].copy_from_slice(new.as_bytes());
            }
            i = start + PREFIX.len();
        }
        Ok(text)
    }
}

/// The offset of each tag carrying an aid or cid in `text`, with its value.
fn aids_in(text: &[u8]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(lt) = text[i..].iter().position(|&b| b == b'<') {
        let start = i + lt;
        let end = text[start..]
            .iter()
            .position(|&b| b == b'>')
            .map_or(text.len(), |e| start + e);
        let tag = &text[start..end];
        let hit = [&b" aid=\""[..], b" cid=\""]
            .iter()
            .filter_map(|pat| find(tag, pat))
            .min();
        if let Some(at) = hit {
            let value_start = at + 6;
            if let Some(len) = tag[value_start..].iter().position(|&b| b == b'"') {
                let value = &tag[value_start..value_start + len];
                if !value.is_empty()
                    && value
                        .iter()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'c')
                {
                    out.push((start, String::from_utf8_lossy(value).into_owned()));
                }
            }
        }
        i = end.max(start + 1);
    }
    out
}

/// A copy of `root` with every namespace removed, so that it serializes as
/// clean HTML 5 markup; the XHTML namespace goes back on at serialization.
/// Only `lang`, `class` and `style` are kept on the root.
pub fn remove_namespaces(root: &Element) -> Element {
    let mut nroot = Element::new("html");
    if let Some((_, lang)) = root.attrs.iter().rev().find(|(k, _)| local(k) == "lang") {
        nroot.attrs.push(("lang".into(), lang.clone()));
    }
    for name in ["class", "style"] {
        if let Some((_, v)) = root.attrs.iter().find(|(k, _)| k == name) {
            nroot.attrs.push((name.into(), v.clone()));
        }
    }
    nroot.children = strip_children(&root.children);
    nroot
}

fn strip_children(children: &[Node]) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(children.len());
    for child in children {
        match child {
            Node::Text(t) => match out.last_mut() {
                Some(Node::Text(prev)) => prev.push_str(t),
                _ => out.push(Node::Text(t.clone())),
            },
            Node::Element(e) => {
                let mut attrs: Vec<(String, String)> = Vec::new();
                for (k, v) in &e.attrs {
                    if k == "xmlns" || k.starts_with("xmlns:") {
                        continue;
                    }
                    let k = local(k);
                    match attrs.iter_mut().find(|(n, _)| n == k) {
                        Some((_, old)) => *old = v.clone(),
                        None => attrs.push((k.to_owned(), v.clone())),
                    }
                }
                // kindlegen always puts the aid last.
                if let Some(pos) = attrs.iter().position(|(k, _)| k == "aid") {
                    let aid = attrs.remove(pos);
                    attrs.push(aid);
                }
                out.push(Node::Element(Element {
                    name: local(&e.name).to_owned(),
                    attrs,
                    children: strip_children(&e.children),
                }));
            }
        }
    }
    out
}

/// One newline after `</body>`, whatever followed it.
fn set_body_tail(root: &mut Element) {
    if let Some(pos) = root
        .children
        .iter()
        .position(|n| matches!(n, Node::Element(e) if e.name == "body"))
    {
        match root.children.get_mut(pos + 1) {
            Some(Node::Text(t)) => *t = "\n".into(),
            _ => root.children.insert(pos + 1, Node::Text("\n".into())),
        }
    }
}

/// What happened to the element before a text node.
#[derive(Clone, Copy, PartialEq)]
enum Before {
    /// Nothing: the text is the start of the parent's content.
    Nothing,
    /// The element went into a chunk; its tail goes with it.
    Chunked,
    /// The element stays in the skeleton; whitespace after it stays too.
    SteppedInto,
}

fn step_into_tag(
    tag: &mut Element,
    chunks: &mut Vec<Chunk>,
    selector: &mut Selector,
) -> Result<()> {
    let aid = tag.get("aid").unwrap_or_default().to_owned();
    *selector = ('P', aid.clone());
    let first_chunk_idx = chunks.len();

    let mut kept = Vec::new();
    let mut before = Before::Nothing;
    for node in std::mem::take(&mut tag.children) {
        match node {
            Node::Text(t) => {
                let blank = t.trim().is_empty();
                if before == Before::Chunked && !t.is_empty() || before != Before::Chunked && !blank
                {
                    chunks.extend(chunk_up_text(&t, selector));
                } else {
                    // Pure whitespace stays in the skeleton.
                    kept.push(Node::Text(t));
                }
            }
            Node::Element(mut child) => {
                let raw = child.to_bytes();
                if raw.len() > CHUNK_SIZE && child.get("aid").is_some() {
                    step_into_tag(&mut child, chunks, selector)?;
                    kept.push(Node::Element(child));
                    before = Before::SteppedInto;
                } else {
                    chunks.push(Chunk::new(raw, selector));
                    before = Before::Chunked;
                }
            }
        }
    }
    tag.children = kept;

    if chunks.len() <= first_chunk_idx {
        if first_chunk_idx > 0 {
            return Err(Error::Unreadable(format!(
                "cannot split the <{}> element into chunks",
                tag.name
            )));
        }
    } else {
        chunks[first_chunk_idx].starts_tags.push(aid.clone());
        chunks.last_mut().unwrap().ends_tags.push(aid.clone());
    }
    *selector = ('S', aid);
    Ok(())
}

/// Escaped `text` in pieces of at most CHUNK_SIZE bytes, never splitting a
/// character or a character reference. Pieces after the first are wrapped in
/// a span, as kindlegen does.
fn chunk_up_text(text: &str, selector: &Selector) -> Vec<Chunk> {
    let mut escaped = Vec::new();
    escape_text(text, &mut escaped);
    let mut ans = Vec::new();
    let mut rest = &escaped[..];
    let mut first = true;
    while first || !rest.is_empty() {
        let mut cut = rest.len().min(CHUNK_SIZE);
        if cut < rest.len() {
            while cut > 0 && rest[cut] & 0xC0 == 0x80 {
                cut -= 1;
            }
            if let Some(amp) = rest[..cut].iter().rposition(|&b| b == b'&')
                && !rest[amp..cut].contains(&b';')
            {
                cut = amp;
            }
        }
        let (piece, tail) = rest.split_at(cut);
        let raw = if first {
            piece.to_vec()
        } else {
            [&b"<span class=\"AmznBigTextBlock\">"[..], piece, b"</span>"].concat()
        };
        ans.push(Chunk::new(raw, selector));
        rest = tail;
        first = false;
    }
    ans
}

fn merge_small_chunks(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut ans: Vec<Chunk> = Vec::new();
    for chunk in chunks {
        match ans.last_mut() {
            Some(prev)
                if chunk.starts_tags.is_empty()
                    && chunk.raw.len() + prev.raw.len() <= CHUNK_SIZE
                    && prev.ends_tags.is_empty() =>
            {
                prev.raw.extend(&chunk.raw);
                prev.ends_tags = chunk.ends_tags;
            }
            _ => ans.push(chunk),
        }
    }
    ans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azw3::dom;

    /// Gives every aid-able element an aid, as the writer does.
    fn with_aids(src: &str) -> Element {
        let mut root = dom::parse(src.as_bytes()).unwrap();
        let mut j = 0;
        root.walk_mut(&mut |e| {
            if AID_ABLE_TAGS.contains(&e.local_name()) {
                e.set("aid", to_base32(j, 1));
                j += 1;
            }
        });
        root
    }

    fn chunk(docs: &[Element]) -> (Chunker, Vec<Vec<u8>>) {
        let originals = docs
            .iter()
            .map(|d| {
                let mut root = remove_namespaces(d);
                set_body_tail(&mut root);
                render(&root)
            })
            .collect();
        (Chunker::new(docs, &HashMap::new()).unwrap(), originals)
    }

    /// Puts the text back together from the tables, as a reader would.
    fn reassemble(c: &Chunker) -> Vec<Vec<u8>> {
        c.skel_table
            .iter()
            .map(|s| {
                let mut file = c.text[s.start_pos..s.start_pos + s.length].to_vec();
                let mut chunk_start = s.start_pos + s.length;
                for ch in c
                    .chunk_table
                    .iter()
                    .filter(|ch| ch.file_number == s.file_number)
                {
                    let raw = &c.text[chunk_start..chunk_start + ch.length];
                    let at = ch.insert_pos - s.start_pos;
                    file.splice(at..at, raw.iter().copied());
                    chunk_start += ch.length;
                }
                file
            })
            .collect()
    }

    #[test]
    fn small_documents_are_one_chunk() {
        let doc = with_aids(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\n<body>\n<p>One</p>\n<p>Two</p>\n</body></html>",
        );
        let (c, originals) = chunk(&[doc]);
        assert_eq!(c.chunk_table.len(), 1);
        assert_eq!(c.chunk_table[0].selector, "P-//*[@aid='0']");
        assert_eq!(reassemble(&c), originals);
        let text = String::from_utf8(c.text.clone()).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\n<body aid=\"0\">\n</body>\n</html>\n"));
    }

    #[test]
    fn large_documents_split_and_reassemble() {
        let para = |i: usize| {
            format!(
                "<p id=\"p{i}\">Paragraph {i} {}</p>\n",
                "lorem ipsum ".repeat(30)
            )
        };
        let mut body = String::new();
        for i in 0..60 {
            body.push_str(&para(i));
        }
        let mut nested = String::from("<div>\n");
        for i in 60..120 {
            nested.push_str(&para(i));
        }
        nested.push_str("</div>\n");
        let long_text = "word &amp; ".repeat(2000);
        let src = format!(
            "<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\"><head><title>T</title></head><body>\n{body}{nested}<p>{long_text}</p>tail text</body></html>"
        );
        let docs = [with_aids(&src), with_aids(&src)];
        let (c, originals) = chunk(&docs);
        assert!(c.chunk_table.len() > 10);
        assert!(c.chunk_table.iter().all(|ch| ch.length <= CHUNK_SIZE + 64));
        assert!(c.chunk_table.iter().any(|ch| ch.selector.starts_with("S-")));
        // Long text is split into spans (the document has no others).
        let unwrapped: Vec<Vec<u8>> = reassemble(&c)
            .iter()
            .map(|f| {
                String::from_utf8_lossy(f)
                    .replace("<span class=\"AmznBigTextBlock\">", "")
                    .replace("</span>", "")
                    .into_bytes()
            })
            .collect();
        assert_eq!(unwrapped, originals);
        assert!(originals[0].starts_with(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\">"));
        assert_eq!(c.skel_table[1].start_pos, reassemble(&c)[0].len());

        // Every aid is found, and its offset points at its tag.
        let rebuilt: Vec<u8> = reassemble(&c).concat();
        for (aid, &(_, _, offset)) in &c.aid_offset_map {
            let tag = &rebuilt[offset..offset + 200.min(rebuilt.len() - offset)];
            assert!(
                find(tag, format!("aid=\"{aid}\"").as_bytes()).is_some(),
                "{aid}"
            );
        }
    }

    #[test]
    fn links_are_pointed_at_their_targets() {
        let mut doc = with_aids(
            "<html><head/><body><p><a href=\"x\">link</a></p><p id=\"t\">target</p></body></html>",
        );
        let placeholder = format!("kindle:pos:fid:0000:off:{}", to_href(1));
        doc.walk_mut(&mut |e| {
            if e.is("a") {
                e.set("href", placeholder.clone());
            }
        });
        let target_aid = "3".to_owned(); // body 0, p 1, a 2, p 3
        let map = HashMap::from([(placeholder, target_aid)]);
        let c = Chunker::new(&[doc], &map).unwrap();
        let text = String::from_utf8(c.text).unwrap();
        let (pos, fid, _) = c.aid_offset_map["3"];
        assert!(text.contains(&format!(
            "href=\"kindle:pos:fid:{}:off:{}\"",
            to_base32(pos, 4),
            to_href(fid)
        )));
    }

    #[test]
    fn long_text_is_never_split_inside_a_reference() {
        let text = format!("{}&", "a".repeat(CHUNK_SIZE - 2));
        let chunks = chunk_up_text(&text, &('P', "0".into()));
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].raw.len(), CHUNK_SIZE - 2);
        assert_eq!(
            chunks[1].raw,
            b"<span class=\"AmznBigTextBlock\">&amp;</span>"
        );
    }
}
