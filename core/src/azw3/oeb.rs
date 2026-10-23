//! An EPUB read into memory for conversion: its manifest, spine, Table of
//! Contents and guide, with HTML documents parsed. This plays the part of
//! calibre's OEB book and EPUB input plugin, much reduced: no CSS flattening,
//! no structure detection, no repair of broken markup.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use super::dom::{self, Element};
use crate::extract::epub::{cover_href, percent_decode, read_entry, resolve_href, rootfile_path};
use crate::xml;
use crate::{Error, Result};

/// Generous caps so a hostile archive cannot make us allocate without bound.
const MAX_ITEM_BYTES: u64 = 64 << 20;
const MAX_TOTAL_BYTES: u64 = 1 << 30;

pub const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

#[derive(Debug, Clone)]
pub enum Data {
    Html(Element),
    Css(String),
    Svg(Element),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    /// Path inside the EPUB, decoded; also how other items refer to it.
    pub href: String,
    pub media_type: String,
    pub data: Data,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TocNode {
    pub title: String,
    /// Item href plus any `#fragment`.
    pub href: Option<String>,
    pub children: Vec<TocNode>,
}

impl TocNode {
    /// Number of nodes below this one.
    pub fn count(&self) -> usize {
        self.children.iter().map(|c| 1 + c.count()).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuideRef {
    pub kind: String,
    pub title: String,
    /// Item href plus any `#fragment`.
    pub href: String,
}

#[derive(Debug, Clone)]
pub struct Oeb {
    pub items: Vec<Item>,
    /// Indices into `items`.
    pub spine: Vec<usize>,
    /// The root's children are the top-level entries.
    pub toc: TocNode,
    pub guide: Vec<GuideRef>,
    pub cover_image: Option<usize>,
    pub page_progression_direction: Option<String>,
    pub primary_writing_mode: Option<String>,
    pub uuid: Option<String>,
    /// Fonts under EPUB font obfuscation, which are left out.
    pub obfuscated: HashSet<String>,
}

fn unreadable(reason: impl Into<String>) -> Error {
    Error::Unreadable(format!("EPUB: {}", reason.into()))
}

impl Oeb {
    pub fn read(path: &Path) -> Result<Oeb> {
        let mut archive = ZipArchive::new(File::open(path)?)
            .map_err(|e| unreadable(format!("not a ZIP archive: {e}")))?;
        let container = xml::parse(&read_entry(
            &mut archive,
            "META-INF/container.xml",
            MAX_ITEM_BYTES,
        )?)
        .map_err(|e| unreadable(format!("bad container.xml: {e}")))?;
        let opf_path = rootfile_path(&container)
            .ok_or_else(|| unreadable("container.xml names no package document"))?;
        let package = xml::parse(&read_entry(&mut archive, &opf_path, MAX_ITEM_BYTES)?)
            .map_err(|e| unreadable(format!("bad package document: {e}")))?;

        let obfuscated = read_encryption(&mut archive);
        let manifest = package
            .child("manifest")
            .ok_or_else(|| unreadable("package document has no manifest"))?;
        let spine_el = package
            .child("spine")
            .ok_or_else(|| unreadable("package document has no spine"))?;
        let spine_ids: Vec<&str> = spine_el
            .children_named("itemref")
            .filter_map(|r| r.attr("idref"))
            .collect();

        let mut items = Vec::new();
        let mut total = 0u64;
        let mut nav_href = None;
        for el in manifest.children_named("item") {
            let (Some(id), Some(href)) = (el.attr("id"), el.attr("href")) else {
                continue;
            };
            if is_external(href) {
                continue;
            }
            let href = resolve_href(&opf_path, href);
            let media_type = el.attr("media-type").unwrap_or("").to_ascii_lowercase();
            // A missing file is left out, as calibre does.
            let Ok(bytes) = read_entry(&mut archive, &href, MAX_ITEM_BYTES) else {
                continue;
            };
            total += bytes.len() as u64;
            if total > MAX_TOTAL_BYTES {
                return Err(unreadable("contents are too large"));
            }
            if el
                .attr("properties")
                .is_some_and(|p| p.split_whitespace().any(|p| p == "nav"))
            {
                nav_href = Some(href.clone());
            }
            let is_html = matches!(media_type.as_str(), "application/xhtml+xml" | "text/html");
            let data = if is_html && spine_ids.contains(&id) {
                Data::Html(parse_html(&bytes).map_err(|e| unreadable(format!("{href}: {e}")))?)
            } else if media_type == "text/css" {
                Data::Css(decode_css(&bytes))
            } else if media_type == "image/svg+xml" {
                match dom::parse(&bytes) {
                    Ok(svg) => Data::Svg(svg),
                    Err(_) => Data::Raw(bytes),
                }
            } else {
                Data::Raw(bytes)
            };
            items.push(Item {
                id: id.to_owned(),
                href,
                media_type,
                data,
            });
        }

        let spine: Vec<usize> = spine_ids
            .iter()
            .filter_map(|id| {
                items
                    .iter()
                    .position(|i| i.id == *id && matches!(i.data, Data::Html(_)))
            })
            .collect();
        if spine.is_empty() {
            return Err(unreadable("the spine has no documents"));
        }

        let cover_image = cover_href(&package)
            .map(|href| resolve_href(&opf_path, &href))
            .and_then(|href| items.iter().position(|i| i.href == href))
            .filter(|&i| items[i].media_type.starts_with("image/"));

        let metadata = package.child("metadata");
        let uuid = metadata.and_then(|m| {
            m.children_named("identifier").find_map(|id| {
                let value = id.text_content();
                let schemed = id
                    .attr("scheme")
                    .is_some_and(|s| s.eq_ignore_ascii_case("uuid"));
                match value.strip_prefix("urn:uuid:") {
                    Some(u) => Some(u.to_owned()),
                    None => schemed.then_some(value),
                }
            })
        });
        let primary_writing_mode = metadata.and_then(|m| {
            m.children_named("meta")
                .find(|e| e.attr("name") == Some("primary-writing-mode"))
                .and_then(|e| e.attr("content"))
                .map(str::to_owned)
        });

        let toc_item = spine_el
            .attr("toc")
            .and_then(|id| items.iter().find(|i| i.id == id))
            .or_else(|| {
                items
                    .iter()
                    .find(|i| i.media_type == "application/x-dtbncx+xml")
            });
        let mut toc = toc_item
            .and_then(|i| match &i.data {
                Data::Raw(bytes) => read_ncx(bytes, &i.href),
                _ => None,
            })
            .unwrap_or_else(|| TocNode {
                title: String::new(),
                href: None,
                children: Vec::new(),
            });
        let nav = nav_href.and_then(|href| {
            let item = items.iter().find(|i| i.href == href)?;
            let root = match &item.data {
                Data::Html(root) => root.clone(),
                Data::Raw(bytes) => parse_html(bytes).ok()?,
                _ => return None,
            };
            Some((href, root))
        });
        // As in calibre, an EPUB 3 navigation document wins over the NCX.
        if let Some((href, root)) = &nav {
            let entries = read_nav(root, href, "toc");
            if !entries.is_empty() {
                toc.children = entries;
            }
        }

        let guide = package
            .child("guide")
            .map(|g| {
                g.children_named("reference")
                    .filter_map(|r| {
                        let href = r.attr("href").filter(|h| !is_external(h))?;
                        Some(GuideRef {
                            kind: r.attr("type")?.to_owned(),
                            title: r.attr("title").unwrap_or("").to_owned(),
                            href: resolve_with_fragment(&opf_path, href),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut oeb = Oeb {
            items,
            spine,
            toc,
            guide,
            cover_image,
            page_progression_direction: spine_el
                .attr("page-progression-direction")
                .map(str::to_owned),
            primary_writing_mode,
            uuid,
            obfuscated,
        };
        let title_page = manifest
            .children_named("item")
            .find(|el| {
                el.attr("properties")
                    .is_some_and(|p| p.split_whitespace().any(|p| p == "calibre:title-page"))
            })
            .and_then(|el| el.attr("href"))
            .map(|h| resolve_href(&opf_path, h));
        let landmark_cover = nav.and_then(|(href, root)| {
            read_landmarks(&root, &href)
                .into_iter()
                .find(|g| g.kind == "cover")
                .map(|g| g.href)
        });
        oeb.remove_html_cover(title_page, landmark_cover);
        Ok(oeb)
    }

    pub fn item(&self, href: &str) -> Option<usize> {
        self.items.iter().position(|i| i.href == href)
    }

    /// Takes an HTML cover page out of the spine, since the Kindle shows the
    /// Cover itself (calibre's `rationalize_cover` and `remove_html_cover`):
    /// the first spine item, when the guide's cover reference or an EPUB 3
    /// landmark points at it, or an item marked as calibre's title page.
    fn remove_html_cover(&mut self, title_page: Option<String>, landmark_cover: Option<String>) {
        let first = &self.items[self.spine[0]].href;
        let guide_cover = self
            .guide
            .iter()
            .find(|g| g.kind.eq_ignore_ascii_case("cover"))
            .map(|g| strip_fragment(&g.href).to_owned());
        let cover_page = title_page.or_else(|| {
            [
                guide_cover,
                landmark_cover.map(|h| strip_fragment(&h).to_owned()),
            ]
            .into_iter()
            .flatten()
            .find(|h| h == first)
        });
        if let Some(page) = cover_page
            && self.spine.len() > 1
        {
            self.spine.retain(|&i| self.items[i].href != page);
        }
        self.guide.retain(|g| {
            !g.kind.eq_ignore_ascii_case("cover") && !g.kind.eq_ignore_ascii_case("titlepage")
        });
    }
}

pub fn strip_fragment(href: &str) -> &str {
    href.split('#').next().unwrap_or(href)
}

/// A URL with a scheme, such as `http:` or `mailto:`, or protocol-relative.
pub fn is_external(href: &str) -> bool {
    if href.starts_with("//") {
        return true;
    }
    let scheme = href.split([':', '/', '#', '?']).next().unwrap_or("");
    href[scheme.len()..].starts_with(':') && !scheme.is_empty()
}

/// Resolves `href` relative to the item at `base`, keeping its fragment:
/// "../Text/ch1.xhtml#s2" from "OEBPS/Text/ch0.xhtml" is "OEBPS/Text/ch1.xhtml#s2".
/// A bare "#frag" refers to `base` itself.
pub fn resolve_with_fragment(base: &str, href: &str) -> String {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (href, None),
    };
    let path = path.split('?').next().unwrap_or(path);
    let resolved = if path.is_empty() {
        base.to_owned()
    } else {
        resolve_href(base, path)
    };
    match frag {
        Some(f) => format!("{resolved}#{}", percent_decode(f)),
        None => resolved,
    }
}

pub(super) fn parse_html(bytes: &[u8]) -> std::result::Result<Element, String> {
    let root = dom::parse(bytes)?;
    if !root.is("html") {
        return Err(format!("root element is <{}>, not <html>", root.name));
    }
    if root.find("body").is_none() {
        return Err("no <body>".into());
    }
    Ok(root)
}

fn decode_css(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let css = String::from_utf8_lossy(bytes).into_owned();
    // The text is UTF-8 from here on, whatever it declared.
    match css.strip_prefix("@charset") {
        Some(rest) => rest
            .split_once(';')
            .map_or(String::new(), |(_, r)| r.to_owned()),
        None => css,
    }
}

/// The files META-INF/encryption.xml says are encrypted or obfuscated.
fn read_encryption<R: Read + Seek>(archive: &mut ZipArchive<R>) -> HashSet<String> {
    let Ok(bytes) = read_entry(archive, "META-INF/encryption.xml", MAX_ITEM_BYTES) else {
        return HashSet::new();
    };
    let Ok(root) = xml::parse(&bytes) else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    fn walk(el: &xml::Element, out: &mut HashSet<String>) {
        if el.name == "CipherReference"
            && let Some(uri) = el.attr("URI")
        {
            out.insert(percent_decode(uri.trim_start_matches('/')));
        }
        for c in &el.children {
            walk(c, out);
        }
    }
    walk(&root, &mut out);
    out
}

fn read_ncx(bytes: &[u8], ncx_href: &str) -> Option<TocNode> {
    let root = xml::parse(bytes).ok()?;
    fn points(parent: &xml::Element, base: &str) -> Vec<TocNode> {
        parent
            .children_named("navPoint")
            .map(|p| TocNode {
                title: p
                    .child("navLabel")
                    .and_then(|l| l.child("text"))
                    .map(|t| t.text_content())
                    .unwrap_or_default(),
                href: p
                    .child("content")
                    .and_then(|c| c.attr("src"))
                    .filter(|s| !is_external(s))
                    .map(|s| resolve_with_fragment(base, s)),
                children: points(p, base),
            })
            .collect()
    }
    let children = points(root.child("navMap")?, ncx_href);
    Some(TocNode {
        title: String::new(),
        href: None,
        children,
    })
}

/// The entries of the EPUB 3 navigation document's `<nav epub:type=kind>`.
fn read_nav(root: &Element, nav_href: &str, kind: &str) -> Vec<TocNode> {
    let mut found = None;
    root.walk(&mut |e| {
        if found.is_none()
            && e.is("nav")
            && e.get("type")
                .is_some_and(|t| t.split_whitespace().any(|t| t == kind))
        {
            found = Some(e.clone());
        }
    });
    let Some(nav) = found else {
        return Vec::new();
    };
    fn list(ol: &Element, base: &str) -> Vec<TocNode> {
        ol.elements()
            .filter(|li| li.is("li"))
            .filter_map(|li| {
                let label = li.elements().find(|e| e.is("a") || e.is("span"));
                let children = li
                    .elements()
                    .find(|e| e.is("ol"))
                    .map(|ol| list(ol, base))
                    .unwrap_or_default();
                let title = label
                    .map(|l| l.text().split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                if title.is_empty() && children.is_empty() {
                    return None;
                }
                Some(TocNode {
                    title,
                    href: label
                        .and_then(|l| l.get("href"))
                        .filter(|h| !is_external(h))
                        .map(|h| resolve_with_fragment(base, h)),
                    children,
                })
            })
            .collect()
    }
    nav.elements()
        .find(|e| e.is("ol"))
        .map(|ol| list(ol, nav_href))
        .unwrap_or_default()
}

/// The links of the EPUB 3 `<nav epub:type="landmarks">`, typed by their
/// own `epub:type`.
fn read_landmarks(root: &Element, nav_href: &str) -> Vec<GuideRef> {
    let mut out = Vec::new();
    root.walk(&mut |nav| {
        if !(nav.is("nav")
            && nav
                .get("type")
                .is_some_and(|t| t.split_whitespace().any(|t| t == "landmarks")))
        {
            return;
        }
        nav.walk(&mut |a| {
            if a.is("a")
                && let (Some(kind), Some(href)) = (a.get("type"), a.get("href"))
                && !is_external(href)
            {
                out.push(GuideRef {
                    kind: kind.to_owned(),
                    title: a.text(),
                    href: resolve_with_fragment(nav_href, href),
                });
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_hrefs_with_fragments() {
        assert_eq!(
            resolve_with_fragment("OEBPS/Text/a.xhtml", "b.xhtml#x"),
            "OEBPS/Text/b.xhtml#x"
        );
        assert_eq!(
            resolve_with_fragment("OEBPS/Text/a.xhtml", "#x%20y"),
            "OEBPS/Text/a.xhtml#x y"
        );
        assert_eq!(
            resolve_with_fragment("OEBPS/toc.ncx", "../c%201.html"),
            "c 1.html"
        );
        assert!(is_external("http://example.com/a"));
        assert!(is_external("mailto:x@y"));
        assert!(!is_external("Text/a.xhtml#b:c"));
    }
}
