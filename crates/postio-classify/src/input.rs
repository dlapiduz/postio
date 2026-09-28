//! What the classifier is handed: the message at filing, then again with its
//! body's own text (`contracts/engine.md`, "The filing pass").

use postio_model::Identity;

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
}
