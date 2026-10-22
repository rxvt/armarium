// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2009 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/compression/palmdoc.c
// (calibre 9.16, commit 0b8c0c2).

//! PalmDoc (LZ77-style) compression of text records.

const MIN_MATCH_LEN: usize = 3;
const MAX_MATCH_LEN: usize = 10;
const MAX_MATCH_DIST: usize = 2047;
const HASH_BITS: u32 = 16;
const HASH_SIZE: usize = 1 << HASH_BITS;
const NONE: u32 = u32::MAX;

fn hash(p: &[u8]) -> usize {
    let v = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

/// Compresses `data`, producing exactly the same bytes as calibre.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut out = Vec::with_capacity(len);
    // Earlier positions are indexed by a hash of their first MIN_MATCH_LEN
    // bytes, in chains ordered nearest first, so a match search only visits
    // positions that can possibly match.
    let mut head = vec![NONE; HASH_SIZE];
    let mut prev = vec![NONE; len];
    let mut indexed = 0;
    let mut i = 0;

    while i < len {
        let c = data[i];
        if i > 10 && len - i > 10 {
            while indexed < i {
                let h = hash(&data[indexed..]);
                prev[indexed] = head[h];
                head[h] = indexed as u32;
                indexed += 1;
            }
            if let Some((chunk_len, dist)) = find_match(data, &head, &prev, i) {
                let compound = (dist << 3) + chunk_len - 3;
                out.push(0x80 + (compound >> 8) as u8);
                out.push((compound & 0xFF) as u8);
                i += chunk_len;
                continue;
            }
        }

        i += 1;
        if c == b' ' && i < len && (0x40..=0x7F).contains(&data[i]) {
            out.push(data[i] ^ 0x80);
            i += 1;
            continue;
        }
        if c == 0 || (c > 8 && c < 0x80) {
            out.push(c);
        } else {
            // A run of up to 8 bytes that would otherwise be read as codes.
            let start = i - 1;
            let mut end = i;
            while end < len
                && end - start < 8
                && !(data[end] == 0 || (data[end] > 8 && data[end] < 0x80))
            {
                end += 1;
            }
            out.push((end - start) as u8);
            out.extend_from_slice(&data[start..end]);
            i = end;
        }
    }
    out
}

/// The longest earlier match (at most MAX_MATCH_LEN, not overlapping `pos`,
/// within MAX_MATCH_DIST) for the text at `pos`, as (length, distance), the
/// nearest one winning ties.
fn find_match(data: &[u8], head: &[u32], prev: &[u32], pos: usize) -> Option<(usize, usize)> {
    let cur = &data[pos..pos + MAX_MATCH_LEN];
    let mut best = MIN_MATCH_LEN - 1;
    let mut best_dist = 0;
    let mut p = head[hash(cur)];
    while p != NONE {
        let d = pos - p as usize;
        if d > MAX_MATCH_DIST {
            break;
        }
        let cand = &data[p as usize..];
        let mut n = cur.iter().zip(cand).take_while(|(a, b)| a == b).count();
        n = n.min(d);
        if n > best {
            best = n;
            best_dist = d;
            if best == MAX_MATCH_LEN {
                break;
            }
        }
        p = prev[p as usize];
    }
    (best >= MIN_MATCH_LEN).then_some((best, best_dist))
}

/// Reverses [`compress`]; used by tests to check round trips.
#[cfg(test)]
pub fn decompress(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        i += 1;
        match c {
            1..=8 => {
                let end = (i + c as usize).min(data.len());
                out.extend_from_slice(&data[i..end]);
                i = end;
            }
            0..=0x7F => out.push(c),
            0xC0.. => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
            _ => {
                if i < data.len() {
                    let code = (c as usize) << 8 | data[i] as usize;
                    i += 1;
                    let dist = (code & 0x3FFF) >> 3;
                    if dist <= out.len() {
                        for _ in 0..(code & 7) + 3 {
                            out.push(out[out.len() - dist]);
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// calibre's pure-Python reference compressor: an exhaustive search for
    /// each match length, longest first.
    fn reference(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        let len = data.len();
        while i < len {
            if i > 10 && len - i > 10 {
                let mut found = None;
                for j in (3..=10).rev() {
                    let chunk = &data[i..i + j];
                    if let Some(m) = (0..=i - j).rev().find(|&m| &data[m..m + j] == chunk)
                        && i - m <= 2047
                    {
                        found = Some((j, i - m));
                        break;
                    }
                }
                if let Some((n, m)) = found {
                    let code = 0x8000 + ((m << 3) & 0x3FF8) + (n - 3);
                    out.extend_from_slice(&(code as u16).to_be_bytes());
                    i += n;
                    continue;
                }
            }
            let ch = data[i];
            i += 1;
            if ch == b' ' && i < len && (0x40..0x80).contains(&data[i]) {
                out.push(data[i] ^ 0x80);
                i += 1;
                continue;
            }
            if ch == 0 || (ch > 8 && ch < 0x80) {
                out.push(ch);
            } else {
                let mut seq = vec![ch];
                let mut j = i;
                while j < len && seq.len() < 8 {
                    let c = data[j];
                    if c == 0 || (c > 8 && c < 0x80) {
                        break;
                    }
                    seq.push(c);
                    j += 1;
                }
                out.push(seq.len() as u8);
                i += seq.len() - 1;
                out.extend(seq);
            }
        }
        out
    }

    fn check(data: &[u8]) {
        let x = compress(data);
        assert_eq!(x, reference(data), "len {}", data.len());
        assert_eq!(decompress(&x), data);
    }

    /// A small deterministic generator, so the tests need no extra crate.
    struct Lcg(u64);
    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn matches_calibre_test_cases() {
        for data in [
            &b"abc\x03\x04\x05\x06ms"[..],
            b"a b c \xfed ",
            b"0123456789axyz2bxyz2cdfgfo9iuyerh",
            b"0123456789asd0123456789asd|yyzzxxffhhjjkk",
            b"ciewacnaq eiu743 r787q 0w%  ; sa fd\xef\x0cfdxosac wocjp acoiecowei owaic jociowapjcivcjpoivjporeivjpoavca; p9aw8743y6r74%$^$^%8 ",
        ] {
            check(data);
        }
    }

    #[test]
    fn matches_the_reference_on_repetitive_and_random_data() {
        let mut rnd = Lcg(0);
        for alphabet in [2, 3, 4, 16, 200] {
            let data: Vec<u8> = (0..400).map(|_| 1 + rnd.below(alphabet) as u8).collect();
            check(&data);
        }
        let data: Vec<u8> = (0..300).map(|_| rnd.below(256) as u8).collect();
        check(&data);
        // A repeat distance of 2047 is the maximum.
        for gap in 2045..2050 {
            let base: Vec<u8> = (0..12).map(|_| 32 + rnd.below(95) as u8).collect();
            let mut data = base.clone();
            data.extend((0..gap - 12).map(|_| 128 + rnd.below(127) as u8));
            data.extend(&base);
            data.extend(b"tail");
            check(&data);
        }
        check(&b"abcdefghij".repeat(50));
        check(&[0; 500]);
    }

    #[test]
    fn handles_edge_cases() {
        assert_eq!(compress(b""), b"");
        for size in 1..25u8 {
            check(&vec![b'x'; size as usize]);
            check(&(1..=size).collect::<Vec<_>>());
            check(&b"ab ".repeat(size as usize));
        }
        let mut data: Vec<u8> = (1..9).cycle().take(24).collect();
        data.extend(0x80..=0xFF);
        data.extend(b"end");
        check(&data);
        for (input, expected) in [
            (&b" A"[..], &b"\xc1"[..]),
            (b" @", b"\xc0"),
            (b" \x7f", b"\xff"),
            (b" ?", b" ?"),
            (b" \x80", b" \x01\x80"),
            (b" ", b" "),
            (b"abc  A", b"abc \xc1"),
        ] {
            assert_eq!(compress(input), expected, "{input:?}");
        }
    }
}
