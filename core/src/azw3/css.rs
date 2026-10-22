//! Just enough CSS handling for the AZW3 writer: rewriting `url()` references
//! and `@import` rules, and dropping `height` from `body` rules. Comments and
//! strings are respected; everything else passes through untouched.

/// A piece of a stylesheet.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    /// Anything else, copied as is.
    Other(&'a str),
    /// `url(...)`, with the URL unquoted.
    Url { raw: &'a str, url: String },
    /// A quoted string, with its quotes, and the string inside.
    Str { raw: &'a str, value: String },
}

fn tokenize(css: &str) -> Vec<Token<'_>> {
    let bytes = css.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut other_start = 0;
    fn flush<'a>(css: &'a str, tokens: &mut Vec<Token<'a>>, from: usize, to: usize) {
        if to > from {
            tokens.push(Token::Other(&css[from..to]));
        }
    }
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 2);
            }
            q @ (b'"' | b'\'') => {
                let (end, value) = read_string(css, i, q);
                flush(css, &mut tokens, other_start, i);
                tokens.push(Token::Str {
                    raw: &css[i..end],
                    value,
                });
                i = end;
                other_start = i;
            }
            b'u' | b'U'
                if css[i..].len() >= 4
                    && css[i..i + 4].eq_ignore_ascii_case("url(")
                    && (i == 0 || !is_name_byte(bytes[i - 1])) =>
            {
                let mut j = i + 4;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let url;
                if let Some(&q @ (b'"' | b'\'')) = bytes.get(j) {
                    let (end, value) = read_string(css, j, q);
                    url = value;
                    j = end;
                    while j < bytes.len() && bytes[j] != b')' {
                        j += 1;
                    }
                } else {
                    let start = j;
                    while j < bytes.len() && bytes[j] != b')' {
                        j += 1;
                    }
                    url = unescape(css[start..j].trim());
                }
                let end = (j + 1).min(bytes.len());
                flush(css, &mut tokens, other_start, i);
                tokens.push(Token::Url {
                    raw: &css[i..end],
                    url,
                });
                i = end;
                other_start = i;
            }
            _ => i += 1,
        }
    }
    flush(css, &mut tokens, other_start, bytes.len());
    tokens
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Reads the string starting with quote `q` at `start`; returns the index
/// after it and its unescaped value.
fn read_string(css: &str, start: usize, q: u8) -> (usize, String) {
    let bytes = css.as_bytes();
    let mut j = start + 1;
    while j < bytes.len() && bytes[j] != q {
        if bytes[j] == b'\\' {
            j += 1;
        }
        j += 1;
    }
    let end = (j + 1).min(bytes.len());
    (end, unescape(&css[start + 1..j.min(bytes.len())]))
}

/// Resolves CSS backslash escapes.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(char::is_ascii_hexdigit) {
            hex.push(chars.next().unwrap());
        }
        if hex.is_empty() {
            match chars.next() {
                Some('\n') | None => {}
                Some(c) => out.push(c),
            }
        } else {
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            out.push(
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or('\u{fffd}'),
            );
        }
    }
    out
}

/// Replaces the targets of `url()` references (outside `@import` rules) for
/// which `replace` returns a new URL.
pub fn replace_urls(css: &str, mut replace: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(css.len());
    let mut in_import = false;
    for token in tokenize(css) {
        match token {
            Token::Other(s) => {
                out.push_str(s);
                update_import_state(s, &mut in_import);
            }
            Token::Str { raw, .. } => out.push_str(raw),
            Token::Url { raw, url } => match (!in_import).then(|| replace(&url)).flatten() {
                Some(new) => {
                    out.push_str("url(");
                    out.push_str(&new);
                    out.push(')');
                }
                None => out.push_str(raw),
            },
        }
    }
    out
}

/// Replaces the targets of `@import` rules for which `replace` returns a new
/// URL. Returns `None` if nothing changed.
pub fn replace_imports(
    css: &str,
    mut replace: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let mut out = String::with_capacity(css.len());
    let mut in_import = false;
    let mut changed = false;
    for token in tokenize(css) {
        let (raw, url) = match token {
            Token::Other(s) => {
                out.push_str(s);
                update_import_state(s, &mut in_import);
                continue;
            }
            Token::Str { raw, value } => (raw, value),
            Token::Url { raw, url } => (raw, url),
        };
        match in_import.then(|| replace(&url)).flatten() {
            Some(new) => {
                out.push_str(&format!("url({new})"));
                changed = true;
            }
            None => out.push_str(raw),
        }
        // Only the first URL of an import rule is its target.
        in_import = false;
    }
    changed.then_some(out)
}

/// Tracks whether we are between `@import` and its URL.
fn update_import_state(s: &str, in_import: &mut bool) {
    // Strip comments so `@import /* x */ "a.css"` still counts.
    let mut text = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("/*") {
        text.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |e| &rest[start + 2 + e + 2..]);
    }
    text.push_str(rest);
    if let Some(pos) = text.to_ascii_lowercase().rfind("@import") {
        *in_import = text[pos + 7..].trim().is_empty();
    } else if !text.trim().is_empty() {
        *in_import = false;
    }
}

/// Removes `height` declarations from rules that select `body` alone, since
/// some Kindles show all-black pages when the body has a height.
pub fn drop_body_height(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut prelude_start = 0;
    let mut i = 0;
    let bytes = css.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = css[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |e| i + 2 + e + 2);
            }
            q @ (b'"' | b'\'') => i = read_string(css, i, q).0,
            b'{' => {
                let prelude = &css[prelude_start..i];
                // A block with no nested block ends at the next '}'.
                let inner = &css[i + 1..];
                let close = inner
                    .find('}')
                    .filter(|&end| !inner[..end].contains('{'))
                    .map(|end| i + 1 + end);
                let selects_body = strip_comments(prelude)
                    .split(',')
                    .any(|s| s.trim().eq_ignore_ascii_case("body"));
                match close {
                    Some(end) if selects_body => {
                        out.push_str(&css[prelude_start..=i]);
                        out.push_str(&drop_height_decls(&css[i + 1..end]));
                        out.push('}');
                        i = end + 1;
                    }
                    _ => {
                        out.push_str(&css[prelude_start..=i]);
                        i += 1;
                    }
                }
                prelude_start = i;
                continue;
            }
            b'}' | b';' => {
                out.push_str(&css[prelude_start..=i]);
                i += 1;
                prelude_start = i;
                continue;
            }
            _ => i += 1,
        }
    }
    out.push_str(&css[prelude_start.min(css.len())..]);
    out
}

fn strip_comments(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |e| &rest[start + 2 + e + 2..]);
    }
    out.push_str(rest);
    out
}

/// A declaration list (as in a rule or a `style` attribute) without `height`.
pub fn drop_height_decls(decls: &str) -> String {
    decls
        .split(';')
        .filter(|d| {
            let name = strip_comments(d.split(':').next().unwrap_or(""));
            !name.trim().eq_ignore_ascii_case("height")
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_urls_but_not_imports_or_strings() {
        let css = r#"@import url(a.css); p { background: url( "img/x.png" ) } /* url(c.png) */ q { content: "url(d.png)" } @font-face { src: url(f.ttf) format("truetype") }"#;
        let out = replace_urls(css, |u| Some(format!("<{u}>")));
        assert_eq!(
            out,
            r#"@import url(a.css); p { background: url(<img/x.png>) } /* url(c.png) */ q { content: "url(d.png)" } @font-face { src: url(<f.ttf>) format("truetype") }"#
        );
    }

    #[test]
    fn leaves_urls_the_callback_declines() {
        let css = "a { b: url('x.png') }";
        assert_eq!(replace_urls(css, |_| None), css);
    }

    #[test]
    fn replaces_import_targets() {
        let css = "@import \"a.css\";\n@import url(b.css) screen;\np { background: url(c.png) }";
        let out = replace_imports(css, |u| (u != "c.png").then(|| format!("<{u}>"))).unwrap();
        assert_eq!(
            out,
            "@import url(<a.css>);\n@import url(<b.css>) screen;\np { background: url(c.png) }"
        );
        assert_eq!(
            replace_imports("p { x: url(a.png) }", |u| Some(u.into())),
            None
        );
    }

    #[test]
    fn drops_height_from_body_only() {
        let css = "body { height: 100%; margin: 0 } div, BODY {height:1em} p { height: 2em } @media x { body { height: 3em } }";
        assert_eq!(
            drop_body_height(css),
            "body { margin: 0 } div, BODY {} p { height: 2em } @media x { body {} }"
        );
        assert_eq!(drop_height_decls("height: 1px; color: red"), " color: red");
    }
}
