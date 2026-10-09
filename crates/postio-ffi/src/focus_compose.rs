//! Writing and replying at the boundary (specs/009-focus-macos T073).
//!
//! The rules are the controller's (`postio_focus`'s compose slice): which
//! composer opens, what it answers, when what is written is saved, and
//! what is said once a composition ends. The draft's words stay the
//! Mac's -- its fields and its editing surface hold them, and it saves
//! them with `save_draft` when `FocusSaveDraft` says -- so what crosses
//! here is the composer opening (`FocusComposer`), the edits that arm the
//! autosave (`focus_composer_edited`), and how each save went
//! (`focus_draft_saved`).

use crate::session::Session;

/// What the composer is asked to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ComposerKindFfi {
    /// A new message: `newDraft()`.
    New,
    /// A reply to the message's sender: `replyDraft(message, false)`.
    Reply,
    /// A reply to everyone on it: `replyDraft(message, true)`.
    ReplyAll,
    /// A forward: `forwardDraft(message)`.
    Forward,
    /// The draft behind the message, to go on writing:
    /// `draftForMessage(message)`.
    Draft,
}

impl ComposerKindFfi {
    /// The controller's kind, as the boundary names it; `None` for one this
    /// build does not write.
    pub(crate) fn of(kind: postio_focus::ComposerKind) -> Option<Self> {
        use postio_focus::ComposerKind as Kind;
        Some(match kind {
            Kind::New => ComposerKindFfi::New,
            Kind::Reply => ComposerKindFfi::Reply,
            Kind::ReplyAll => ComposerKindFfi::ReplyAll,
            Kind::Forward => ComposerKindFfi::Forward,
            Kind::Draft => ComposerKindFfi::Draft,
            _ => return None,
        })
    }
}

#[uniffi::export]
impl Session {
    /// Something was written in the composer: a recipient, the subject, a
    /// word of the body. Say it on every edit; the controller waits out the
    /// quiet period and says `FocusSaveDraft` once.
    pub fn focus_composer_edited(&self) {
        self.focus_driver()
            .input(postio_focus::Input::ComposerEdited);
    }

    /// How the save `FocusSaveDraft { composition }` asked for went: `kept`
    /// when the draft was written to the store, `false` when there was
    /// nothing in it worth keeping (or it was sent), `error` the sentence
    /// when it could not be saved.
    pub fn focus_draft_saved(&self, composition: u64, kept: bool, error: Option<String>) {
        let saved = match error {
            Some(sentence) => Err(sentence),
            None => Ok(kept),
        };
        self.focus_driver()
            .input(postio_focus::Input::DraftSaved { composition, saved });
    }
}
