// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/tbs.py
// (calibre 9.16, commit 0b8c0c2).

//! Trailing byte sequences: which Table of Contents entries touch each text
//! record.
//!
//! For each text record, the entries that start, end, span or are contained
//! in it are arranged into strands, each a hierarchical progression from the
//! top level down ([`separate_strands`]). The strands are encoded as
//! sequences of numbers with flags, which are turned into bytes appended to
//! the record.

use super::index::NcxEntry;
use super::utils::{TbsExtra, encode_tbs, encode_trailing_data};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Spans,
    Ends,
    Starts,
    Completes,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    index: usize,
    start: usize,
    length: usize,
    depth: usize,
    parent: Option<usize>,
    action: Action,
    length_offset: i64,
}

fn fill_entry(e: &NcxEntry, start_offset: i64, record_length: usize) -> Entry {
    let length_offset = start_offset + e.length as i64;
    let past_end = length_offset > record_length as i64;
    let action = match (start_offset < 0, past_end) {
        (true, true) => Action::Spans,
        (true, false) => Action::Ends,
        (false, true) => Action::Starts,
        (false, false) => Action::Completes,
    };
    Entry {
        index: e.index,
        start: e.offset,
        length: e.length,
        depth: e.depth,
        parent: e.parent,
        action,
        length_offset,
    }
}

fn populate_strand(parent: Entry, entries: &mut Vec<Entry>) -> Vec<Entry> {
    let mut ans = vec![parent.clone()];
    if let Some(pos) = entries.iter().position(|c| c.parent == Some(parent.index)) {
        // Add the first child to this strand, and recurse downwards.
        let child = entries.remove(pos);
        ans.extend(populate_strand(child, entries));
    } else {
        // Add entries at the same depth that form a contiguous run of
        // indices under the same parent; one sequence with the 0b100 flag
        // can represent them all.
        let mut current = parent.index;
        let snapshot = entries.clone();
        for entry in snapshot {
            if entry.depth == parent.depth
                && entry.parent == parent.parent
                && entry.index == current + 1
            {
                current += 1;
                let pos = entries.iter().position(|e| *e == entry).unwrap();
                entries.remove(pos);
                if entries.iter().any(|c| c.parent == Some(entry.index)) {
                    // No more siblings once one has children.
                    ans.extend(populate_strand(entry, entries));
                    break;
                }
                ans.push(entry);
            }
        }
    }
    ans
}

/// A strand's entries grouped by depth, in order of first appearance.
type Strand = Vec<(usize, Vec<Entry>)>;

fn separate_strands(mut entries: Vec<Entry>) -> Vec<Strand> {
    let mut ans = Vec::new();
    while !entries.is_empty() {
        let top = entries.remove(0);
        let mut layers: Strand = Vec::new();
        for entry in populate_strand(top, &mut entries) {
            match layers.iter_mut().find(|(d, _)| *d == entry.depth) {
                Some((_, layer)) => layer.push(entry),
                None => layers.push((entry.depth, vec![entry])),
            }
        }
        ans.push(layers);
    }
    ans
}

fn collect_indexing_data(entries: &[NcxEntry], record_lengths: &[usize]) -> Vec<Vec<Strand>> {
    let mut sorted: Vec<&NcxEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.offset);
    let mut data = Vec::new();
    let mut record_start = 0;
    for &rec_length in record_lengths {
        let next_record_start = record_start + rec_length;
        let mut local = Vec::new();
        for e in &sorted {
            if e.offset >= next_record_start {
                break;
            }
            if e.offset + e.length <= record_start {
                continue;
            }
            local.push(fill_entry(
                e,
                e.offset as i64 - record_start as i64,
                rec_length,
            ));
        }
        data.push(separate_strands(local));
        record_start = next_record_start;
    }
    data
}

struct NegativeStrandIndex;

fn encode_strands_as_sequences(
    strands: &[Strand],
    tbs_type: u64,
) -> Result<Vec<(u64, TbsExtra)>, NegativeStrandIndex> {
    let first_entry = strands
        .iter()
        .flat_map(|s| s.iter().flat_map(|(_, l)| l))
        .next();
    let mut ans: Vec<(u64, TbsExtra)> = Vec::new();
    let mut last_index = 0usize;

    for strand in strands {
        let mut strand_seqs: Vec<(u64, TbsExtra)> = Vec::new();
        for (_, entries) in strand {
            let mut extra = TbsExtra::default();
            let last = entries.last().unwrap();
            if last.action == Action::Spans {
                extra.offset = Some(0);
            }
            if Some(&entries[0]) == first_entry {
                extra.tbs_type = Some(tbs_type);
            }
            if entries.len() > 1 {
                extra.count = Some(entries.len() as u8);
            }
            let mut index = entries[0].index as i64 - entries[0].parent.unwrap_or(0) as i64;
            if !ans.is_empty() && strand_seqs.is_empty() {
                // In the second or later strands the index is relative to the
                // last entry of the previous strand.
                index = last_index as i64 - entries[0].index as i64;
                if index < 0 {
                    if tbs_type == 5 {
                        index = -index;
                    } else {
                        return Err(NegativeStrandIndex);
                    }
                } else {
                    extra.strand_flag = true;
                }
            }
            last_index = last.index;
            strand_seqs.push((index as u64, extra));
        }

        // Of consecutive spanning entries, only the last keeps the 0b1 flag.
        for i in 0..strand_seqs.len().saturating_sub(1) {
            if strand_seqs[i].1.offset.is_some() && strand_seqs[i + 1].1.offset.is_some() {
                strand_seqs[i].1.offset = None;
            }
        }
        ans.extend(strand_seqs);
    }
    Ok(ans)
}

fn sequences_to_bytes(sequences: &[(u64, TbsExtra)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, (val, extra)) in sequences.iter().enumerate() {
        // Only the first sequence has a 3-bit flag; later ones may need 0b1000.
        out.extend(encode_tbs(*val, extra, if i == 0 { 3 } else { 4 }));
    }
    out
}

fn calculate_all_tbs(
    data: &[Vec<Strand>],
    tbs_type: u64,
) -> Result<Vec<Vec<u8>>, NegativeStrandIndex> {
    data.iter()
        .map(|strands| {
            Ok(sequences_to_bytes(&encode_strands_as_sequences(
                strands, tbs_type,
            )?))
        })
        .collect()
}

/// Appends each text record's trailing byte sequence; `records[1..]` are the
/// text records, whose uncompressed lengths are `record_lengths`.
pub fn apply_trailing_byte_sequences(
    toc: &[NcxEntry],
    records: &mut [Vec<u8>],
    record_lengths: &[usize],
) {
    let data = collect_indexing_data(toc, record_lengths);
    let all = calculate_all_tbs(&data, 8)
        .or_else(|_| calculate_all_tbs(&data, 5))
        .unwrap_or_else(|_| unreachable!("type 5 never fails"));
    for (i, tbs) in all.into_iter().enumerate() {
        records[i + 1].extend(encode_trailing_data(&tbs));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        index: usize,
        offset: usize,
        length: usize,
        depth: usize,
        parent: Option<usize>,
    ) -> NcxEntry {
        NcxEntry {
            index,
            label: format!("E{index}"),
            depth,
            offset,
            length,
            pos_fid: (0, 0),
            parent,
            first_child: None,
            last_child: None,
        }
    }

    #[test]
    fn single_level_toc() {
        // Two chapters in the first record, the second spilling into the next.
        let toc = [entry(0, 0, 100, 0, None), entry(1, 100, 4096, 0, None)];
        let mut records = vec![Vec::new(), Vec::new(), Vec::new()];
        apply_trailing_byte_sequences(&toc, &mut records, &[4096, 4096]);
        // Record 1: index 0, type 8, two entries → fvwi(0, 0b110, 3) 8 2.
        assert_eq!(records[1], [0x86, 0x88, 0x02, 0x84]);
        // Record 2: entry 1 ends here → fvwi(1, 0b010, 3) 8.
        assert_eq!(records[2], [0x8A, 0x88, 0x83]);
    }

    #[test]
    fn records_without_entries_get_an_empty_sequence() {
        let mut records = vec![Vec::new(), Vec::new()];
        apply_trailing_byte_sequences(&[], &mut records, &[100]);
        assert_eq!(records[1], [0x81]);
    }

    #[test]
    fn nested_entries_form_strands() {
        let toc = [
            entry(0, 0, 9000, 0, None),
            entry(1, 10, 50, 1, Some(0)),
            entry(2, 60, 8940, 1, Some(0)),
        ];
        let data = collect_indexing_data(&toc, &[4096]);
        let strands = &data[0];
        assert_eq!(strands.len(), 1);
        let depths: Vec<(usize, Vec<usize>)> = strands[0]
            .iter()
            .map(|(d, l)| (*d, l.iter().map(|e| e.index).collect()))
            .collect();
        assert_eq!(depths, [(0, vec![0]), (1, vec![1, 2])]);
    }
}
