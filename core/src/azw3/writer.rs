// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/main.py and
// src/calibre/ebooks/mobi/writer8/cleanup.py (calibre 9.16, commit 0b8c0c2).

//! Turning the book's markup into KF8 text records and indices.

use std::collections::{HashMap, HashSet};

use super::css;
use super::dom::{Element, Node};
use super::index::{self, ChunkEntry, GuideRef, NcxEntry};
use super::oeb::{Data, Oeb, TocNode, is_external, resolve_with_fragment, strip_fragment};
use super::palmdoc;
use super::resources::Resources;
use super::skeleton::{AID_ABLE_TAGS, Chunker, to_href};
use super::tbs::apply_trailing_byte_sequences;
use super::utils::{create_text_record, to_base32, to_ref};
use crate::Result;

/// What the KF8 writer produces, ready to become a book.
pub struct Kf8 {
    /// Index 0 is a placeholder for record 0.
    pub records: Vec<Vec<u8>>,
    pub text_length: usize,
    pub last_text_record: usize,
    pub first_non_text_record: usize,
    pub chunk_records: Vec<Vec<u8>>,
    pub skel_records: Vec<Vec<u8>>,
    pub guide_records: Vec<Vec<u8>>,
    pub ncx_records: Vec<Vec<u8>>,
    pub fdst_record: Vec<u8>,
    pub fdst_count: usize,
    pub has_tbs: bool,
    pub start_offset: Option<usize>,
    pub used_images: HashSet<String>,
}

pub struct Writer<'a> {
    oeb: &'a mut Oeb,
    resources: &'a Resources,
    compress: bool,
    used_images: HashSet<String>,
    /// The text first, then stylesheets and SVG images.
    flows: Vec<Vec<u8>>,
    /// Link placeholder → (item href, fragment).
    link_map: HashMap<String, (String, String)>,
    /// (item href, id) → aid or cid; an empty id stands for the body.
    id_map: HashMap<(String, String), String>,
}

impl<'a> Writer<'a> {
    pub fn new(oeb: &'a mut Oeb, resources: &'a Resources, compress: bool) -> Writer<'a> {
        Writer {
            oeb,
            resources,
            compress,
            used_images: HashSet::new(),
            flows: vec![Vec::new()],
            link_map: HashMap::new(),
            id_map: HashMap::new(),
        }
    }

    pub fn write(mut self) -> Result<Kf8> {
        self.remove_duplicate_anchors();
        self.cleanup_markup();
        self.replace_resource_links();
        self.extract_css_into_flows();
        self.extract_svg_into_flows();
        self.replace_internal_links_with_placeholders();
        self.insert_aid_attributes();
        let chunker = self.chunk_it_up()?;
        self.flows[0] = chunker.text.clone();

        let (records, record_lengths, text_length) = self.create_text_records();
        let mut kf8 = Kf8 {
            last_text_record: record_lengths.len(),
            first_non_text_record: records.len(),
            records,
            text_length,
            chunk_records: index::chunk_index(&chunker.chunk_table),
            skel_records: index::skel_index(&chunker.skel_table),
            guide_records: Vec::new(),
            ncx_records: Vec::new(),
            fdst_record: Vec::new(),
            fdst_count: self.flows.len(),
            has_tbs: false,
            start_offset: None,
            used_images: std::mem::take(&mut self.used_images),
        };
        self.create_fdst_record(&mut kf8);
        self.create_ncx(&chunker, &record_lengths, &mut kf8);
        self.create_guide(&chunker, &mut kf8);
        Ok(kf8)
    }

    fn spine_docs(&mut self) -> impl Iterator<Item = (String, &mut Element)> {
        let spine = self.oeb.spine.clone();
        self.oeb
            .items
            .iter_mut()
            .enumerate()
            .filter(move |(i, _)| spine.contains(i))
            .filter_map(|(_, item)| match &mut item.data {
                Data::Html(root) => Some((item.href.clone(), root)),
                _ => None,
            })
    }

    /// The Kindle misbehaves on duplicate anchors (calibre bug 1454199).
    fn remove_duplicate_anchors(&mut self) {
        for (_, root) in self.spine_docs() {
            let mut seen = HashSet::new();
            root.walk_mut(&mut |tag| {
                for attr in ["id", "name"] {
                    if let Some(anchor) = tag.get(attr).map(str::to_owned)
                        && !seen.insert(anchor)
                    {
                        tag.remove_attr(attr);
                    }
                }
            });
        }
    }

    fn cleanup_markup(&mut self) {
        for (_, root) in self.spine_docs() {
            // Empty script tags are pointless.
            remove_elements(root, &|e| {
                e.is("script") && e.text().is_empty() && e.get("src").is_none()
            });
            // aid and cid are used by this code to map anchors to offsets.
            root.walk_mut(&mut |tag| {
                tag.remove_attr("aid");
                tag.remove_attr("cid");
            });
            // Kindle Touches show all-black pages if the body has a height.
            if let Some(body) = root.find_mut("body")
                && let Some(style) = body.get("style").map(str::to_owned)
            {
                body.set("style", css::drop_height_decls(&style));
            }
        }
    }

    /// The `kindle:embed:` link for the resource at `href`, relative to the
    /// item at `base`; images are noted as used.
    fn pointer(&mut self, base: &str, href: &str) -> Option<String> {
        if is_external(href) {
            return None;
        }
        let abs = strip_fragment(&resolve_with_fragment(base, href)).to_owned();
        let idx = *self.resources.item_map.get(&abs)?;
        if self.resources.is_font(idx) {
            Some(format!("kindle:embed:{}", to_ref(idx)))
        } else {
            self.used_images.insert(abs.clone());
            Some(format!(
                "kindle:embed:{}?mime={}",
                to_ref(idx),
                self.resources.mime_map[&abs]
            ))
        }
    }

    /// Links to images and fonts become `kindle:embed:XXXX` pointers to the
    /// resource records.
    fn replace_resource_links(&mut self) {
        for i in 0..self.oeb.items.len() {
            let href = self.oeb.items[i].href.clone();
            let mut data = std::mem::replace(&mut self.oeb.items[i].data, Data::Raw(Vec::new()));
            match &mut data {
                Data::Html(root) | Data::Svg(root) => {
                    let mut edits = Vec::new();
                    collect_resource_links(root, &mut Vec::new(), &mut edits);
                    for (path, attr, value) in edits {
                        if let Some(new) = self.pointer(&href, &value) {
                            node_at_mut(root, &path).attrs[attr].1 = new;
                        }
                    }
                    let mut styles = Vec::new();
                    collect_paths(root, &mut Vec::new(), &|e| e.is("style"), &mut styles);
                    for path in styles {
                        let style = node_at_mut(root, &path);
                        let text = style.text();
                        if text.is_empty() {
                            continue;
                        }
                        let replaced = css::replace_urls(&text, |u| self.pointer(&href, u));
                        style.children = vec![Node::Text(format!("\n{replaced}\n"))];
                    }
                }
                Data::Css(sheet) => {
                    *sheet = css::replace_urls(sheet, |u| self.pointer(&href, u));
                }
                Data::Raw(_) => {}
            }
            self.oeb.items[i].data = data;
        }
    }

    fn extract_css_into_flows(&mut self) {
        let mut sheets: HashMap<String, usize> = HashMap::new();
        for item in &self.oeb.items {
            if let Data::Css(sheet) = &item.data {
                sheets.insert(item.href.clone(), self.flows.len());
                self.flows.push(css::drop_body_height(sheet).into_bytes());
            }
        }
        let first_inline = self.flows.len();
        let flow_link = |idx: usize| format!("kindle:flow:{}?mime=text/css", to_ref(idx));
        let fix_imports = |base: &str, sheet: &str| {
            css::replace_imports(sheet, |u| {
                let abs = strip_fragment(&resolve_with_fragment(base, u)).to_owned();
                sheets.get(&abs).map(|&i| flow_link(i))
            })
        };

        // Identical <style>s go into one flow, numbered in order of first
        // appearance.
        let mut inlines: Vec<String> = Vec::new();
        for i in self.oeb.spine.clone() {
            let (href, root) = doc_mut(self.oeb, i);
            let mut links = Vec::new();
            collect_paths(
                root,
                &mut Vec::new(),
                &|e| e.is("link") && e.get("href").is_some(),
                &mut links,
            );
            for path in links {
                let link = node_at_mut(root, &path);
                let abs = strip_fragment(&resolve_with_fragment(&href, link.get("href").unwrap()))
                    .to_owned();
                if let Some(&idx) = sheets.get(&abs) {
                    link.set("href", flow_link(idx));
                }
            }

            let mut styles = Vec::new();
            collect_paths(root, &mut Vec::new(), &|e| e.is("style"), &mut styles);
            let mut replacements = Vec::new();
            for path in styles {
                let raw = node_at_mut(root, &path).text();
                if raw.trim().is_empty() {
                    replacements.push((path, None));
                    continue;
                }
                let raw = css::drop_body_height(&fix_imports(&href, &raw).unwrap_or(raw));
                let n = inlines.iter().position(|r| *r == raw).unwrap_or_else(|| {
                    inlines.push(raw);
                    inlines.len() - 1
                });
                replacements.push((path, Some(n)));
            }
            // Last first, so the paths of earlier ones stay valid.
            for (path, inline) in replacements.into_iter().rev() {
                let (parent_path, pos) = path.split_at(path.len() - 1);
                let parent = node_at_mut(root, parent_path);
                let Some(n) = inline else {
                    parent.children.remove(pos[0]);
                    continue;
                };
                let mut link = Element::new("link");
                link.attrs.push(("type".into(), "text/css".into()));
                link.attrs.push(("rel".into(), "stylesheet".into()));
                link.attrs
                    .push(("href".into(), flow_link(first_inline + n)));
                parent.children[pos[0]] = Node::Element(link);
                match parent.children.get_mut(pos[0] + 1) {
                    Some(Node::Text(tail)) => tail.insert(0, '\n'),
                    _ => parent.children.insert(pos[0] + 1, Node::Text("\n".into())),
                }
            }
        }
        for raw in inlines {
            self.flows.push(raw.into_bytes());
        }
        for item in &self.oeb.items {
            if let Data::Css(_) = &item.data {
                let idx = sheets[&item.href];
                let sheet = String::from_utf8_lossy(&self.flows[idx]).into_owned();
                if let Some(fixed) = fix_imports(&item.href, &sheet) {
                    self.flows[idx] = fixed.into_bytes();
                }
            }
        }
    }

    fn extract_svg_into_flows(&mut self) {
        let mut images: HashMap<String, usize> = HashMap::new();
        for item in &self.oeb.items {
            if let Data::Svg(svg) = &item.data {
                images.insert(item.href.clone(), self.flows.len());
                let mut raw = br#"<?xml version="1.0" encoding="UTF-8"?>"#.to_vec();
                raw.push(b'\n');
                svg_with_namespaces(svg).write(&mut raw);
                self.flows.push(raw);
            }
        }
        let svg_link = |idx: usize| format!("kindle:flow:{}?mime=image/svg+xml", to_ref(idx));

        for i in self.oeb.spine.clone() {
            let (href, root) = doc_mut(self.oeb, i);
            let mut svgs = Vec::new();
            collect_paths(
                root,
                &mut Vec::new(),
                &|e| e.local_name() == "svg",
                &mut svgs,
            );
            let mut new_flows = Vec::new();
            for path in svgs {
                let idx = self.flows.len() + new_flows.len();
                let svg = node_at_mut(root, &path);
                new_flows.push(svg_with_namespaces(svg).to_bytes());
                let mut img = Element::new("img");
                img.attrs.push(("src".into(), svg_link(idx)));
                *svg = img;
            }
            let mut imgs = Vec::new();
            collect_paths(
                root,
                &mut Vec::new(),
                &|e| e.is("img") && e.get("src").is_some(),
                &mut imgs,
            );
            for path in imgs {
                let img = node_at_mut(root, &path);
                let abs = strip_fragment(&resolve_with_fragment(&href, img.get("src").unwrap()))
                    .to_owned();
                if let Some(&idx) = images.get(&abs) {
                    img.set("src", svg_link(idx));
                }
            }
            self.flows.extend(new_flows);
        }
    }

    fn replace_internal_links_with_placeholders(&mut self) {
        let hrefs: HashSet<String> = self
            .oeb
            .spine
            .iter()
            .map(|&i| self.oeb.items[i].href.clone())
            .collect();
        let mut count = 0;
        let mut link_map = HashMap::new();
        for i in self.oeb.spine.clone() {
            let (base, root) = doc_mut(self.oeb, i);
            root.walk_mut(&mut |a| {
                if !a.is("a") {
                    return;
                }
                let Some(href) = a.get("href").map(str::to_owned) else {
                    return;
                };
                count += 1;
                if is_external(&href) {
                    return;
                }
                let abs = resolve_with_fragment(&base, &href);
                let (path, frag) = abs.split_once('#').unwrap_or((&abs, ""));
                if hrefs.contains(path) {
                    let placeholder = format!("kindle:pos:fid:0000:off:{}", to_href(count));
                    link_map.insert(placeholder.clone(), (path.to_owned(), frag.to_owned()));
                    a.set("href", placeholder);
                }
            });
        }
        self.link_map = link_map;
    }

    fn insert_aid_attributes(&mut self) {
        let mut id_map = HashMap::new();
        let mut cid = 0;
        for (n, i) in self.oeb.spine.clone().into_iter().enumerate() {
            let (href, root) = doc_mut(self.oeb, i);
            let aidbase = n * 1_000_000;
            let mut j = 0;
            insert_aids(root, false, &mut |tag, in_table| {
                let mut id = tag.get("id").map(str::to_owned);
                if id.is_none() && tag.name == "a" {
                    id = tag.get("name").map(str::to_owned);
                    if let Some(id) = &id {
                        tag.set("id", id.clone());
                    }
                }
                let tagname = tag.local_name().to_ascii_lowercase();
                if id.is_none() && !AID_ABLE_TAGS.contains(&tagname.as_str()) {
                    return;
                }
                if in_table {
                    // The Kindle renderer barfs on large tables that have aid
                    // on any of their tags (calibre bug 1489495).
                    if let Some(id) = id.filter(|id| !id.is_empty()) {
                        cid += 1;
                        let val = format!("c{cid}");
                        id_map.insert((href.clone(), id), val.clone());
                        tag.set("cid", val);
                    }
                } else {
                    let aid = to_base32(aidbase + j, 1);
                    tag.set("aid", aid.clone());
                    if tagname == "body" {
                        id_map.insert((href.clone(), String::new()), aid.clone());
                    }
                    if let Some(id) = id {
                        id_map.insert((href.clone(), id), aid);
                    }
                    j += 1;
                }
            });
        }
        self.id_map = id_map;
    }

    fn lookup_aid(&self, href: &str, frag: &str) -> Option<&String> {
        self.id_map
            .get(&(href.to_owned(), frag.to_owned()))
            .or_else(|| self.id_map.get(&(href.to_owned(), String::new())))
    }

    fn chunk_it_up(&mut self) -> Result<Chunker> {
        let placeholder_map: HashMap<String, String> = self
            .link_map
            .iter()
            .filter_map(|(placeholder, (href, frag))| {
                Some((placeholder.clone(), self.lookup_aid(href, frag)?.clone()))
            })
            .collect();
        let docs: Vec<Element> = self
            .oeb
            .spine
            .clone()
            .into_iter()
            .map(|i| doc_mut(self.oeb, i).1.clone())
            .collect();
        Chunker::new(&docs, &placeholder_map)
    }

    /// The text records (from index 1), their uncompressed lengths and the
    /// total text length.
    fn create_text_records(&self) -> (Vec<Vec<u8>>, Vec<usize>, usize) {
        let text: Vec<u8> = self.flows.concat();
        let mut records = vec![Vec::new()];
        let mut lengths = Vec::new();
        let mut records_size = 0;
        let mut pos = 0;
        while pos < text.len() {
            let (data, overlap) = create_text_record(&text, pos);
            lengths.push(data.len());
            let mut rec = if self.compress {
                palmdoc::compress(data)
            } else {
                data.to_vec()
            };
            rec.extend(overlap);
            rec.push(overlap.len() as u8);
            records_size += rec.len();
            records.push(rec);
            pos += data.len();
        }
        // Pad so that the next record starts at a 4 byte boundary (as
        // calibre writes it).
        if records_size % 4 != 0 {
            records.push(vec![0; records_size % 4]);
        }
        (records, lengths, text.len())
    }

    fn create_fdst_record(&self, kf8: &mut Kf8) {
        let mut rec = b"FDST".to_vec();
        rec.extend(12u32.to_be_bytes());
        rec.extend((self.flows.len() as u32).to_be_bytes());
        let mut start = 0u32;
        for flow in &self.flows {
            let end = start + flow.len() as u32;
            rec.extend(start.to_be_bytes());
            rec.extend(end.to_be_bytes());
            start = end;
        }
        kf8.fdst_record = rec;
    }

    fn create_ncx(&self, chunker: &Chunker, record_lengths: &[usize], kf8: &mut Kf8) {
        if self.oeb.toc.count() < 1 {
            return;
        }
        struct Flat {
            id: usize,
            entry: NcxEntry,
            children: Vec<usize>,
            parent_id: Option<usize>,
        }
        // Flatten the ToC depth first, ids being positions in that order.
        fn flatten<'t>(
            node: &'t TocNode,
            depth: usize,
            parent: Option<usize>,
            out: &mut Vec<(&'t TocNode, usize, Option<usize>)>,
        ) {
            for child in &node.children {
                let id = out.len();
                out.push((child, depth, parent));
                flatten(child, depth + 1, Some(id), out);
            }
        }
        let mut nodes = Vec::new();
        flatten(&self.oeb.toc, 0, None, &mut nodes);

        let first_chunk = |c: &[ChunkEntry]| c.first().map_or(0, |c| c.insert_pos);
        let mut entries: Vec<Flat> = nodes
            .iter()
            .enumerate()
            .map(|(id, (node, depth, parent))| {
                let href = node.href.as_deref().unwrap_or("");
                let (path, frag) = href.split_once('#').unwrap_or((href, ""));
                let (pos_fid, offset) = match self.lookup_aid(path, frag) {
                    Some(aid) => {
                        let (pos, fid, offset) = chunker.aid_offset_map[aid];
                        ((pos, fid), offset)
                    }
                    None => ((0, 0), first_chunk(&chunker.chunk_table)),
                };
                let title = if node.title.trim().is_empty() {
                    "Unknown".into()
                } else {
                    node.title.clone()
                };
                Flat {
                    id,
                    entry: NcxEntry {
                        index: id,
                        label: title,
                        depth: *depth,
                        offset,
                        length: 0,
                        pos_fid,
                        parent: None,
                        first_child: None,
                        last_child: None,
                    },
                    children: nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, _, p))| *p == Some(id))
                        .map(|(c, _)| c)
                        .collect(),
                    parent_id: *parent,
                }
            })
            .collect();

        // The Kindle requires entries sorted by (depth, play order), but a
        // non-linear ToC (an entry after a later one at the same depth)
        // breaks section to section jumping, so sort by (depth, offset)
        // instead, which linearizes it.
        entries.sort_by_key(|e| (e.entry.depth, e.entry.offset));
        let id_to_index: HashMap<usize, usize> =
            entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        for (i, e) in entries.iter_mut().enumerate() {
            e.entry.index = i;
            e.entry.first_child = e.children.first().map(|c| id_to_index[c]);
            e.entry.last_child = e.children.last().map(|c| id_to_index[c]);
            e.entry.parent = e.parent_id.map(|p| id_to_index[&p]);
        }
        let text_len = self.flows[0].len();
        let starts: Vec<(usize, usize)> = entries
            .iter()
            .map(|e| (e.entry.depth, e.entry.offset))
            .collect();
        for e in &mut entries {
            let next = starts
                .iter()
                .filter(|(d, o)| *d <= e.entry.depth && *o > e.entry.offset)
                .map(|(_, o)| *o)
                .min()
                .unwrap_or(text_len);
            e.entry.length = next - e.entry.offset;
        }
        let toc: Vec<NcxEntry> = entries.into_iter().map(|e| e.entry).collect();

        apply_trailing_byte_sequences(&toc, &mut kf8.records, record_lengths);
        kf8.has_tbs = true;
        kf8.ncx_records = index::ncx_index(&toc);
    }

    fn create_guide(&self, chunker: &Chunker, kf8: &mut Kf8) {
        let mut table = Vec::new();
        for r in &self.oeb.guide {
            let (path, frag) = r.href.split_once('#').unwrap_or((&r.href, ""));
            let Some(aid) = self
                .id_map
                .get(&(path.to_owned(), frag.to_owned()))
                .or_else(|| self.id_map.get(&(path.to_owned(), String::new())))
            else {
                continue;
            };
            let (pos, fid, offset) = chunker.aid_offset_map[aid];
            let kind = r.kind.to_ascii_lowercase();
            if r.title.eq_ignore_ascii_case("start")
                || matches!(kind.as_str(), "start" | "other.start" | "text")
            {
                kf8.start_offset = Some(offset);
            }
            let title = if r.title.is_empty() {
                "Unknown".into()
            } else {
                r.title.clone()
            };
            table.push(GuideRef {
                title,
                kind: r.kind.clone(),
                pos_fid: (pos, fid),
            });
        }
        if !table.is_empty() {
            // Needed by the Kindle.
            table.sort_by(|a, b| a.kind.cmp(&b.kind));
            kf8.guide_records = index::guide_index(&table);
        }
    }
}

/// Calls `f` on every element below `el` in document order, saying whether
/// it is a table or inside one.
fn insert_aids(el: &mut Element, in_table: bool, f: &mut impl FnMut(&mut Element, bool)) {
    for child in el.elements_mut() {
        let table = in_table || child.is("table");
        f(child, table);
        insert_aids(child, table, f);
    }
}

/// Removes every element matching `pred`, keeping the text after it.
fn remove_elements(el: &mut Element, pred: &dyn Fn(&Element) -> bool) {
    el.children
        .retain(|n| !matches!(n, Node::Element(e) if pred(e)));
    for child in el.elements_mut() {
        remove_elements(child, pred);
    }
}

/// Child-index paths to the outermost elements matching `pred`.
fn collect_paths(
    el: &Element,
    path: &mut Vec<usize>,
    pred: &dyn Fn(&Element) -> bool,
    out: &mut Vec<Vec<usize>>,
) {
    for (i, child) in el.children.iter().enumerate() {
        if let Node::Element(e) = child {
            path.push(i);
            if pred(e) {
                out.push(path.clone());
            } else {
                collect_paths(e, path, pred, out);
            }
            path.pop();
        }
    }
}

/// Paths, attribute positions and values of the `src`/`href` attributes of
/// `<img>` and SVG `<image>` elements.
fn collect_resource_links(
    el: &Element,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, usize, String)>,
) {
    for (i, child) in el.children.iter().enumerate() {
        if let Node::Element(e) = child {
            path.push(i);
            if e.local_name() == "img" || e.local_name() == "image" {
                for (n, (k, v)) in e.attrs.iter().enumerate() {
                    let name = super::dom::local(k).to_ascii_lowercase();
                    if (name == "src" || name == "href") && !k.starts_with("xmlns") {
                        out.push((path.clone(), n, v.clone()));
                    }
                }
            }
            collect_resource_links(e, path, out);
            path.pop();
        }
    }
}

fn doc_mut(oeb: &mut Oeb, index: usize) -> (String, &mut Element) {
    let item = &mut oeb.items[index];
    match &mut item.data {
        Data::Html(root) => (item.href.clone(), root),
        _ => unreachable!("spine items are HTML"),
    }
}

fn node_at_mut<'e>(root: &'e mut Element, path: &[usize]) -> &'e mut Element {
    let mut el = root;
    for &i in path {
        el = match &mut el.children[i] {
            Node::Element(e) => e,
            Node::Text(_) => unreachable!("paths lead to elements"),
        };
    }
    el
}

/// A copy of an SVG element that declares the namespaces it uses, since it
/// will stand alone.
fn svg_with_namespaces(svg: &Element) -> Element {
    let mut svg = svg.clone();
    let declared = |svg: &Element, attr: &str| svg.attrs.iter().any(|(k, _)| k == attr);
    match svg.name.split_once(':') {
        Some((prefix, _)) => {
            let attr = format!("xmlns:{prefix}");
            if !declared(&svg, &attr) {
                svg.attrs
                    .insert(0, (attr, "http://www.w3.org/2000/svg".into()));
            }
        }
        None => {
            if !declared(&svg, "xmlns") {
                svg.attrs
                    .insert(0, ("xmlns".into(), "http://www.w3.org/2000/svg".into()));
            }
        }
    }
    let mut uses_xlink = false;
    fn scan(e: &Element, found: &mut bool) {
        *found |= e.attrs.iter().any(|(k, _)| k.starts_with("xlink:"));
        for c in e.elements() {
            scan(c, found);
        }
    }
    scan(&svg, &mut uses_xlink);
    if uses_xlink && !declared(&svg, "xmlns:xlink") {
        svg.attrs
            .push(("xmlns:xlink".into(), "http://www.w3.org/1999/xlink".into()));
    }
    svg
}
