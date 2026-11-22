use postio_model::EmailAddress;

use crate::digests::Digests;
use crate::outcome::MarkerKind;
use crate::senders::Senders;

/// Everything the classifier decides by, as data rather than code (constitution VII): the
/// automated-senders table, the user's digest rules, and the marker kinds the user stopped.
/// The needs-action detector's own rules are not here: they are words, plain code with a small
/// compiled-in table.
pub trait Rules {
    /// [`Senders::shipped`] in production. The needs-action question is not asked of mail from a
    /// sender in it.
    fn senders(&self) -> &Senders;

    /// In the file's order: the first that matches holds the message. None unless the rules say
    /// otherwise.
    fn digests(&self) -> &Digests {
        Digests::none()
    }

    /// The person's correction, made by dismissing them as wrong again and again (`[focus.filter]
    /// stop_markers`); a detector's answer has to accept it. Nothing is stopped unless the rules
    /// say otherwise.
    fn stops(&self, _sender: &EmailAddress, _kind: MarkerKind) -> bool {
        false
    }
}
