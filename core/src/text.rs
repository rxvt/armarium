//! Text helpers for matching, searching and sorting.

/// Folds `s` for case- and diacritic-insensitive comparison: lowercases it and
/// strips accents from Latin letters ("Brontë" → "bronte").
///
/// Covers Latin-1 Supplement and Latin Extended-A, which is enough for the
/// Western European names a personal library mostly holds; other scripts are
/// only lowercased.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match strip_accent(c) {
            Some(base) => out.push_str(base),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// Normalizes `s` for deciding whether two Books are the same work: folds it,
/// drops punctuation and collapses whitespace ("The Hobbit: or, There and
/// Back Again" and "the hobbit or there and back again" match).
pub fn match_key(s: &str) -> String {
    let folded = fold(s);
    let mut out = String::with_capacity(folded.len());
    for word in folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Derives an author's sort name: "Frank Herbert" → "Herbert, Frank",
/// "Martin Luther King Jr." → "King, Martin Luther, Jr.". Names that already
/// contain a comma, or are a single word, are returned unchanged.
pub fn author_sort_name(name: &str) -> String {
    let name = name.trim();
    if name.contains(',') {
        return name.to_owned();
    }
    let mut words: Vec<&str> = name.split_whitespace().collect();
    let suffix = match words.last() {
        Some(last) if words.len() > 1 && is_name_suffix(last) => words.pop(),
        _ => None,
    };
    if words.len() < 2 {
        return name.to_owned();
    }
    let surname = words.pop().unwrap();
    let mut sort = format!("{surname}, {}", words.join(" "));
    if let Some(suffix) = suffix {
        sort.push_str(", ");
        sort.push_str(suffix);
    }
    sort
}

fn is_name_suffix(word: &str) -> bool {
    matches!(
        word.trim_end_matches('.').to_lowercase().as_str(),
        "jr" | "sr" | "ii" | "iii" | "iv"
    )
}

fn strip_accent(c: char) -> Option<&'static str> {
    Some(match c {
        'À'..='Å' | 'à'..='å' | 'Ā' | 'ā' | 'Ă' | 'ă' | 'Ą' | 'ą' => "a",
        'Æ' | 'æ' => "ae",
        'Ç' | 'ç' | 'Ć' | 'ć' | 'Ĉ' | 'ĉ' | 'Ċ' | 'ċ' | 'Č' | 'č' => "c",
        'Ð' | 'ð' | 'Ď' | 'ď' | 'Đ' | 'đ' => "d",
        'È'..='Ë' | 'è'..='ë' | 'Ē' | 'ē' | 'Ĕ' | 'ĕ' | 'Ė' | 'ė' | 'Ę' | 'ę' | 'Ě' | 'ě' => {
            "e"
        }
        'Ĝ' | 'ĝ' | 'Ğ' | 'ğ' | 'Ġ' | 'ġ' | 'Ģ' | 'ģ' => "g",
        'Ĥ' | 'ĥ' | 'Ħ' | 'ħ' => "h",
        'Ì'..='Ï' | 'ì'..='ï' | 'Ĩ' | 'ĩ' | 'Ī' | 'ī' | 'Ĭ' | 'ĭ' | 'Į' | 'į' | 'İ' | 'ı' => {
            "i"
        }
        'Ĳ' | 'ĳ' => "ij",
        'Ĵ' | 'ĵ' => "j",
        'Ķ' | 'ķ' | 'ĸ' => "k",
        'Ĺ' | 'ĺ' | 'Ļ' | 'ļ' | 'Ľ' | 'ľ' | 'Ŀ' | 'ŀ' | 'Ł' | 'ł' => "l",
        'Ñ' | 'ñ' | 'Ń' | 'ń' | 'Ņ' | 'ņ' | 'Ň' | 'ň' | 'ŉ' | 'Ŋ' | 'ŋ' => "n",
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | 'Ō' | 'ō' | 'Ŏ' | 'ŏ' | 'Ő' | 'ő' => "o",
        'Œ' | 'œ' => "oe",
        'Ŕ' | 'ŕ' | 'Ŗ' | 'ŗ' | 'Ř' | 'ř' => "r",
        'Ś' | 'ś' | 'Ŝ' | 'ŝ' | 'Ş' | 'ş' | 'Š' | 'š' | 'ſ' => "s",
        'ß' => "ss",
        'Ţ' | 'ţ' | 'Ť' | 'ť' | 'Ŧ' | 'ŧ' => "t",
        'Þ' | 'þ' => "th",
        'Ù'..='Ü'
        | 'ù'..='ü'
        | 'Ũ'
        | 'ũ'
        | 'Ū'
        | 'ū'
        | 'Ŭ'
        | 'ŭ'
        | 'Ů'
        | 'ů'
        | 'Ű'
        | 'ű'
        | 'Ų'
        | 'ų' => "u",
        'Ŵ' | 'ŵ' => "w",
        'Ý' | 'ý' | 'ÿ' | 'Ŷ' | 'ŷ' | 'Ÿ' => "y",
        'Ź' | 'ź' | 'Ż' | 'ż' | 'Ž' | 'ž' => "z",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_strips_case_and_diacritics() {
        assert_eq!(fold("Brontë"), "bronte");
        assert_eq!(fold("ÉMILE Zola"), "emile zola");
        assert_eq!(fold("Straße"), "strasse");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("Толстой"), "толстой");
    }

    #[test]
    fn match_key_ignores_punctuation_and_spacing() {
        assert_eq!(
            match_key("The Hobbit: or, There and Back Again"),
            match_key("the hobbit   or there and back again")
        );
        assert_eq!(match_key("  Brontë, Charlotte. "), "bronte charlotte");
        assert_eq!(match_key("!!!"), "");
    }

    #[test]
    fn sort_name_puts_surname_first() {
        assert_eq!(author_sort_name("Frank Herbert"), "Herbert, Frank");
        // Particles are not recognised; the user can edit the sort name.
        assert_eq!(
            author_sort_name(" Ursula K. Le Guin "),
            "Guin, Ursula K. Le"
        );
        assert_eq!(
            author_sort_name("Martin Luther King Jr."),
            "King, Martin Luther, Jr."
        );
        assert_eq!(author_sort_name("Herbert, Frank"), "Herbert, Frank");
        assert_eq!(author_sort_name("Homer"), "Homer");
        assert_eq!(author_sort_name("Smith Jr."), "Smith Jr.");
    }
}
