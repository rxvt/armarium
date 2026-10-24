//! Finding Cover Candidates for a Book online, in the Apple Books store
//! (through the public iTunes Search API). This module only builds the
//! request and reads the response; the app does the network I/O.

use serde::Deserialize;

use crate::model::Metadata;
use crate::text;
use crate::{Error, Result};

/// How many results to ask for.
pub const LIMIT: u32 = 25;

/// Size requested for the picker's thumbnails.
const THUMBNAIL_SIZE: &str = "300x450";
/// Size requested for the chosen Cover. Apple scales artwork down to fit but
/// never past the original, which is usually about 1600×2400.
const FULL_SIZE: &str = "3000x3000";

/// An image found online that could become a Book's Cover.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverCandidate {
    /// The store's title and author, for a tooltip.
    pub title: String,
    pub author: String,
    pub thumbnail_url: String,
    pub full_url: String,
}

/// The search to start from: the title before any subtitle, and the first
/// author's surname ("Dune Messiah herbert").
pub fn search_term(metadata: &Metadata) -> String {
    let surname = metadata
        .authors
        .first()
        .and_then(|a| a.sort_name.split(',').next())
        .unwrap_or_default();
    format!("{} {}", main_title(&metadata.title), surname.trim())
        .trim()
        .to_owned()
}

/// The search request for `term`, in the US store (the default and the
/// largest catalogue of ebooks).
pub fn search_url(term: &str) -> String {
    format!(
        "https://itunes.apple.com/search?media=ebook&limit={LIMIT}&term={}",
        form_encode(term.trim())
    )
}

/// Reads a search response into candidates, those whose title is `title`
/// (ignoring subtitles, case and punctuation) first and otherwise in the
/// store's order. Results without artwork are dropped.
pub fn parse_results(json: &[u8], title: &str) -> Result<Vec<CoverCandidate>> {
    #[derive(Deserialize)]
    struct Response {
        results: Vec<Item>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        #[serde(default)]
        track_name: String,
        #[serde(default)]
        artist_name: String,
        artwork_url100: Option<String>,
    }

    let response: Response = serde_json::from_slice(json)
        .map_err(|e| Error::Unreadable(format!("unexpected response from Apple Books: {e}")))?;
    let wanted = text::match_key(main_title(title));
    let mut candidates: Vec<(bool, CoverCandidate)> = response
        .results
        .into_iter()
        .filter_map(|item| {
            let artwork = item.artwork_url100?;
            let same_title = text::match_key(main_title(&item.track_name)) == wanted;
            Some((
                same_title,
                CoverCandidate {
                    thumbnail_url: artwork_at(&artwork, THUMBNAIL_SIZE)?,
                    full_url: artwork_at(&artwork, FULL_SIZE)?,
                    title: item.track_name,
                    author: item.artist_name,
                },
            ))
        })
        .collect();
    candidates.sort_by_key(|(same_title, _)| !same_title);
    Ok(candidates.into_iter().map(|(_, c)| c).collect())
}

/// The title before any subtitle: "Dune: Deluxe Edition" → "Dune".
fn main_title(title: &str) -> &str {
    title.split([':', '(']).next().unwrap_or_default().trim()
}

/// An artwork URL ending in `/100x100bb.jpg` (any size and extension) at
/// `size` instead, as JPEG. `None` for URLs not in that shape.
fn artwork_at(url: &str, size: &str) -> Option<String> {
    let (base, last) = url.rsplit_once('/')?;
    let (dimensions, _extension) = last.split_once("bb.")?;
    let (w, h) = dimensions.split_once('x')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(w) && digits(h)).then(|| format!("{base}/{size}bb.jpg"))
}

/// Encodes `s` for a URL query value, spaces as `+`.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    fn metadata(title: &str, authors: &[&str]) -> Metadata {
        Metadata {
            title: title.to_owned(),
            authors: authors.iter().map(|&a| Author::new(a)).collect(),
            ..Metadata::default()
        }
    }

    #[test]
    fn term_is_main_title_and_first_surname() {
        let m = metadata("Dune Messiah: Book Two", &["Frank Herbert", "Someone Else"]);
        assert_eq!(search_term(&m), "Dune Messiah Herbert");
        assert_eq!(search_term(&metadata("Emma (Penguin)", &[])), "Emma");
        let king = metadata("Why We Can't Wait", &["Martin Luther King Jr."]);
        assert_eq!(search_term(&king), "Why We Can't Wait King");
    }

    #[test]
    fn url_encodes_the_term() {
        assert_eq!(
            search_url(" Brontë & co "),
            "https://itunes.apple.com/search?media=ebook&limit=25&term=Bront%C3%AB+%26+co"
        );
    }

    #[test]
    fn artwork_is_resized_to_jpeg() {
        let url = "https://is1-ssl.mzstatic.com/image/thumb/Pub/v4/ab/cd/x.png/100x100bb.png";
        assert_eq!(
            artwork_at(url, "300x450").as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/Pub/v4/ab/cd/x.png/300x450bb.jpg")
        );
        assert_eq!(artwork_at("https://example.com/cover.jpg", "300x450"), None);
    }

    #[test]
    fn same_title_first_and_no_artwork_dropped() {
        let json = br#"{"resultCount": 4, "results": [
            {"trackName": "Dune Messiah", "artistName": "Frank Herbert",
             "artworkUrl100": "https://a.example/m/100x100bb.jpg"},
            {"trackName": "No Art", "artistName": "Frank Herbert"},
            {"trackName": "Dune: Deluxe Edition", "artistName": "Frank Herbert",
             "artworkUrl100": "https://a.example/d/100x100bb.jpg"},
            {"trackName": "DUNE", "artistName": "Frank Herbert",
             "artworkUrl100": "https://a.example/u/60x60bb.png", "extra": 1}
        ]}"#;
        let found = parse_results(json, "Dune").unwrap();
        let titles: Vec<&str> = found.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Dune: Deluxe Edition", "DUNE", "Dune Messiah"]);
        assert_eq!(found[0].thumbnail_url, "https://a.example/d/300x450bb.jpg");
        assert_eq!(found[0].full_url, "https://a.example/d/3000x3000bb.jpg");
        assert_eq!(found[0].author, "Frank Herbert");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_results(b"<html>", "Dune").is_err());
        assert!(
            parse_results(br#"{"results": []}"#, "Dune")
                .unwrap()
                .is_empty()
        );
    }
}
