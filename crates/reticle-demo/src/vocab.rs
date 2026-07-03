//! The task-vocabulary input filter.
//!
//! The demo must not become a general-purpose model proxy, so a prompt may only
//! use words drawn from the deployment's [`allowed_vocabulary`] plus a small
//! built-in stoplist of common function words (articles, prepositions, and the
//! like). A prompt containing any other word is rejected before it reaches a
//! model.
//!
//! Matching is case-insensitive and punctuation-insensitive: the prompt is
//! lowercased and split on any non-alphanumeric character, so `"Place a metal1
//! rectangle."` yields the words `place`, `a`, `metal1`, `rectangle`. Pure
//! numbers (coordinates, widths) are always allowed. An empty allowed vocabulary
//! is treated as *unconfigured* and permits any words, so a deployment that only
//! wants the length limit is not forced to enumerate a vocabulary; a deployment
//! that wants the filter provides a non-empty set.
//!
//! [`allowed_vocabulary`]: crate::LimitConfig::allowed_vocabulary

use std::collections::HashSet;

/// Common English function words that are always permitted, so a deployment's
/// task vocabulary only has to list domain terms, not glue words.
const STOPLIST: &[&str] = &[
    "a", "an", "the", "and", "or", "of", "to", "in", "on", "at", "by", "for", "with", "from",
    "into", "onto", "as", "is", "are", "be", "please", "then", "that", "this", "it", "its", "each",
    "all", "some", "any", "no", "not", "if", "so", "up", "down", "left", "right", "near", "next",
    "between",
];

/// Splits `prompt` into lowercased word tokens on any non-alphanumeric boundary,
/// skipping empty tokens.
fn tokenize(prompt: &str) -> impl Iterator<Item = String> + '_ {
    prompt
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
}

/// Returns the words in `prompt` that are outside the allowed vocabulary.
///
/// A word is allowed when it is purely numeric, appears in the built-in
/// stoplist, or appears in `allowed` (compared case-insensitively). The returned
/// list preserves first-seen order and contains no duplicates, so it reads as the
/// distinct set of offending tokens.
///
/// When `allowed` is empty the filter is unconfigured and returns an empty list
/// (everything is permitted); see the module docs.
#[must_use]
pub fn offending_words(prompt: &str, allowed: &[String]) -> Vec<String> {
    if allowed.is_empty() {
        return Vec::new();
    }
    let allowed_set: HashSet<String> = allowed.iter().map(|w| w.to_lowercase()).collect();
    let stop: HashSet<&str> = STOPLIST.iter().copied().collect();

    let mut seen = HashSet::new();
    let mut bad = Vec::new();
    for word in tokenize(prompt) {
        if word.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if stop.contains(word.as_str()) || allowed_set.contains(&word) {
            continue;
        }
        if seen.insert(word.clone()) {
            bad.push(word);
        }
    }
    bad
}

#[cfg(test)]
mod tests {
    use super::offending_words;

    fn vocab() -> Vec<String> {
        ["place", "metal1", "rectangle", "route", "wire", "cell"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    #[test]
    fn in_vocabulary_prompt_is_clean() {
        let bad = offending_words("Please place a metal1 rectangle at 10 20", &vocab());
        assert!(bad.is_empty(), "unexpected offenders: {bad:?}");
    }

    #[test]
    fn out_of_vocabulary_word_is_reported() {
        // "a" is in the stoplist and "metal1" is in the vocabulary, so only the
        // genuinely out-of-scope words are reported, in first-seen order.
        let bad = offending_words("write me a poem about metal1", &vocab());
        assert_eq!(
            bad,
            vec![
                "write".to_string(),
                "me".to_string(),
                "poem".to_string(),
                "about".to_string(),
            ]
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(offending_words("PLACE a RECTANGLE", &vocab()).is_empty());
    }

    #[test]
    fn offenders_are_deduplicated_in_order() {
        let bad = offending_words("hack hack the cell hack", &vocab());
        assert_eq!(bad, vec!["hack".to_string()]);
    }

    #[test]
    fn empty_vocabulary_permits_anything() {
        assert!(offending_words("arbitrary unrestricted text", &[]).is_empty());
    }

    #[test]
    fn bare_numbers_are_allowed() {
        assert!(offending_words("place cell 42 at 100 200", &vocab()).is_empty());
    }
}
