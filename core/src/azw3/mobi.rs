// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/mobi.py
// (calibre 9.16, commit 0b8c0c2).

//! Assembling the records into a MOBI file: record 0 (the MOBI header and
//! EXTH), and the Palm database around them all.

use super::exth::{ExthOptions, build_exth};
use super::langcodes::iana2mobi;
use super::oeb::Oeb;
use super::resources::Resources;
use super::utils::{RECORD_SIZE, random_bytes, utf8_text};
use super::writer::Kf8;
use crate::model::Metadata;

const NULL: u32 = 0xFFFF_FFFF;
const PALMDOC: u16 = 2;

const FLIS: &[u8] = b"FLIS\0\0\0\x08\0\x41\0\0\0\0\0\0\xff\xff\xff\xff\0\x01\0\x03\0\0\0\x03\0\0\0\x01\xff\xff\xff\xff";
const EOF: &[u8] = b"\xe9\x8e\r\n";

fn fcis(text_length: usize) -> Vec<u8> {
    let mut fcis = b"FCIS\x00\x00\x00\x14\x00\x00\x00\x10\x00\x00\x00\x02\x00\x00\x00\x00".to_vec();
    fcis.extend((text_length as u32).to_be_bytes());
    fcis.extend(b"\x00\x00\x00\x00\x00\x00\x00\x28\x00\x00\x00\x00\x00\x00\x00");
    fcis.extend(b"\x28\x00\x00\x00\x08\x00\x01\x00\x01\x00\x00\x00\x00");
    fcis
}

/// The finished AZW3 file.
pub fn write(
    kf8: Kf8,
    resources: Resources,
    oeb: &Oeb,
    metadata: &Metadata,
    share_not_sync: bool,
) -> Vec<u8> {
    let Kf8 {
        mut records,
        text_length,
        last_text_record,
        first_non_text_record,
        chunk_records,
        skel_records,
        guide_records,
        ncx_records,
        fdst_record,
        fdst_count,
        has_tbs,
        start_offset,
        used_images,
    } = kf8;

    let chunk_index = records.len();
    records.extend(chunk_records);
    let skel_index = records.len();
    records.extend(skel_records);
    let mut guide_index = NULL;
    if !guide_records.is_empty() {
        guide_index = records.len() as u32;
        records.extend(guide_records);
    }
    let mut ncx_index = NULL;
    if !ncx_records.is_empty() {
        ncx_index = records.len() as u32;
        records.extend(ncx_records);
    }

    let (cover_offset, thumbnail_offset, has_fonts) = (
        resources.cover_offset,
        resources.thumbnail_offset,
        resources.has_fonts,
    );
    let has_resources = !resources.records.is_empty();
    let mut first_resource_record = NULL;
    let before = records.len();
    if has_resources {
        first_resource_record = records.len() as u32;
        records.extend(resources.serialize(&used_images));
    }
    let num_of_resources = records.len() - before;

    let fdst_index = records.len();
    records.push(fdst_record);
    let flis_index = records.len();
    records.push(FLIS.to_vec());
    let fcis_index = records.len();
    records.push(fcis(text_length));
    records.push(EOF.to_vec());

    let page_progression_direction = oeb.page_progression_direction.as_deref();
    // Without a writing mode the Kindle ignores right-to-left page progression.
    let primary_writing_mode = oeb
        .primary_writing_mode
        .as_deref()
        .or((page_progression_direction == Some("rtl")).then_some("horizontal-rl"));
    let uuid = oeb.uuid.clone().unwrap_or_else(uuid4);
    let pubdate = metadata.published.clone().unwrap_or_else(now_iso8601);
    let exth = build_exth(
        metadata,
        &ExthOptions {
            uuid: &uuid,
            share_not_sync,
            pubdate: &pubdate,
            cover_offset,
            thumbnail_offset,
            start_offset,
            num_of_resources,
            kf8_unknown_count: has_resources.then_some(0),
            page_progression_direction,
            primary_writing_mode,
        },
    );

    let full_title = utf8_text(&metadata.title);
    let mut extra_data_flags = 0b1;
    if has_tbs {
        extra_data_flags |= 0b10;
    }
    let mut exth_flags = 0b1010000u32;
    if has_fonts {
        exth_flags |= 0b1000000000000;
    }
    let uid = u32::from_be_bytes(random_bytes(4).try_into().unwrap());
    let language = iana2mobi(metadata.language.as_deref().unwrap_or(""));

    let mut r0: Vec<u8> = Vec::new();
    let u16be = |r: &mut Vec<u8>, v: u16| r.extend(v.to_be_bytes());
    let u32be = |r: &mut Vec<u8>, v: u32| r.extend(v.to_be_bytes());
    u16be(&mut r0, PALMDOC);
    u16be(&mut r0, 0);
    u32be(&mut r0, text_length as u32);
    u16be(&mut r0, last_text_record as u16);
    u16be(&mut r0, RECORD_SIZE as u16);
    u16be(&mut r0, 0); // encryption
    u16be(&mut r0, 0);
    r0.extend(b"MOBI");
    u32be(&mut r0, 264); // header length
    u32be(&mut r0, 2); // book type: book
    u32be(&mut r0, 65001); // UTF-8
    u32be(&mut r0, uid);
    u32be(&mut r0, 8); // file version
    for _ in 0..10 {
        u32be(&mut r0, NULL); // orth, inflection and extra indices
    }
    u32be(&mut r0, first_non_text_record as u32);
    let title_offset_pos = r0.len();
    u32be(&mut r0, 0); // title offset, filled in below
    u32be(&mut r0, full_title.len() as u32);
    r0.extend(language);
    u32be(&mut r0, 0); // dictionary input language
    u32be(&mut r0, 0); // dictionary output language
    u32be(&mut r0, 8); // min version
    u32be(&mut r0, first_resource_record);
    r0.extend([0; 16]); // Huff/CDIC
    u32be(&mut r0, exth_flags);
    r0.extend([0; 32]);
    u32be(&mut r0, NULL);
    u32be(&mut r0, NULL); // DRM offset
    r0.extend([0; 12]); // DRM count, size, flags
    r0.extend([0; 8]);
    u32be(&mut r0, fdst_index as u32);
    u32be(&mut r0, fdst_count as u32);
    u32be(&mut r0, fcis_index as u32);
    u32be(&mut r0, 1);
    u32be(&mut r0, flis_index as u32);
    u32be(&mut r0, 1);
    r0.extend([0; 8]);
    u32be(&mut r0, NULL); // SRCS
    u32be(&mut r0, 0);
    r0.extend([0xFF; 8]);
    u32be(&mut r0, extra_data_flags);
    u32be(&mut r0, ncx_index);
    u32be(&mut r0, chunk_index as u32);
    u32be(&mut r0, skel_index as u32);
    u32be(&mut r0, NULL); // DATP
    u32be(&mut r0, guide_index);
    u32be(&mut r0, NULL);
    u32be(&mut r0, 0);
    u32be(&mut r0, NULL);
    u32be(&mut r0, 0);
    debug_assert_eq!(r0.len(), 16 + 264);
    r0.extend(exth);
    let title_offset = r0.len() as u32;
    r0[title_offset_pos..title_offset_pos + 4].copy_from_slice(&title_offset.to_be_bytes());
    r0.extend(&full_title);
    // Padding to allow Amazon's DTP service to add data.
    r0.extend([0; 8192]);
    records[0] = r0;

    pdb(&metadata.title, &records)
}

/// The records wrapped in a Palm database named after `title`.
fn pdb(title: &str, records: &[Vec<u8>]) -> Vec<u8> {
    let mut name: Vec<u8> = title
        .chars()
        .map(|c| match c {
            ' ' => b'_',
            c if c.is_ascii_alphanumeric() || "-_.,()[]!'&+=".contains(c) => c as u8,
            _ => b'_',
        })
        .take(31)
        .collect();
    if name.is_empty() {
        name = b"Unknown".to_vec();
    }
    name.resize(32, 0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let n = records.len();
    let mut out = name;
    out.extend(0u16.to_be_bytes()); // attributes
    out.extend(0u16.to_be_bytes()); // version
    out.extend(now.to_be_bytes()); // created
    out.extend(now.to_be_bytes()); // modified
    out.extend([0; 16]); // backed up, modification number, app info, sort info
    out.extend(b"BOOKMOBI");
    out.extend((2 * n as u32 - 1).to_be_bytes()); // unique ID seed
    out.extend(0u32.to_be_bytes()); // next record list
    out.extend((n as u16).to_be_bytes());
    let mut offset = out.len() + 8 * n + 2;
    for (i, rec) in records.iter().enumerate() {
        out.extend((offset as u32).to_be_bytes());
        out.extend(((2 * i) as u32).to_be_bytes());
        let attributes = out.len() - 4;
        out[attributes] = 0; // the unique ID is 3 bytes
        offset += rec.len();
    }
    out.extend([0, 0]);
    for rec in records {
        out.extend(rec);
    }
    out
}

fn uuid4() -> String {
    let mut b = random_bytes(16);
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The current time as `YYYY-MM-DDTHH:MM:SS+00:00`.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palm_database_layout() {
        let db = pdb(
            "Dune: Messiah ünd more more more more more",
            &[vec![1, 2], vec![3]],
        );
        assert_eq!(&db[..31], b"Dune__Messiah__nd_more_more_mor");
        assert_eq!(db[31], 0);
        assert_eq!(&db[60..68], b"BOOKMOBI");
        assert_eq!(u16::from_be_bytes([db[76], db[77]]), 2);
        let first = u32::from_be_bytes(db[78..82].try_into().unwrap()) as usize;
        assert_eq!(first, 78 + 16 + 2);
        assert_eq!(&db[90..94], &[0, 0, 0, 2]); // second record's attributes and ID
        assert_eq!(&db[first..], &[1, 2, 3]);
    }

    #[test]
    fn formats_dates_and_uuids() {
        let now = now_iso8601();
        assert_eq!(now.len(), 25);
        assert!(now.starts_with("20"));
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }
}
