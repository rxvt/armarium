//! A minimal XML element tree over quick-xml, for small documents such as an
//! EPUB's container.xml and OPF. Namespaces are dropped: elements and
//! attributes are known by their local names ("dc:title" → "title",
//! "opf:file-as" → "file-as").

use quick_xml::Reader;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// All text inside this element, children included, in document order.
    pub text: String,
}

impl Element {
    pub fn attr(&self, local_name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(name, _)| name == local_name)
            .map(|(_, value)| value.as_str())
    }

    pub fn children_named<'a, 'n>(
        &'a self,
        name: &'n str,
    ) -> impl Iterator<Item = &'a Element> + use<'a, 'n> {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children_named(name).next()
    }

    /// [`Element::text`] with runs of whitespace collapsed to single spaces
    /// and the ends trimmed.
    pub fn text_content(&self) -> String {
        self.text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// Parses `bytes` (UTF-8) and returns the root element.
pub fn parse(bytes: &[u8]) -> Result<Element, String> {
    let mut reader = Reader::from_reader(bytes);
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
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("unbalanced XML")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(el),
                    None => root = Some(el),
                }
            }
            Event::Text(text) => push_text(&mut stack, &text.xml10_content()),
            Event::CData(cdata) => push_text(&mut stack, &cdata.xml10_content()),
            Event::GeneralRef(entity) => {
                let name = entity.xml10_content();
                match entity.resolve_char_ref() {
                    Ok(Some(c)) => push_text(&mut stack, c.encode_utf8(&mut [0; 4])),
                    _ => match resolve_predefined_entity(&name) {
                        Some(s) => push_text(&mut stack, s),
                        // An undeclared entity such as &nbsp;: keep it as written.
                        None => push_text(&mut stack, &format!("&{name};")),
                    },
                }
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

/// Text belongs to every open element, so each one's `text` is complete.
fn push_text(stack: &mut [Element], s: &str) {
    for el in stack {
        el.text.push_str(s);
    }
}

fn element(start: &BytesStart) -> Result<Element, String> {
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|e| format!("invalid XML attribute: {e}"))?;
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| format!("invalid XML attribute: {e}"))?;
        attrs.push((
            attr.key.local_name().as_ref().to_owned(),
            value.into_owned(),
        ));
    }
    Ok(Element {
        name: start.local_name().as_ref().to_owned(),
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_elements_attributes_and_text_by_local_name() {
        let root = parse(
            br#"<?xml version="1.0"?>
            <package xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="x">
              <metadata>
                <dc:title>Pride &amp; Prejudice&#x21;</dc:title>
                <dc:creator opf:file-as="Austen, Jane"><![CDATA[Jane]]> Austen</dc:creator>
                <meta name="cover" content="img"/>
              </metadata>
            </package>"#,
        )
        .unwrap();

        assert_eq!(root.name, "package");
        let metadata = root.child("metadata").unwrap();
        assert_eq!(metadata.child("title").unwrap().text, "Pride & Prejudice!");
        let creator = metadata.child("creator").unwrap();
        assert_eq!(creator.text, "Jane Austen");
        assert_eq!(creator.attr("file-as"), Some("Austen, Jane"));
        assert_eq!(metadata.child("meta").unwrap().attr("content"), Some("img"));
    }

    #[test]
    fn text_content_includes_children_and_collapses_whitespace() {
        let root = parse(b"<d>  A <b>bold</b>\n\n move </d>").unwrap();
        assert_eq!(root.text_content(), "A bold move");
    }

    #[test]
    fn keeps_unknown_entities_and_rejects_broken_xml() {
        assert_eq!(parse(b"<a>x&nbsp;y</a>").unwrap().text, "x&nbsp;y");
        assert!(parse(b"<a><b></a>").is_err());
        assert!(parse(b"<a>").is_err());
        assert!(parse(b"").is_err());
    }
}
