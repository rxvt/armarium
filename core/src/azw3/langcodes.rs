// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2008 Kovid Goyal <kovid@kovidgoyal.net>
// Ported to Rust from calibre's src/calibre/ebooks/mobi/langcodes.py
// (calibre 9.16, commit 0b8c0c2).

//! The MOBI header's language code.

/// Region subtag ("" for none) → (main code, sub code).
type Regions = &'static [(&'static str, (u8, u8))];

/// IANA language → its regions.
const IANA_MOBI: &[(&str, Regions)] = &[
    ("af", &[("", (54, 0))]),
    (
        "ar",
        &[
            ("", (1, 0)),
            ("AE", (1, 56)),
            ("BH", (1, 60)),
            ("DZ", (1, 20)),
            ("EG", (1, 12)),
            ("JO", (1, 44)),
            ("KW", (1, 52)),
            ("LB", (1, 48)),
            ("MA", (1, 24)),
            ("OM", (1, 32)),
            ("QA", (1, 64)),
            ("SA", (1, 4)),
            ("SY", (1, 40)),
            ("TN", (1, 28)),
            ("YE", (1, 36)),
        ],
    ),
    ("as", &[("", (77, 0))]),
    ("az", &[("", (44, 0))]),
    ("be", &[("", (35, 0))]),
    ("bg", &[("", (2, 0))]),
    ("bn", &[("", (69, 0))]),
    ("ca", &[("", (3, 0))]),
    ("cs", &[("", (5, 0))]),
    ("da", &[("", (6, 0))]),
    (
        "de",
        &[
            ("", (7, 0)),
            ("AT", (7, 12)),
            ("CH", (7, 8)),
            ("LI", (7, 20)),
            ("LU", (7, 16)),
        ],
    ),
    ("el", &[("", (8, 0))]),
    (
        "en",
        &[
            ("", (9, 0)),
            ("AU", (9, 12)),
            ("BZ", (9, 40)),
            ("CA", (9, 16)),
            ("GB", (9, 8)),
            ("IE", (9, 24)),
            ("JM", (9, 32)),
            ("NZ", (9, 20)),
            ("PH", (9, 52)),
            ("TT", (9, 44)),
            ("US", (9, 4)),
            ("ZA", (9, 28)),
            ("ZW", (9, 48)),
        ],
    ),
    (
        "es",
        &[
            ("", (10, 0)),
            ("AR", (10, 44)),
            ("BO", (10, 64)),
            ("CL", (10, 52)),
            ("CO", (10, 36)),
            ("CR", (10, 20)),
            ("DO", (10, 28)),
            ("EC", (10, 48)),
            ("ES", (10, 4)),
            ("GT", (10, 16)),
            ("HN", (10, 72)),
            ("MX", (10, 8)),
            ("NI", (10, 76)),
            ("PA", (10, 24)),
            ("PE", (10, 40)),
            ("PR", (10, 80)),
            ("PY", (10, 60)),
            ("SV", (10, 68)),
            ("UY", (10, 56)),
            ("VE", (10, 32)),
        ],
    ),
    ("et", &[("", (37, 0))]),
    ("eu", &[("", (45, 0))]),
    ("fa", &[("", (41, 0))]),
    ("fi", &[("", (11, 0))]),
    ("fo", &[("", (56, 0))]),
    (
        "fr",
        &[
            ("", (12, 0)),
            ("BE", (12, 8)),
            ("CA", (12, 12)),
            ("CH", (12, 16)),
            ("FR", (12, 4)),
            ("LU", (12, 20)),
            ("MC", (12, 24)),
        ],
    ),
    ("gu", &[("", (71, 0))]),
    ("he", &[("", (13, 0))]),
    ("hi", &[("", (57, 0))]),
    ("hr", &[("", (26, 0))]),
    ("hu", &[("", (14, 0))]),
    ("hy", &[("", (43, 0))]),
    ("id", &[("", (33, 0))]),
    ("is", &[("", (15, 0))]),
    ("it", &[("", (16, 0)), ("CH", (16, 8)), ("IT", (16, 4))]),
    ("ja", &[("", (17, 0))]),
    ("ka", &[("", (55, 0))]),
    ("kk", &[("", (63, 0))]),
    ("kn", &[("", (75, 0))]),
    ("ko", &[("", (18, 0))]),
    ("kok", &[("", (87, 0))]),
    ("lt", &[("", (39, 0))]),
    ("lv", &[("", (38, 0))]),
    ("mk", &[("", (47, 0))]),
    ("ml", &[("", (76, 0))]),
    ("mr", &[("", (78, 0))]),
    ("ms", &[("", (62, 0))]),
    ("mt", &[("", (58, 0))]),
    ("ne", &[("", (97, 0))]),
    ("nl", &[("", (19, 0)), ("BE", (19, 8))]),
    ("no", &[("", (20, 0))]),
    ("or", &[("", (72, 0))]),
    ("pa", &[("", (70, 0))]),
    ("pl", &[("", (21, 0))]),
    ("pt", &[("", (22, 0)), ("BR", (22, 4)), ("PT", (22, 8))]),
    ("rm", &[("", (23, 0))]),
    ("ro", &[("", (24, 0))]),
    ("ru", &[("", (25, 0))]),
    ("sa", &[("", (79, 0))]),
    ("se", &[("", (59, 0))]),
    ("sk", &[("", (27, 0))]),
    ("sl", &[("", (36, 0))]),
    ("sq", &[("", (28, 0))]),
    ("sr", &[("", (26, 12)), ("RS", (26, 12))]),
    ("st", &[("", (48, 0))]),
    ("sv", &[("", (29, 0)), ("FI", (29, 8))]),
    ("sw", &[("", (65, 0))]),
    ("ta", &[("", (73, 0))]),
    ("te", &[("", (74, 0))]),
    ("th", &[("", (30, 0))]),
    ("tn", &[("", (50, 0))]),
    ("tr", &[("", (31, 0))]),
    ("ts", &[("", (49, 0))]),
    ("tt", &[("", (68, 0))]),
    ("uk", &[("", (34, 0))]),
    ("ur", &[("", (32, 0))]),
    ("uz", &[("", (67, 0)), ("UZ", (67, 8))]),
    ("vi", &[("", (42, 0))]),
    ("wen", &[("", (46, 0))]),
    ("xh", &[("", (52, 0))]),
    (
        "zh",
        &[
            ("", (4, 0)),
            ("CN", (4, 8)),
            ("HK", (4, 12)),
            ("SG", (4, 16)),
            ("TW", (4, 4)),
        ],
    ),
    ("zu", &[("", (53, 0))]),
];

/// ISO 639-2 codes (bibliographic and terminology) of the languages above,
/// standing in for calibre's full ISO 639 database.
const ISO639_2_TO_1: &[(&str, &str)] = &[
    ("afr", "af"),
    ("ara", "ar"),
    ("asm", "as"),
    ("aze", "az"),
    ("bel", "be"),
    ("bul", "bg"),
    ("ben", "bn"),
    ("cat", "ca"),
    ("ces", "cs"),
    ("cze", "cs"),
    ("dan", "da"),
    ("deu", "de"),
    ("ger", "de"),
    ("ell", "el"),
    ("gre", "el"),
    ("eng", "en"),
    ("spa", "es"),
    ("est", "et"),
    ("eus", "eu"),
    ("baq", "eu"),
    ("fas", "fa"),
    ("per", "fa"),
    ("fin", "fi"),
    ("fao", "fo"),
    ("fra", "fr"),
    ("fre", "fr"),
    ("guj", "gu"),
    ("heb", "he"),
    ("hin", "hi"),
    ("hrv", "hr"),
    ("hun", "hu"),
    ("hye", "hy"),
    ("arm", "hy"),
    ("ind", "id"),
    ("isl", "is"),
    ("ice", "is"),
    ("ita", "it"),
    ("jpn", "ja"),
    ("kat", "ka"),
    ("geo", "ka"),
    ("kaz", "kk"),
    ("kan", "kn"),
    ("kor", "ko"),
    ("lit", "lt"),
    ("lav", "lv"),
    ("mkd", "mk"),
    ("mac", "mk"),
    ("mal", "ml"),
    ("mar", "mr"),
    ("msa", "ms"),
    ("may", "ms"),
    ("mlt", "mt"),
    ("nep", "ne"),
    ("nld", "nl"),
    ("dut", "nl"),
    ("nor", "no"),
    ("ori", "or"),
    ("pan", "pa"),
    ("pol", "pl"),
    ("por", "pt"),
    ("roh", "rm"),
    ("ron", "ro"),
    ("rum", "ro"),
    ("rus", "ru"),
    ("san", "sa"),
    ("sme", "se"),
    ("slk", "sk"),
    ("slo", "sk"),
    ("slv", "sl"),
    ("sqi", "sq"),
    ("alb", "sq"),
    ("srp", "sr"),
    ("sot", "st"),
    ("swe", "sv"),
    ("swa", "sw"),
    ("tam", "ta"),
    ("tel", "te"),
    ("tha", "th"),
    ("tsn", "tn"),
    ("tur", "tr"),
    ("tso", "ts"),
    ("tat", "tt"),
    ("ukr", "uk"),
    ("urd", "ur"),
    ("uzb", "uz"),
    ("vie", "vi"),
    ("xho", "xh"),
    ("zho", "zh"),
    ("chi", "zh"),
    ("zul", "zu"),
];

/// The two-letter code for `lang` if it is a three-letter one we know.
pub fn as_iso639_1(lang: &str) -> String {
    let lower = lang.to_ascii_lowercase();
    ISO639_2_TO_1
        .iter()
        .find(|(three, _)| *three == lower)
        .map_or(lower, |(_, two)| (*two).to_owned())
}

/// The 4-byte MOBI language field for an IANA tag such as "en-GB".
pub fn iana2mobi(icode: &str) -> [u8; 4] {
    let mut subtags = icode.split(['-', '_']).filter(|s| !s.is_empty());
    let mut langdict: Regions = &[("", (0, 0))];
    for tag in subtags.by_ref() {
        let lang = as_iso639_1(tag);
        if let Some((_, d)) = IANA_MOBI.iter().find(|(code, _)| *code == lang) {
            langdict = d;
            break;
        }
    }
    let mut mcode = langdict[0].1;
    for subtag in subtags {
        let upper = subtag.to_ascii_uppercase();
        if let Some((_, code)) = langdict.iter().skip(1).find(|(r, _)| *r == upper) {
            mcode = *code;
            break;
        }
    }
    [0, 0, mcode.1, mcode.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_language_tags() {
        assert_eq!(iana2mobi("en"), [0, 0, 0, 9]);
        assert_eq!(iana2mobi("en-US"), [0, 0, 4, 9]);
        assert_eq!(iana2mobi("en-gb"), [0, 0, 8, 9]);
        assert_eq!(iana2mobi("fre"), [0, 0, 0, 12]);
        assert_eq!(iana2mobi("pt-BR"), [0, 0, 4, 22]);
        assert_eq!(iana2mobi("xx"), [0, 0, 0, 0]);
        assert_eq!(iana2mobi(""), [0, 0, 0, 0]);
    }
}
