//! Drawing the palette's rows.
//!
//! The matching, the ranking and the context filter moved to
//! [`postio_ui::palette`] in #658, so the macOS palette ranks the same query
//! the same way rather than growing a second matcher — the drift ADR 0019
//! exists to prevent. This is what could not move: Pango markup.
//!
//! The names are re-exported so nothing in this crate had to change, and so
//! `crate::palette::score` still reads as it did in every comment that names
//! it.

pub use postio_ui::palette::{Entry, MAX_ROWS, Match, entries, score};

/// `title` as Pango markup, with the matched characters in bold.
///
/// Escaped as it goes: a command title is static text from the registry
/// today, but a folder name is not, and markup assembled from a name the
/// server chose is a hole waiting to be found.
pub fn highlight(title: &str, positions: &[usize]) -> String {
    let mut out = String::with_capacity(title.len() + positions.len() * 7);
    for (index, character) in title.char_indices() {
        let escaped = glib::markup_escape_text(&character.to_string());
        if positions.contains(&index) {
            out.push_str("<b>");
            out.push_str(&escaped);
            out.push_str("</b>");
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighting_marks_the_matched_characters_and_escapes_the_rest() {
        assert_eq!(highlight("Reply", &[0]), "<b>R</b>eply");
        assert_eq!(highlight("Move to…", &[]), "Move to…");
        assert_eq!(
            highlight("A & B", &[]),
            "A &amp; B",
            "a title is markup once it reaches a label"
        );
    }
}
