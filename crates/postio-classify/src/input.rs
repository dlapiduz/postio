//! What the classifier is handed: the message at filing, then again with its
//! body's own text (`contracts/engine.md`, "The filing pass").

use postio_model::Identity;

use crate::outcome::Span;

/// A message as filing knows it, before any body: the envelope,
/// `References` and `List-Id`, the flags (`$Junk` among them), what the
/// promoted headers said (research R8), its structure, its folder's role and
/// its conversation. The model's one type: sync hands the classifier exactly
/// what it filed, with nothing copied in between (tasks.md T102).
pub use postio_model::filing::FiledMessage;

/// A message whose body has arrived: what the body stage classifies.
#[derive(Debug, Clone, Copy)]
pub struct BodyMessage<'a> {
    /// Everything that was known at filing.
    pub filed: FiledMessage<'a>,
    /// The user's identities on the message's account: the addresses mail
    /// reaches them at, and the names they go by. They tell mail sent to the
    /// user from mail they are only copied on, or sent themselves, and an ask
    /// put to them by name from one put to somebody else (research R10).
    /// With none, nothing is marked: nothing says who "you" is.
    pub identities: &'a [Identity],
}

/// The newest message's own text: the body without quoted history or
/// signature, which is all a marker may quote (FR-104). A marker's span is
/// character offsets into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnText<'a> {
    text: &'a str,
}

impl<'a> OwnText<'a> {
    /// `text`, already cut down to the newest message's own words.
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// The text itself.
    pub fn as_str(&self) -> &'a str {
        self.text
    }

    /// How many characters it has: the bound every span must keep within.
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// The sentence `span` quotes, as a marker stores it: the text's own
    /// characters, verbatim, and at most [`EXCERPT_CHARS`] of them -- a
    /// plain prefix when the sentence is longer, with no ellipsis or
    /// anything else added, since the reader finds the sentence again by
    /// its excerpt (research R2). A span past the end is cut at the end.
    pub fn excerpt(&self, span: &Span) -> String {
        self.text
            .chars()
            .skip(span.start)
            .take(span.end.saturating_sub(span.start).min(EXCERPT_CHARS))
            .collect()
    }
}

/// The most characters a marker's excerpt holds (data-model.md, `markers`).
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
        // R2: the locator finds a sentence again by its excerpt, so the
        // excerpt is the sentence's own first 200 characters and nothing
        // added: no ellipsis, no trimmed word.
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
