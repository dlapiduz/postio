use postio_model::Identity;

use crate::outcome::Span;

pub use postio_model::filing::FiledMessage;

#[derive(Debug, Clone, Copy)]
pub struct BodyMessage<'a> {
    pub filed: FiledMessage<'a>,
    /// Tells mail sent to the user from mail they are only copied on or sent themselves. With none,
    /// nothing is marked.
    pub identities: &'a [Identity],
}

/// All a marker may quote; a marker's span is character offsets into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnText<'a> {
    text: &'a str,
}

impl<'a> OwnText<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    pub fn as_str(&self) -> &'a str {
        self.text
    }

    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// At most [`EXCERPT_CHARS`] of them, a plain prefix with no ellipsis added, since the reader
    /// finds
    /// the sentence again by its excerpt. A span past the end is cut at the end.
    pub fn excerpt(&self, span: &Span) -> String {
        self.text
            .chars()
            .skip(span.start)
            .take(span.end.saturating_sub(span.start).min(EXCERPT_CHARS))
            .collect()
    }
}

pub const EXCERPT_CHARS: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_excerpt_is_the_sentence_as_written() {
        let text = OwnText::new("Hi Ada,\n\nCan you approve these by Friday?\n\nThanks");
        let start = "Hi Ada,\n\n".chars().count();
        let span = start..start + "Can you approve these by Friday?".chars().count();

        assert_eq!(text.excerpt(&span), "Can you approve these by Friday?");
    }

    #[test]
    fn a_long_sentence_is_cut_to_a_plain_prefix_with_no_ellipsis() {
        // The excerpt is the sentence's first 200 characters with nothing added: the reader finds
        // it by that.
        let sentence = format!("Could you {}?", "é".repeat(300));
        let text = OwnText::new(&sentence);
        let excerpt = text.excerpt(&(0..sentence.chars().count()));

        assert_eq!(excerpt.chars().count(), EXCERPT_CHARS);
        assert!(sentence.starts_with(&excerpt));
        assert!(!excerpt.ends_with('…') && !excerpt.ends_with("..."));
    }

    #[test]
    fn a_span_past_the_end_is_cut_at_the_end() {
        let text = OwnText::new("Short.");

        assert_eq!(text.excerpt(&(0..400)), "Short.");
        assert_eq!(text.excerpt(&(10..20)), "");
    }
}
