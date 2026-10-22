//! A small mutable XHTML tree for the AZW3 writer: parsed with quick-xml,
//! serialized the same way every time so that byte offsets computed from a
//! part of a tree agree with those of the whole.
//!
//! Names keep any prefix as written ("svg:rect", "epub:type"); namespace
//! declarations are kept as attributes. Comments, processing instructions
//! and the doctype are dropped, as kindlegen drops them.

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};

use super::entities;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// Elements serialized as `<name/>`; everything else always gets a closing
/// tag, since a self-closed `<div/>` means something else to an HTML parser.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// The name without any namespace prefix.
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

impl Element {
    pub fn new(name: impl Into<String>) -> Element {
        Element {
            name: name.into(),
            ..Element::default()
        }
    }

    pub fn local_name(&self) -> &str {
        local(&self.name)
    }

    pub fn is(&self, local_name: &str) -> bool {
        self.local_name().eq_ignore_ascii_case(local_name)
    }

    /// The value of the attribute with this local name.
    pub fn get(&self, local_name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| local(k) == local_name && !k.starts_with("xmlns"))
            .map(|(_, v)| v.as_str())
    }

    /// Sets the attribute with this local name, adding it at the end if absent.
    pub fn set(&mut self, local_name: &str, value: impl Into<String>) {
        let value = value.into();
        match self
            .attrs
            .iter_mut()
            .find(|(k, _)| local(k) == local_name && !k.starts_with("xmlns"))
        {
            Some((_, v)) => *v = value,
            None => self.attrs.push((local_name.to_owned(), value)),
        }
    }

    pub fn remove_attr(&mut self, local_name: &str) -> Option<String> {
        let pos = self
            .attrs
            .iter()
            .position(|(k, _)| local(k) == local_name && !k.starts_with("xmlns"))?;
        Some(self.attrs.remove(pos).1)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// Calls `f` on every element below this one, in document order, parents
    /// before their children.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Element)) {
        for child in self.elements_mut() {
            f(child);
            child.walk_mut(f);
        }
    }

    pub fn walk(&self, f: &mut impl FnMut(&Element)) {
        for child in self.elements() {
            f(child);
            child.walk(f);
        }
    }

    /// The first descendant (or self) with this local name.
    pub fn find(&self, local_name: &str) -> Option<&Element> {
        if self.is(local_name) {
            return Some(self);
        }
        self.elements().find_map(|e| e.find(local_name))
    }

    pub fn find_mut(&mut self, local_name: &str) -> Option<&mut Element> {
        if self.is(local_name) {
            return Some(self);
        }
        self.elements_mut().find_map(|e| e.find_mut(local_name))
    }

    /// Concatenated text of this element and its descendants.
    pub fn text(&self) -> String {
        let mut out = String::new();
        fn collect(e: &Element, out: &mut String) {
            for c in &e.children {
                match c {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(e) => collect(e, out),
                }
            }
        }
        collect(self, &mut out);
        out
    }

    /// Serializes this element and its contents.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        self.write_start(out);
        if self.children.is_empty() && VOID.contains(&self.name.as_str()) {
            out.truncate(out.len() - 1);
            out.extend(b"/>");
            return;
        }
        for child in &self.children {
            match child {
                Node::Element(e) => e.write(out),
                Node::Text(t) => escape_text(t, out),
            }
        }
        self.write_end(out);
    }

    /// `<name attr="value">`
    pub fn write_start(&self, out: &mut Vec<u8>) {
        out.push(b'<');
        out.extend(self.name.as_bytes());
        for (k, v) in &self.attrs {
            out.push(b' ');
            out.extend(k.as_bytes());
            out.extend(b"=\"");
            escape_attr(v, out);
            out.push(b'"');
        }
        out.push(b'>');
    }

    pub fn write_end(&self, out: &mut Vec<u8>) {
        out.extend(b"</");
        out.extend(self.name.as_bytes());
        out.push(b'>');
    }
}

pub fn escape_text(s: &str, out: &mut Vec<u8>) {
    for b in s.bytes() {
        match b {
            b'&' => out.extend(b"&amp;"),
            b'<' => out.extend(b"&lt;"),
            b'>' => out.extend(b"&gt;"),
            b => out.push(b),
        }
    }
}

fn escape_attr(s: &str, out: &mut Vec<u8>) {
    for b in s.bytes() {
        match b {
            b'&' => out.extend(b"&amp;"),
            b'<' => out.extend(b"&lt;"),
            b'>' => out.extend(b"&gt;"),
            b'"' => out.extend(b"&quot;"),
            b'\t' => out.extend(b"&#9;"),
            b'\n' => out.extend(b"&#10;"),
            b'\r' => out.extend(b"&#13;"),
            b => out.push(b),
        }
    }
}

/// Parses an XHTML (or SVG) document, resolving HTML's named entities such
/// as `&nbsp;` as well as XML's. Returns the root element.
pub fn parse(bytes: &[u8]) -> Result<Element, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if std::str::from_utf8(bytes).is_err() {
        return Err("not UTF-8".into());
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("invalid XML at byte {}: {e}", reader.error_position()))?;
        match event {
            Event::Start(start) => stack.push(element(&start)?),
            Event::Empty(start) => {
                let el = element(&start)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("unbalanced XML")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(text) => push_text(&mut stack, &text.xml10_content()),
            Event::CData(cdata) => push_text(&mut stack, &cdata.xml10_content()),
            Event::GeneralRef(entity) => {
                let name = entity.xml10_content();
                let resolved = match entity.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => {
                        match resolve_predefined_entity(&name).or_else(|| entities::resolve(&name))
                        {
                            Some(s) => s.to_owned(),
                            None => return Err(format!("unknown entity &{name};")),
                        }
                    }
                };
                push_text(&mut stack, &resolved);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if !stack.is_empty() {
        return Err("XML ends inside an element".into());
    }
    root.ok_or_else(|| "XML has no root element".into())
}

fn push_text(stack: &mut [Element], s: &str) {
    // Text outside the root element is only whitespace in well-formed XML.
    let Some(parent) = stack.last_mut() else {
        return;
    };
    match parent.children.last_mut() {
        Some(Node::Text(t)) => t.push_str(s),
        _ => parent.children.push(Node::Text(s.to_owned())),
    }
}

fn element(start: &BytesStart) -> Result<Element, String> {
    let mut attrs: Vec<(String, String)> = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("invalid XML attribute: {e}"))?;
        let key = attr.key.as_ref().to_owned();
        let value = unescape_attr(&attr.value)?;
        if !attrs.iter().any(|(k, _)| *k == key) {
            attrs.push((key, value));
        }
    }
    Ok(Element {
        name: start.name().as_ref().to_owned(),
        attrs,
        children: Vec::new(),
    })
}

/// Resolves references in an attribute value and normalizes whitespace
/// characters to spaces, as an XML parser does.
fn unescape_attr(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.extend(rest[..amp].chars().map(attr_space));
        let after = &rest[amp + 1..];
        let semi = after
            .find(';')
            .ok_or_else(|| format!("unterminated entity in attribute {raw:?}"))?;
        let name = &after[..semi];
        let resolved = if let Some(num) = name.strip_prefix('#') {
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16),
                None => num.parse(),
            }
            .map_err(|_| format!("bad character reference &{name};"))?;
            char::from_u32(code)
                .ok_or_else(|| format!("bad character reference &{name};"))?
                .to_string()
        } else {
            resolve_predefined_entity(name)
                .or_else(|| entities::resolve(name))
                .ok_or_else(|| format!("unknown entity &{name};"))?
                .to_owned()
        };
        out.push_str(&resolved);
        rest = &after[semi + 1..];
    }
    out.extend(rest.chars().map(attr_space));
    Ok(out)
}

fn attr_space(c: char) -> char {
    if matches!(c, '\t' | '\n' | '\r') {
        ' '
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes_round_trip() {
        let src = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\
                   <body><p class=\"a\">x &amp; &lt;y&gt; z&nbsp;!</p><br/><div/></body></html>";
        let root = parse(src.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(root.to_bytes()).unwrap(),
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T</title></head>\
             <body><p class=\"a\">x &amp; &lt;y&gt; z\u{a0}!</p><br/><div></div></body></html>"
        );
    }

    #[test]
    fn keeps_whitespace_and_drops_comments() {
        let root =
            parse(b"<?xml version='1.0'?><!DOCTYPE html><a>\n <!-- c --> <b>1</b> \n</a>").unwrap();
        assert_eq!(root.to_bytes(), b"<a>\n  <b>1</b> \n</a>");
    }

    #[test]
    fn attributes_resolve_entities_and_keep_prefixes() {
        let root = parse(br#"<a epub:type="x&amp;y" title="&#169;&eacute;"/>"#).unwrap();
        assert_eq!(root.get("type"), Some("x&y"));
        assert_eq!(root.get("title"), Some("\u{a9}\u{e9}"));
        assert_eq!(root.attrs[0].0, "epub:type");
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(parse(b"<a><b></a>").is_err());
        assert!(parse(b"<a>&bogus;</a>").is_err());
        assert!(parse(b"<a>\xff</a>").is_err());
    }
}
