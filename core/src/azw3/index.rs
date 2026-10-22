// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/index.py
// (calibre 9.16, commit 0b8c0c2).

//! KF8 index records (INDX): the skeleton, chunk, guide and NCX tables.

use super::utils::{Cncx, align_block, encint};

const HEADER_LENGTH: usize = 192;
const NULL: u32 = 0xFFFF_FFFF;

/// One column of an index: name, tag number, values per entry, bitmask in
/// the control byte. A tag with `end` set closes a control byte.
#[derive(Clone, Copy)]
struct TagMeta {
    name: &'static str,
    number: u8,
    values_per_entry: u8,
    mask: u8,
    end: bool,
}

const fn tag(name: &'static str, number: u8, values_per_entry: u8, mask: u8) -> TagMeta {
    TagMeta {
        name,
        number,
        values_per_entry,
        mask,
        end: false,
    }
}

const END: TagMeta = TagMeta {
    name: "eof",
    number: 0,
    values_per_entry: 0,
    mask: 0,
    end: true,
};

const SKEL_TAGS: &[TagMeta] = &[tag("chunk_count", 1, 1, 3), tag("geometry", 6, 2, 12), END];
const CHUNK_TAGS: &[TagMeta] = &[
    tag("cncx_offset", 2, 1, 1),
    tag("file_number", 3, 1, 2),
    tag("sequence_number", 4, 1, 4),
    tag("geometry", 6, 2, 8),
    END,
];
const GUIDE_TAGS: &[TagMeta] = &[tag("title", 1, 1, 1), tag("pos_fid", 6, 2, 2), END];
const NCX_TAGS: &[TagMeta] = &[
    tag("offset", 1, 1, 1),
    tag("length", 2, 1, 2),
    tag("label", 3, 1, 4),
    tag("depth", 4, 1, 8),
    tag("parent", 21, 1, 16),
    tag("first_child", 22, 1, 32),
    tag("last_child", 23, 1, 64),
    tag("pos_fid", 6, 2, 128),
    END,
];

fn bit_shifts(mask: u8) -> u32 {
    match mask {
        1 | 3 => 0,
        2 => 1,
        4 | 12 => 2,
        8 => 3,
        16 | 48 => 4,
        32 => 5,
        64 | 192 => 6,
        128 => 7,
        _ => unreachable!("mask {mask}"),
    }
}

/// An index entry: its key and its values by tag name.
type Entry = (String, Vec<(&'static str, Vec<u64>)>);

/// Serializes `entries` as an index header record, the index records and the
/// CNCX records.
fn build(tag_types: &[TagMeta], entries: &[Entry], cncx: Cncx) -> Vec<Vec<u8>> {
    // kindlegen uses 1048 (there has to be some margin because of block
    // alignment).
    const RECORD_LIMIT: usize = 0x10000 - HEADER_LENGTH - 1048;

    let mut index_blocks: Vec<Vec<u8>> = vec![Vec::new()];
    let mut idxt_blocks: Vec<Vec<u8>> = vec![Vec::new()];
    let mut record_counts = vec![0u16];
    let mut last_indices: Vec<Vec<u8>> = vec![Vec::new()];

    for (key, tags) in entries {
        let values = |name: &str| tags.iter().find(|(n, _)| *n == name).map(|(_, v)| v);
        let mut raw = vec![key.len() as u8];
        raw.extend(key.as_bytes());
        let mut control = 0u8;
        for t in tag_types {
            if t.end {
                raw.push(control);
                control = 0;
                continue;
            }
            let nvals = values(t.name).map_or(0, Vec::len);
            let nentries = (nvals / t.values_per_entry as usize) as u8;
            control |= t.mask & (nentries << bit_shifts(t.mask));
        }
        for t in tag_types.iter().filter(|t| !t.end) {
            for &v in values(t.name).into_iter().flatten() {
                raw.extend(encint(v, true));
            }
        }

        let used = index_blocks.last().unwrap().len() + idxt_blocks.last().unwrap().len();
        if used + raw.len() + 2 > RECORD_LIMIT {
            index_blocks.push(Vec::new());
            idxt_blocks.push(Vec::new());
            record_counts.push(0);
            last_indices.push(Vec::new());
        }
        let offset = index_blocks.last().unwrap().len();
        *record_counts.last_mut().unwrap() += 1;
        idxt_blocks
            .last_mut()
            .unwrap()
            .extend(((HEADER_LENGTH + offset) as u16).to_be_bytes());
        index_blocks.last_mut().unwrap().extend(&raw);
        *last_indices.last_mut().unwrap() = key.as_bytes().to_vec();
    }

    let mut records = Vec::new();
    for ((index_block, idxt_block), &count) in index_blocks
        .into_iter()
        .zip(idxt_blocks)
        .zip(&record_counts)
    {
        let index_block = align_block(index_block);
        let mut idxt = b"IDXT".to_vec();
        idxt.extend(idxt_block);
        let idxt = align_block(idxt);
        let mut rec = b"INDX".to_vec();
        rec.extend((HEADER_LENGTH as u32).to_be_bytes());
        rec.extend([0; 4]);
        // Header type: 0 for the index header record, 1 for index records.
        rec.extend(1u32.to_be_bytes());
        rec.extend([0; 4]);
        rec.extend(((HEADER_LENGTH + index_block.len()) as u32).to_be_bytes());
        rec.extend((count as u32).to_be_bytes());
        rec.extend([0xFF; 8]);
        rec.extend([0; 156]);
        rec.extend(index_block);
        rec.extend(idxt);
        assert!(
            rec.len() <= 0x10000,
            "failed to roll over a very large index"
        );
        records.push(rec);
    }

    let mut tagx = b"TAGX".to_vec();
    let tag_count = tag_types.len() as u32;
    tagx.extend((12 + 4 * tag_count).to_be_bytes());
    tagx.extend(1u32.to_be_bytes()); // control byte count
    for t in tag_types {
        tagx.extend([t.number, t.values_per_entry, t.mask, t.end as u8]);
    }

    // The geometry of the index records is written as index entries pointed
    // to by the header's IDXT.
    let mut geometry = Vec::new();
    let mut idxt = b"IDXT".to_vec();
    let mut pos = HEADER_LENGTH + tagx.len();
    for (last, &num) in last_indices.iter().zip(&record_counts) {
        let start = geometry.len();
        idxt.extend((pos as u16).to_be_bytes());
        geometry.push(last.len() as u8);
        geometry.extend(last);
        geometry.extend(num.to_be_bytes());
        pos += geometry.len() - start;
    }

    let tagx = align_block(tagx);
    let geometry = align_block(geometry);
    let idxt = align_block(idxt);
    let mut header = b"INDX".to_vec();
    header.extend((HEADER_LENGTH as u32).to_be_bytes());
    header.extend([0; 8]);
    header.extend(2u32.to_be_bytes()); // index type
    let idxt_offset = HEADER_LENGTH + tagx.len() + geometry.len();
    header.extend((idxt_offset as u32).to_be_bytes());
    header.extend((records.len() as u32).to_be_bytes());
    header.extend(65001u32.to_be_bytes()); // UTF-8
    header.extend(NULL.to_be_bytes());
    let total: u32 = record_counts.iter().map(|&c| c as u32).sum();
    header.extend(total.to_be_bytes());
    header.extend([0; 12]); // ORDT, LIGT offsets and count
    header.extend((cncx.records.len() as u32).to_be_bytes());
    header.extend([0; 124]);
    header.extend((HEADER_LENGTH as u32).to_be_bytes()); // TAGX offset
    header.extend([0; 8]);
    debug_assert_eq!(header.len(), HEADER_LENGTH);
    header.extend(tagx);
    header.extend(geometry);
    header.extend(idxt);

    let mut out = vec![align_block(header)];
    out.extend(records);
    out.extend(cncx.records);
    out
}

/// One file of the text (an HTML document) with its chunks taken out.
#[derive(Debug, Clone, PartialEq)]
pub struct Skel {
    pub file_number: usize,
    pub chunk_count: usize,
    pub start_pos: usize,
    pub length: usize,
}

/// A piece of a file's body, inserted into its skeleton at `insert_pos`.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkEntry {
    pub insert_pos: usize,
    pub selector: String,
    pub file_number: usize,
    pub sequence_number: usize,
    pub start_pos: usize,
    pub length: usize,
}

pub fn skel_index(skels: &[Skel]) -> Vec<Vec<u8>> {
    let entries: Vec<Entry> = skels
        .iter()
        .map(|s| {
            let (count, start, len) = (s.chunk_count as u64, s.start_pos as u64, s.length as u64);
            (
                format!("SKEL{:010}", s.file_number),
                // Don't ask why these have to be repeated twice (calibre).
                vec![
                    ("chunk_count", vec![count, count]),
                    ("geometry", vec![start, len, start, len]),
                ],
            )
        })
        .collect();
    build(SKEL_TAGS, &entries, Cncx::default())
}

pub fn chunk_index(chunks: &[ChunkEntry]) -> Vec<Vec<u8>> {
    let cncx = Cncx::new(chunks.iter().map(|c| c.selector.as_str()));
    let entries: Vec<Entry> = chunks
        .iter()
        .map(|c| {
            (
                format!("{:010}", c.insert_pos),
                vec![
                    ("cncx_offset", vec![cncx.offset(&c.selector)]),
                    ("file_number", vec![c.file_number as u64]),
                    ("sequence_number", vec![c.sequence_number as u64]),
                    ("geometry", vec![c.start_pos as u64, c.length as u64]),
                ],
            )
        })
        .collect();
    build(CHUNK_TAGS, &entries, cncx)
}

/// A guide reference: its type, title and position as (chunk, offset).
#[derive(Debug, Clone, PartialEq)]
pub struct GuideRef {
    pub title: String,
    pub kind: String,
    pub pos_fid: (usize, usize),
}

pub fn guide_index(refs: &[GuideRef]) -> Vec<Vec<u8>> {
    let cncx = Cncx::new(refs.iter().map(|r| r.title.as_str()));
    let entries: Vec<Entry> = refs
        .iter()
        .map(|r| {
            (
                r.kind.clone(),
                vec![
                    ("title", vec![cncx.offset(&r.title)]),
                    ("pos_fid", vec![r.pos_fid.0 as u64, r.pos_fid.1 as u64]),
                ],
            )
        })
        .collect();
    build(GUIDE_TAGS, &entries, cncx)
}

/// A Table of Contents entry, flattened depth first.
#[derive(Debug, Clone, PartialEq)]
pub struct NcxEntry {
    pub index: usize,
    pub label: String,
    pub depth: usize,
    pub offset: usize,
    pub length: usize,
    pub pos_fid: (usize, usize),
    pub parent: Option<usize>,
    pub first_child: Option<usize>,
    pub last_child: Option<usize>,
}

pub fn ncx_index(toc: &[NcxEntry]) -> Vec<Vec<u8>> {
    let cncx = Cncx::new(toc.iter().map(|e| e.label.as_str()));
    let largest = toc.iter().map(|e| e.index).max().unwrap_or(0);
    let width = format!("{largest:X}").len().max(2);
    let entries: Vec<Entry> = toc
        .iter()
        .map(|e| {
            let mut tags = vec![
                ("offset", vec![e.offset as u64]),
                ("length", vec![e.length as u64]),
                ("depth", vec![e.depth as u64]),
                ("pos_fid", vec![e.pos_fid.0 as u64, e.pos_fid.1 as u64]),
            ];
            for (name, v) in [
                ("parent", e.parent),
                ("first_child", e.first_child),
                ("last_child", e.last_child),
            ] {
                if let Some(v) = v {
                    tags.push((name, vec![v as u64]));
                }
            }
            tags.push(("label", vec![cncx.offset(&e.label)]));
            (format!("{:0width$X}", e.index), tags)
        })
        .collect();
    build(NCX_TAGS, &entries, cncx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_header_index_and_cncx_records() {
        let records = guide_index(&[GuideRef {
            title: "Start".into(),
            kind: "text".into(),
            pos_fid: (1, 0),
        }]);
        assert_eq!(records.len(), 3);
        let header = &records[0];
        assert_eq!(&header[..4], b"INDX");
        assert_eq!(&header[192..196], b"TAGX");
        assert!(records.iter().all(|r| r.len() % 4 == 0));

        let rec = &records[1];
        let entry = &rec[192..];
        // key "text", control byte 0b11 (one title, one pos_fid pair), then
        // the CNCX offset 0 and the position 1, 0.
        assert_eq!(&entry[..9], b"\x04text\x03\x80\x81\x80");
        assert_eq!(records[2], b"\x85Start\0\0".to_vec());
    }

    #[test]
    fn rolls_over_large_indices() {
        let chunks: Vec<ChunkEntry> = (0..5000)
            .map(|i| ChunkEntry {
                insert_pos: i * 100,
                selector: format!("P-//*[@aid='{i}']"),
                file_number: 0,
                sequence_number: i,
                start_pos: i * 100,
                length: 100,
            })
            .collect();
        let records = chunk_index(&chunks);
        let num_records = u32::from_be_bytes(records[0][24..28].try_into().unwrap());
        assert!(num_records > 1);
        let entries = u32::from_be_bytes(records[0][36..40].try_into().unwrap());
        assert_eq!(entries, 5000);
        assert!(records.iter().all(|r| r.len() <= 0x10000));
    }
}
