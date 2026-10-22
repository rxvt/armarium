// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2012 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/writer8/exth.py
// (calibre 9.16, commit 0b8c0c2).

//! The EXTH header: the book's Metadata and pointers into it.

use super::langcodes::as_iso639_1;
use super::utils::{to_base32, utf8_text};
use crate::model::Metadata;

const CREATOR: u32 = 100;
const PUBLISHER: u32 = 101;
const DESCRIPTION: u32 = 103;
const IDENTIFIER: u32 = 104;
const PUBDATE: u32 = 106;
const SOURCE: u32 = 112;
const ASIN: u32 = 113;
const START_READING: u32 = 116;
const NUM_OF_RESOURCES: u32 = 125;
const KF8_THUMBNAIL_URI: u32 = 129;
const KF8_UNKNOWN_COUNT: u32 = 131;
const COVER_OFFSET: u32 = 201;
const THUMB_OFFSET: u32 = 202;
const HAS_FAKE_COVER: u32 = 203;
const CDE_TYPE: u32 = 501;
const TITLE: u32 = 503;
const LANGUAGE: u32 = 524;
const PRIMARY_WRITING_MODE: u32 = 525;
const PAGE_PROGRESSION_DIRECTION: u32 = 527;
const OVERRIDE_KINDLE_FONTS: u32 = 528;
const CREATOR_BUILD_TAG: u32 = 535;

#[derive(Debug, Default)]
pub struct ExthOptions<'a> {
    /// Identifies the book: its source, and its ASIN unless `share_not_sync`.
    pub uuid: &'a str,
    /// Write the book as a personal document: no ASIN and no `EBOK` type.
    /// The Kindle then keeps the embedded Cover instead of fetching a (blank)
    /// one from Amazon, but does not sync the reading position.
    pub share_not_sync: bool,
    /// As found or entered, e.g. "1965" or "2019-05-07T00:00:00+00:00".
    pub pubdate: &'a str,
    pub cover_offset: Option<usize>,
    pub thumbnail_offset: Option<usize>,
    pub start_offset: Option<usize>,
    pub num_of_resources: usize,
    pub kf8_unknown_count: Option<u32>,
    pub page_progression_direction: Option<&'a str>,
    pub primary_writing_mode: Option<&'a str>,
}

struct Exth {
    buf: Vec<u8>,
    count: u32,
}

impl Exth {
    fn bytes(&mut self, code: u32, data: &[u8]) {
        self.buf.extend(code.to_be_bytes());
        self.buf.extend((data.len() as u32 + 8).to_be_bytes());
        self.buf.extend(data);
        self.count += 1;
    }

    fn int(&mut self, code: u32, value: u32) {
        self.bytes(code, &value.to_be_bytes());
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn build_exth(metadata: &Metadata, opts: &ExthOptions) -> Vec<u8> {
    let mut exth = Exth {
        buf: Vec::new(),
        count: 0,
    };

    exth.bytes(TITLE, &utf8_text(&collapse_whitespace(&metadata.title)));
    for author in &metadata.authors {
        exth.bytes(CREATOR, &utf8_text(&collapse_whitespace(&author.name)));
    }
    if let Some(publisher) = &metadata.publisher {
        exth.bytes(PUBLISHER, &utf8_text(&collapse_whitespace(publisher)));
    }
    if let Some(description) = &metadata.description {
        exth.bytes(DESCRIPTION, &utf8_text(description));
    }
    if let Some(isbn) = &metadata.isbn {
        exth.bytes(IDENTIFIER, &utf8_text(isbn));
    }
    if let Some(language) = &metadata.language {
        exth.bytes(
            LANGUAGE,
            &utf8_text(&as_iso639_1(&collapse_whitespace(language))),
        );
    }

    let uuid = opts.uuid.as_bytes();
    if !opts.share_not_sync {
        exth.bytes(ASIN, uuid);
    }
    exth.bytes(SOURCE, &[b"armarium:", uuid].concat());
    if !opts.share_not_sync {
        exth.bytes(CDE_TYPE, b"EBOK");
    }
    exth.bytes(PUBDATE, opts.pubdate.as_bytes());

    // Pretend to be kindlegen 2 on Linux.
    for (code, value) in [(204, 201), (205, 2), (206, 9), (207, 0)] {
        exth.int(code, value);
    }
    exth.bytes(CREATOR_BUILD_TAG, b"0730-890adc2");

    if let Some(cover) = opts.cover_offset {
        exth.int(COVER_OFFSET, cover as u32);
        exth.int(HAS_FAKE_COVER, 0);
    }
    if let Some(thumb) = opts.thumbnail_offset {
        exth.int(THUMB_OFFSET, thumb as u32);
        exth.bytes(
            KF8_THUMBNAIL_URI,
            format!("kindle:embed:{}", to_base32(thumb, 4)).as_bytes(),
        );
    }
    if let Some(start) = opts.start_offset {
        exth.int(START_READING, start as u32);
    }
    exth.int(NUM_OF_RESOURCES, opts.num_of_resources as u32);
    if let Some(count) = opts.kf8_unknown_count {
        exth.int(KF8_UNKNOWN_COUNT, count);
    }
    if let Some(pwm) = opts.primary_writing_mode {
        exth.bytes(PRIMARY_WRITING_MODE, pwm.as_bytes());
    }
    if let Some(ppd) = opts
        .page_progression_direction
        .filter(|p| matches!(*p, "rtl" | "ltr" | "default"))
    {
        exth.bytes(PAGE_PROGRESSION_DIRECTION, ppd.as_bytes());
    }
    exth.bytes(OVERRIDE_KINDLE_FONTS, b"true");

    // Always padded with at least one byte.
    let pad = 4 - exth.buf.len() % 4;
    let mut out = b"EXTH".to_vec();
    out.extend((exth.buf.len() as u32 + 12).to_be_bytes());
    out.extend(exth.count.to_be_bytes());
    out.extend(exth.buf);
    out.extend(std::iter::repeat_n(0, pad));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    /// The EXTH records as (code, data).
    fn records(exth: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let u32_at = |i: usize| u32::from_be_bytes(exth[i..i + 4].try_into().unwrap());
        let count = u32_at(8);
        let mut pos = 12;
        let mut out = Vec::new();
        for _ in 0..count {
            let (code, len) = (u32_at(pos), u32_at(pos + 4) as usize);
            out.push((code, exth[pos + 8..pos + len].to_vec()));
            pos += len;
        }
        assert_eq!(u32_at(4) as usize, pos);
        out
    }

    #[test]
    fn writes_metadata_and_pointers() {
        let metadata = Metadata {
            title: "Dune\n Messiah".into(),
            authors: vec![Author::new("Frank Herbert")],
            language: Some("eng".into()),
            ..Metadata::default()
        };
        let exth = build_exth(
            &metadata,
            &ExthOptions {
                uuid: "abc",
                pubdate: "1969",
                cover_offset: Some(0),
                thumbnail_offset: Some(1),
                num_of_resources: 2,
                kf8_unknown_count: Some(0),
                ..ExthOptions::default()
            },
        );
        assert_eq!(&exth[..4], b"EXTH");
        assert_eq!(exth.len() % 4, 0);
        let recs = records(&exth);
        let get = |code: u32| {
            recs.iter()
                .find(|(c, _)| *c == code)
                .map(|(_, d)| d.clone())
        };
        assert_eq!(get(TITLE).unwrap(), b"Dune Messiah");
        assert_eq!(get(CREATOR).unwrap(), b"Frank Herbert");
        assert_eq!(get(LANGUAGE).unwrap(), b"en");
        assert_eq!(get(ASIN).unwrap(), b"abc");
        assert_eq!(get(CDE_TYPE).unwrap(), b"EBOK");
        assert_eq!(get(COVER_OFFSET).unwrap(), 0u32.to_be_bytes());
        assert_eq!(get(KF8_THUMBNAIL_URI).unwrap(), b"kindle:embed:0001");
        assert_eq!(get(START_READING), None);

        let personal = records(&build_exth(
            &metadata,
            &ExthOptions {
                uuid: "abc",
                share_not_sync: true,
                ..ExthOptions::default()
            },
        ));
        assert!(!personal.iter().any(|(c, _)| *c == ASIN || *c == CDE_TYPE));
        assert!(
            personal
                .iter()
                .any(|(c, d)| *c == SOURCE && d == b"armarium:abc")
        );
    }
}
