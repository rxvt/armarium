// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/toc.py
// (calibre 9.16, commit 0b8c0c2).

//! An HTML Table of Contents at the end of the book, which the Kindle's
//! "Table of Contents" menu item opens, unless the book has its own.

use super::dom::{self, Element, Node};
use super::oeb::{Data, GuideRef, Item, Oeb, XHTML_NS, strip_fragment};

const TITLE: &str = "Table of Contents";
const STYLE: &str =
    "\n  li { list-style-type: none }\n  a { text-decoration: none }\n  a:hover { color: red }\n  ";

/// Adds the inline Table of Contents if the book needs one.
pub fn add_inline_toc(oeb: &mut Oeb) {
    let has_toc = oeb.toc.count() > 1;

    if let Some(pos) = oeb
        .guide
        .iter()
        .position(|g| g.kind.eq_ignore_ascii_case("toc"))
    {
        // Keep the book's own ToC if it has links, making sure it is in the
        // spine; otherwise drop the guide entry.
        let href = strip_fragment(&oeb.guide[pos].href).to_owned();
        if let Some(i) = oeb.item(&href) {
            if let Data::Raw(bytes) = &oeb.items[i].data
                && oeb.items[i].media_type.contains("html")
                && let Ok(root) = dom::parse(bytes)
            {
                oeb.items[i].data = Data::Html(root);
            }
            if let Data::Html(root) = &oeb.items[i].data
                && has_links(root)
            {
                if !oeb.spine.contains(&i) {
                    oeb.spine.push(i);
                }
                return;
            }
        }
        oeb.guide.remove(pos);
    }
    if !has_toc {
        return;
    }

    let mut ul = Element::new("ul");
    ul.children.push(Node::Text("\n\t".into()));
    for node in &oeb.toc.children {
        process_node(node, &mut ul, 0);
    }
    let html = element(
        "html",
        &[("xmlns", XHTML_NS)],
        vec![
            text("\n"),
            element(
                "head",
                &[],
                vec![
                    text("\n  "),
                    element("title", &[], vec![text(TITLE)]),
                    text("\n  "),
                    element("style", &[("type", "text/css")], vec![text(STYLE)]),
                    text("\n"),
                ],
            ),
            text("\n"),
            element(
                "body",
                &[("id", "calibre_generated_inline_toc")],
                vec![
                    text("\n"),
                    element("h2", &[], vec![text(TITLE)]),
                    text("\n"),
                    Node::Element(ul),
                    text("\n"),
                ],
            ),
            text("\n"),
        ],
    );
    let Node::Element(root) = html else {
        unreachable!()
    };

    let mut href = "contents.xhtml".to_owned();
    let mut n = 0;
    while oeb.item(&href).is_some() {
        n += 1;
        href = format!("contents{n}.xhtml");
    }
    oeb.items.push(Item {
        id: format!("calibre_inline_toc_{n}"),
        href: href.clone(),
        media_type: "application/xhtml+xml".into(),
        data: Data::Html(root),
    });
    oeb.spine.push(oeb.items.len() - 1);
    oeb.guide.push(GuideRef {
        kind: "toc".into(),
        title: TITLE.into(),
        href,
    });
}

fn has_links(root: &Element) -> bool {
    let mut found = false;
    root.walk(&mut |e| found |= e.is("a") && e.get("href").is_some());
    found
}

fn text(s: &str) -> Node {
    Node::Text(s.to_owned())
}

fn element(name: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
    Node::Element(Element {
        name: name.into(),
        attrs: attrs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        children,
    })
}

/// The generated page sits at the top of the EPUB, so a link to an item is
/// its path, with `%` escaped since links are percent-decoded when read.
fn link(href: &str) -> String {
    href.replace('%', "%25")
}

fn process_node(node: &super::oeb::TocNode, parent: &mut Element, level: usize) {
    let mut li = Element::new("li");
    let mut a = Element::new("a");
    a.attrs
        .push(("href".into(), node.href.as_deref().map_or("#".into(), link)));
    a.children.push(Node::Text(node.title.clone()));
    li.children.push(Node::Element(a));
    if !node.children.is_empty() {
        li.children
            .push(Node::Text(format!("\n{}", "\t".repeat(level))));
        let mut ul = Element::new("ul");
        ul.children
            .push(Node::Text(format!("\n{}", "\t".repeat(level + 1))));
        for child in &node.children {
            process_node(child, &mut ul, level + 1);
        }
        li.children.push(Node::Element(ul));
        li.children
            .push(Node::Text(format!("\n{}", "\t".repeat(level))));
    }
    parent.children.push(Node::Element(li));
    parent
        .children
        .push(Node::Text(format!("\n{}", "\t".repeat(level))));
}
