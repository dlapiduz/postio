//! Folding text for search.
//!
//! # Why the application does this
//!
//! FTS5 was asked for `unicode61 remove_diacritics 2` and folded inside its
//! own index. The engine's full-text index is tantivy's `SimpleTokenizer` plus
//! `LowerCaser`: it folds case and nothing else, and takes no tokenizer
//! option. So `CAFÉ` finds `café` and `cafe` does not, which is a search
//! result a person will read as a bug.
//!
//! [`fold`] is what makes up the difference, and the rule it has to obey is
//! that **both sides go through it or neither does**. The indexed column is
//! folded on the way in (`postio-storage` writes `messages.body_search`), the
//! query is folded on the way out (`postio-index`), and a change to one
//! without the other silently stops them meeting.
//!
//! # Here rather than in either caller
//!
//! Because both callers need it and neither is upstream of the other:
//! `postio-storage` writes the column, `postio-index` reads it, and the only
//! crate both depend on is this one. It also belongs here on the merits — a
//! fold is a property of the text, not of the store it happens to sit in, and
//! this crate is where text is already Postio's business.

use unicode_normalization::UnicodeNormalization;

/// `text`, folded so that an accented word and its unaccented spelling match.
///
/// Lowercased, decomposed to NFD, and stripped of combining marks. The
/// lowercasing duplicates what the engine's analyzer already does, which is
/// deliberate: it makes the folded column self-describing rather than correct
/// only in combination with something declared elsewhere.
///
/// ```
/// # use postio_model::fold::fold;
/// assert_eq!(fold("un CAFÉ très noir"), "un cafe tres noir");
/// ```
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        // `is_mark_nonspacing` is the combining-accent category. Dropping it
        // after NFD is what turns `é` (e + U+0301) into `e`.
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accent_folds_away_and_the_letter_stays() {
        assert_eq!(fold("café"), "cafe");
        assert_eq!(fold("très"), "tres");
        assert_eq!(fold("naïve"), "naive");
        assert_eq!(fold("Müller"), "muller");
    }

    #[test]
    fn case_folds_too() {
        assert_eq!(fold("CAFÉ"), "cafe");
        assert_eq!(fold("MiXeD"), "mixed");
    }

    #[test]
    fn the_fold_is_idempotent() {
        // The query path folds a term the write path already folded whenever
        // someone searches for exactly what is stored. Folding twice must not
        // differ from folding once, or the two paths stop meeting on their
        // second encounter.
        for word in ["café", "CAFÉ", "cafe", "straße", "ñ", "日本語"] {
            assert_eq!(fold(&fold(word)), fold(word), "folding {word} twice");
        }
    }

    #[test]
    fn scripts_without_accents_are_left_alone() {
        // Nothing here decomposes into a base plus a combining mark, so the
        // fold must be the identity on it -- a fold that mangled CJK would
        // make every such search miss.
        assert_eq!(fold("日本語"), "日本語");
        assert_eq!(fold("Привет"), "привет");
    }

    #[test]
    fn a_word_with_no_accent_is_untouched() {
        assert_eq!(fold("plain"), "plain");
        assert_eq!(fold(""), "");
    }
}
