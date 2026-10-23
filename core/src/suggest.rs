//! Suggestions for a free-text Metadata field from the values the Library
//! already uses, so new entries stay consistent with existing ones.

use std::collections::HashMap;

use crate::text;

/// The distinct values one field holds across the Library.
pub struct Suggestions {
    /// One per distinct value, in display order.
    entries: Vec<Entry>,
}

struct Entry {
    /// The spelling shown and filled in.
    value: String,
    /// `value` folded with [`text::fold`], for matching.
    folded: String,
    /// `value` as [`text::title_sort_key`] sees it, for matching and order.
    sort_key: String,
}

impl Suggestions {
    /// Collects the distinct `values`. Values that differ only by case,
    /// accents or surrounding whitespace count as one, shown in the spelling
    /// most of them use; empty values are skipped.
    pub fn new<'a>(values: impl IntoIterator<Item = &'a str>) -> Suggestions {
        // Per folded value, in first-seen order, its spellings in first-seen
        // order with counts; `group_of` finds a folded value's group.
        let mut groups: Vec<Vec<(&str, usize)>> = Vec::new();
        let mut group_of: HashMap<String, usize> = HashMap::new();
        for value in values {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let index = *group_of.entry(text::fold(value)).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            let spellings = &mut groups[index];
            match spellings.iter_mut().find(|(s, _)| *s == value) {
                Some((_, count)) => *count += 1,
                None => spellings.push((value, 1)),
            }
        }
        let mut entries: Vec<Entry> = groups
            .into_iter()
            .map(|spellings| {
                // max_by_key keeps the last of equals, so walk backwards to
                // let the first-seen spelling win a tie.
                let (value, _) = spellings
                    .into_iter()
                    .rev()
                    .max_by_key(|&(_, count)| count)
                    .unwrap();
                // Every spelling in a group folds the same, so refold the
                // winner rather than keep each folded value twice.
                Entry {
                    value: value.to_owned(),
                    folded: text::fold(value),
                    sort_key: text::title_sort_key(value),
                }
            })
            .collect();
        entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.value.cmp(&b.value)));
        Suggestions { entries }
    }

    /// The values that contain `typed`, ignoring case and accents: those
    /// starting with it first (a leading article aside), then the rest, each
    /// alphabetically. Nothing while `typed` is blank or already equals a
    /// value exactly.
    pub fn matching(&self, typed: &str) -> Vec<&str> {
        let typed = typed.trim();
        if typed.is_empty() || self.entries.iter().any(|e| e.value == typed) {
            return Vec::new();
        }
        let query = text::fold(typed);
        let (mut prefixed, rest): (Vec<&Entry>, Vec<&Entry>) = self
            .entries
            .iter()
            .filter(|e| e.folded.contains(&query))
            .partition(|e| e.folded.starts_with(&query) || e.sort_key.starts_with(&query));
        prefixed.extend(rest);
        prefixed.into_iter().map(|e| e.value.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggest(values: &[&str], typed: &str) -> Vec<String> {
        Suggestions::new(values.iter().copied())
            .matching(typed)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn matches_anywhere_ignoring_case_and_accents() {
        let values = ["Discworld", "The Lord of the Rings", "Brontë Classics"];
        assert_eq!(suggest(&values, "RING"), ["The Lord of the Rings"]);
        assert_eq!(suggest(&values, "bronte"), ["Brontë Classics"]);
        assert_eq!(suggest(&values, "Bronte\u{308}"), ["Brontë Classics"]);
        assert!(suggest(&values, "dune").is_empty());
    }

    #[test]
    fn prefix_matches_come_first_then_alphabetical() {
        let values = [
            "Mistborn",
            "The Stormlight Archive",
            "Storm Front",
            "Riftwar",
        ];
        // "The Stormlight Archive" starts with "storm" once its article is
        // dropped; "Mistborn" only contains "s".
        assert_eq!(
            suggest(&values, "storm"),
            ["Storm Front", "The Stormlight Archive"]
        );
        assert_eq!(
            suggest(&values, "r"),
            [
                "Riftwar",
                "Mistborn",
                "Storm Front",
                "The Stormlight Archive"
            ]
        );
    }

    #[test]
    fn variants_collapse_to_the_most_common_spelling() {
        let values = ["discworld ", "Discworld", "Discworld", "DISCWORLD"];
        assert_eq!(suggest(&values, "disc"), ["Discworld"]);
    }

    #[test]
    fn spelling_ties_go_to_the_first_seen() {
        let values = ["Dune Chronicles", "dune chronicles"];
        assert_eq!(suggest(&values, "dune"), ["Dune Chronicles"]);
    }

    #[test]
    fn nothing_for_blank_or_exactly_typed_text() {
        let values = ["Dune", "Dune Encyclopedia", "", "  "];
        assert!(suggest(&values, "").is_empty());
        assert!(suggest(&values, "   ").is_empty());
        assert!(suggest(&values, "Dune").is_empty());
        // A different case is not exact, so the Library's spelling is offered.
        assert_eq!(suggest(&values, "dune"), ["Dune", "Dune Encyclopedia"]);
    }
}
