//! What the classifier is handed: the message at filing, then again with its
//! body's own text (`contracts/engine.md`, "The filing pass").

use postio_model::{MailboxRole, Message};

/// A message as the filing pass knows it, before any body: the envelope,
/// `References` and `List-Id`, the flags (`$Junk` among them), and what the
/// promoted headers said (research R8).
#[derive(Debug, Clone, Copy)]
pub struct FiledMessage<'a> {
    /// The message as filed: envelope, addresses, flags, thread.
    pub message: &'a Message,
    /// The role of the mailbox it was filed into, when it has one.
    pub mailbox: Option<MailboxRole>,
    /// Whether it carries `List-Unsubscribe` (`messages.unsubscribe_offered`),
    /// or `None` while that is not known.
    pub unsubscribe_offered: Option<bool>,
    /// `messages.automation`: 1 `Precedence: bulk`, 2 `list`, 4 `junk`, 8
    /// `Auto-Submitted: auto-generated`, 16 `auto-replied`; `None` while not
    /// known.
    pub automation: Option<u8>,
    /// Its structure holds a `text/calendar` part.
    pub has_calendar: bool,
}

/// A message whose body has arrived: what the body stage classifies.
#[derive(Debug, Clone, Copy)]
pub struct BodyMessage<'a> {
    /// Everything that was known at filing.
    pub filed: FiledMessage<'a>,
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
