// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2011 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/utils.py
// (calibre 9.16, commit 0b8c0c2).

//! Number encodings, text records and CNCX string tables shared by the KF8
//! writer.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Uncompressed size of a text record.
pub const RECORD_SIZE: usize = 0x1000;

/// Encodes `value` as a variable-width integer: big-endian, 7 bits per byte.
/// Forward-encoded numbers (read from the front) have the high bit set on
/// their last byte, backward-encoded ones (read from the end) on their first.
pub fn encint(value: u64, forward: bool) -> Vec<u8> {
    let mut value = value;
    let mut bytes = Vec::new();
    loop {
        bytes.push((value & 0x7F) as u8);
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    let flagged = if forward { 0 } else { bytes.len() - 1 };
    bytes[flagged] |= 0x80;
    bytes.reverse();
    bytes
}

/// Reads a variable-width integer from the start (`forward`) or end of
/// `raw`, returning it and the number of bytes it took.
#[cfg(test)]
pub fn decint(raw: &[u8], forward: bool) -> (u64, usize) {
    let mut bytes: Vec<u8> = Vec::new();
    let mut take = |b: u8| {
        bytes.push(b & 0x7F);
        b & 0x80 != 0
    };
    if forward {
        for &b in raw {
            if take(b) {
                break;
            }
        }
    } else {
        for &b in raw.iter().rev() {
            if take(b) {
                break;
            }
        }
        bytes.reverse();
    }
    let value = bytes.iter().fold(0u64, |v, &b| v << 7 | b as u64);
    (value, bytes.len())
}

/// `<data><size>`, where size is a backward-encoded integer giving the length
/// of the whole thing: one trailing entry of a text record.
pub fn encode_trailing_data(raw: &[u8]) -> Vec<u8> {
    let mut lsize = 1;
    loop {
        let encoded = encint((raw.len() + lsize) as u64, false);
        if encoded.len() == lsize {
            let mut out = raw.to_vec();
            out.extend(encoded);
            return out;
        }
        lsize += 1;
    }
}

/// `val` with the low `flag_size` bits of `flags` packed below it.
fn encode_fvwi(val: u64, flags: u8, flag_size: u32) -> Vec<u8> {
    let mut ans = val << flag_size;
    for i in 0..flag_size {
        ans |= (flags & (1 << i)) as u64;
    }
    encint(ans, true)
}

/// Extra data that may follow one number in a trailing byte sequence, keyed
/// by flag bit.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TbsExtra {
    /// 0b1000
    pub strand_flag: bool,
    /// 0b0010
    pub tbs_type: Option<u64>,
    /// 0b0100
    pub count: Option<u8>,
    /// 0b0001
    pub offset: Option<u64>,
}

/// One number of a trailing byte sequence plus its extra data.
pub fn encode_tbs(val: u64, extra: &TbsExtra, flag_size: u32) -> Vec<u8> {
    let mut flags = 0;
    if extra.strand_flag {
        flags |= 0b1000;
    }
    if extra.tbs_type.is_some() {
        flags |= 0b0010;
    }
    if extra.count.is_some() {
        flags |= 0b0100;
    }
    if extra.offset.is_some() {
        flags |= 0b0001;
    }
    let mut ans = encode_fvwi(val, flags, flag_size);
    if let Some(t) = extra.tbs_type {
        ans.extend(encint(t, true));
    }
    if let Some(c) = extra.count {
        ans.push(c);
    }
    if let Some(o) = extra.offset {
        ans.extend(encint(o, true));
    }
    ans
}

/// `text` trimmed, as UTF-8, or "Unknown" if nothing is left.
pub fn utf8_text(text: &str) -> Vec<u8> {
    let text = text.trim();
    if text.is_empty() {
        b"Unknown".to_vec()
    } else {
        text.as_bytes().to_vec()
    }
}

/// `raw` padded with zeroes to a multiple of 4 bytes.
pub fn align_block(mut raw: Vec<u8>) -> Vec<u8> {
    let extra = raw.len() % 4;
    if extra != 0 {
        raw.resize(raw.len() + 4 - extra, 0);
    }
    raw
}

/// `num` in base 32 (digits 0-9A-V), zero-padded to `min_digits`.
pub fn to_base32(num: usize, min_digits: usize) -> String {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut num = num;
    let mut ans = Vec::new();
    while num > 0 {
        ans.push(DIGITS[num % 32]);
        num /= 32;
    }
    while ans.len() < min_digits.max(1) {
        ans.push(b'0');
    }
    ans.reverse();
    String::from_utf8(ans).unwrap()
}

/// The record number `num` as KF8 refers to it in `kindle:` links.
pub fn to_ref(num: usize) -> String {
    to_base32(num, 4)
}

/// Splits the next text record off `text` starting at `pos`: RECORD_SIZE
/// bytes, plus the bytes needed to finish a UTF-8 character cut at the end
/// (the overlap), which the reader must skip.
pub fn create_text_record(text: &[u8], pos: usize) -> (&[u8], &[u8]) {
    let end = (pos + RECORD_SIZE).min(text.len());
    let data = &text[pos..end];
    // Walk back to the start of the last character and see whether it is
    // complete.
    let mut start = end;
    while start > pos && start > end.saturating_sub(4) && text[start - 1] & 0xC0 == 0x80 {
        start -= 1;
    }
    let mut extra = 0;
    if start > pos {
        let lead = text[start - 1];
        let width = match lead {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        let have = end - (start - 1);
        if width > have {
            extra = (width - have).min(text.len() - end);
        }
    }
    (data, &text[end..end + extra])
}

/// A font as a MOBI FONT record: zlib-compressed, with the first 1040 bytes
/// XOR-obfuscated with a random key.
pub fn write_font_record(data: &[u8]) -> Vec<u8> {
    const KEY_LEN: usize = 20;
    let usize = data.len() as u32;
    let mut flags = 0b1;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data).expect("writing to a Vec cannot fail");
    let mut data = enc.finish().expect("writing to a Vec cannot fail");
    let mut xor_key = Vec::new();
    if data.len() >= 1040 {
        flags |= 0b10;
        xor_key = random_bytes(KEY_LEN);
        for i in 0..1040 {
            data[i] ^= xor_key[i % KEY_LEN];
        }
    }
    let key_start = 24u32;
    let data_start = key_start + xor_key.len() as u32;
    let mut out = b"FONT".to_vec();
    for v in [usize, flags, data_start, xor_key.len() as u32, key_start] {
        out.extend(v.to_be_bytes());
    }
    out.extend(xor_key);
    out.extend(data);
    out
}

/// `n` bytes from the system's random source, falling back to the clock.
pub fn random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0; n];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_err()
    {
        let mut seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
            ^ std::process::id() as u64;
        for b in &mut buf {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (seed >> 33) as u8;
        }
    }
    buf
}

/// The strings of an index, packed into records as `<vwi length><utf-8>`.
/// Strings are referred to by offset; each record adds 0x10000 to the
/// offsets of the strings in it.
#[derive(Debug, Default)]
pub struct Cncx {
    offsets: Vec<(String, u64)>,
    pub records: Vec<Vec<u8>>,
}

impl Cncx {
    const MAX_STRING_LENGTH: usize = 500;
    /// kindlegen appears to use 1024 less than the PDB limit of 0x10000.
    const RECORD_LIMIT: usize = 0x10000 - 1024;

    pub fn new<'a>(strings: impl IntoIterator<Item = &'a str>) -> Cncx {
        let mut cncx = Cncx::default();
        let mut buf: Vec<u8> = Vec::new();
        let mut offset = 0u64;
        for s in strings {
            if cncx.offsets.iter().any(|(k, _)| k == s) {
                continue;
            }
            let truncated: String = s.chars().take(Self::MAX_STRING_LENGTH).collect();
            let utf8 = utf8_text(&truncated);
            let mut raw = encint(utf8.len() as u64, true);
            raw.extend(utf8);
            if buf.len() + raw.len() > Self::RECORD_LIMIT {
                cncx.records.push(align_block(std::mem::take(&mut buf)));
                offset = cncx.records.len() as u64 * 0x10000;
            }
            buf.extend(&raw);
            cncx.offsets.push((s.to_owned(), offset));
            offset += raw.len() as u64;
        }
        if !buf.is_empty() {
            cncx.records.push(align_block(buf));
        }
        cncx
    }

    pub fn offset(&self, s: &str) -> u64 {
        self.offsets
            .iter()
            .find(|(k, _)| k == s)
            .map(|(_, o)| *o)
            .expect("string added to the CNCX")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_width_integers_round_trip() {
        assert_eq!(encint(0x11111, true), [0x04, 0x22, 0x91]);
        assert_eq!(encint(0x11111, false), [0x84, 0x22, 0x11]);
        for n in [0, 1, 127, 128, 0x3FFF, 0x4000, 0x11111, u32::MAX as u64] {
            for forward in [true, false] {
                let raw = encint(n, forward);
                assert_eq!(decint(&raw, forward), (n, raw.len()));
            }
        }
    }

    #[test]
    fn trailing_data_ends_with_its_own_length() {
        let out = encode_trailing_data(b"abc");
        assert_eq!(out, b"abc\x84");
        let long = vec![7; 200];
        let out = encode_trailing_data(&long);
        assert_eq!(decint(&out, false).0 as usize, out.len());
    }

    #[test]
    fn base32_matches_calibre() {
        assert_eq!(to_base32(0, 4), "0000");
        assert_eq!(to_base32(31, 1), "V");
        assert_eq!(to_base32(32, 4), "0010");
        assert_eq!(to_base32(1_000_001, 1), "UGI1");
        assert_eq!(to_ref(5), "0005");
    }

    #[test]
    fn text_records_never_split_a_character() {
        let mut text = vec![b'a'; RECORD_SIZE - 1];
        text.extend("€bc".as_bytes()); // a 3-byte character across the boundary
        let (data, overlap) = create_text_record(&text, 0);
        assert_eq!(data.len(), RECORD_SIZE);
        assert_eq!(overlap, &"€".as_bytes()[1..]);
        let (data, overlap) = create_text_record(&text, RECORD_SIZE);
        assert_eq!(data, &"€bc".as_bytes()[1..]);
        assert!(overlap.is_empty());

        let ascii = vec![b'x'; RECORD_SIZE + 5];
        assert!(create_text_record(&ascii, 0).1.is_empty());
    }

    #[test]
    fn cncx_records_offsets_by_record() {
        let cncx = Cncx::new(["One", "Two", "One"]);
        assert_eq!(cncx.offset("One"), 0);
        assert_eq!(cncx.offset("Two"), 4);
        assert_eq!(cncx.records, [b"\x83One\x83Two".to_vec()]);
    }

    #[test]
    fn font_records_round_trip() {
        let font = random_bytes(5000);
        let rec = write_font_record(&font);
        let field = |i: usize| u32::from_be_bytes(rec[4 + 4 * i..8 + 4 * i].try_into().unwrap());
        let (usize, flags, dstart, xor_len, xor_start) =
            (field(0), field(1), field(2), field(3), field(4));
        assert_eq!(usize as usize, font.len());
        assert_eq!(flags, 0b11);
        let key = &rec[xor_start as usize..(xor_start + xor_len) as usize];
        let mut data = rec[dstart as usize..].to_vec();
        for i in 0..1040.min(data.len()) {
            data[i] ^= key[i % key.len()];
        }
        let mut out = Vec::new();
        use std::io::Read;
        flate2::read::ZlibDecoder::new(&data[..])
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, font);
    }
}
