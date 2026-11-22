//! `fold` lowercases and strips diacritics: the index folds only case, so `cafe` would not find
//! `café`.
//! Both sides must go through it, the indexed column (`postio-storage`) and the query
//! (`postio-index`), or they stop meeting.

use unicode_normalization::UnicodeNormalization;

/// Lowercased, NFD-decomposed, combining marks stripped. The lowercasing duplicates the engine's
/// analyzer on purpose, so the folded column is self-describing.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        // Dropping combining marks after NFD turns `é` (e + U+0301) into `e`.
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
        // Folding twice must equal folding once: the query path folds an already-folded term.
        for word in ["café", "CAFÉ", "cafe", "straße", "ñ", "日本語"] {
            assert_eq!(fold(&fold(word)), fold(word), "folding {word} twice");
        }
    }

    #[test]
    fn scripts_without_accents_are_left_alone() {
        // Scripts with no combining marks must fold to themselves.
        assert_eq!(fold("日本語"), "日本語");
        assert_eq!(fold("Привет"), "привет");
    }

    #[test]
    fn a_word_with_no_accent_is_untouched() {
        assert_eq!(fold("plain"), "plain");
        assert_eq!(fold(""), "");
    }
}
