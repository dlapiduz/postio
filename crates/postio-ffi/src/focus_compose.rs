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
            Kind::Forward { .. } => ComposerKindFfi::Forward,
            Kind::Draft => ComposerKindFfi::Draft,
            _ => return None,
        })
    }
}

/// What a forward carries (spec 013 FR-023): a chooser card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardFormFfi {
    /// The one message, quoted as usual.
    This,
    /// Every message, oldest first, in one.
    Thread,
    /// Each message as an `.eml` attachment.
    Attachments,
}

impl From<postio_model::ForwardForm> for ForwardFormFfi {
    fn from(form: postio_model::ForwardForm) -> Self {
        match form {
            postio_model::ForwardForm::Thread => ForwardFormFfi::Thread,
            postio_model::ForwardForm::Attachments => ForwardFormFfi::Attachments,
            _ => ForwardFormFfi::This,
        }
    }
}

impl From<ForwardFormFfi> for postio_model::ForwardForm {
    fn from(form: ForwardFormFfi) -> Self {
        match form {
            ForwardFormFfi::This => postio_model::ForwardForm::This,
            ForwardFormFfi::Thread => postio_model::ForwardForm::Thread,
            ForwardFormFfi::Attachments => postio_model::ForwardForm::Attachments,
        }
    }
}

/// A change the engine makes to the composer's draft, which the composer
/// is asked for first so what was typed is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftEditFfi {
    /// The forward carries `this` form.
    ForwardThis,
    /// The forward carries the whole thread.
    ForwardThread,
    /// The forward carries each message as an attachment.
    ForwardAttachments,
    /// The reply answers the thread's latest received message.
    ReplyToLatest,
}

impl DraftEditFfi {
    pub(crate) fn of(edit: postio_focus::DraftEdit) -> Option<Self> {
        use postio_focus::DraftEdit as Edit;
        Some(match edit {
            Edit::ForwardForm(postio_model::ForwardForm::Thread) => DraftEditFfi::ForwardThread,
            Edit::ForwardForm(postio_model::ForwardForm::Attachments) => {
                DraftEditFfi::ForwardAttachments
            }
            Edit::ForwardForm(_) => DraftEditFfi::ForwardThis,
            Edit::ReplyToLatest => DraftEditFfi::ReplyToLatest,
            _ => return None,
        })
    }

    fn into_edit(self) -> postio_focus::DraftEdit {
        use postio_focus::DraftEdit as Edit;
        use postio_model::ForwardForm as Form;
        match self {
            DraftEditFfi::ForwardThis => Edit::ForwardForm(Form::This),
            DraftEditFfi::ForwardThread => Edit::ForwardForm(Form::Thread),
            DraftEditFfi::ForwardAttachments => Edit::ForwardForm(Form::Attachments),
            DraftEditFfi::ReplyToLatest => Edit::ReplyToLatest,
        }
    }
}

/// One card of the *What to forward* chooser.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChooserCardFfi {
    /// What choosing it does.
    pub form: ForwardFormFfi,
    /// Its number key, as the keymap spells it.
    pub key: Option<String>,
    /// "This message".
    pub title: String,
    /// One line under it.
    pub detail: String,
}

/// The chooser above To when a thread of two or more is forwarded.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChooserFfi {
    /// "What to forward".
    pub heading: String,
    /// The three cards, in key order.
    pub cards: Vec<ChooserCardFfi>,
    /// The one chosen: its card is ringed.
    pub selected: ForwardFormFfi,
}

impl From<postio_ui::compose::ChooserView> for ChooserFfi {
    fn from(view: postio_ui::compose::ChooserView) -> Self {
        ChooserFfi {
            heading: view.heading,
            cards: view
                .cards
                .into_iter()
                .map(|card| ChooserCardFfi {
                    form: card.form.into(),
                    key: card.key,
                    title: card.title,
                    detail: card.detail,
                })
                .collect(),
            selected: view.selected.into(),
        }
    }
}

/// An action on the older-reply bar.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarActionFfi {
    /// "Show them", "Reply to latest".
    pub label: String,
    /// Its key, as the keymap spells it.
    pub key: Option<String>,
}

/// The bar under the composer's title on a reply to a message that is not
/// the latest.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OlderReplyFfi {
    /// "You're replying to message 4 of 6, not the latest".
    pub title: String,
    /// Which newer messages there are, and what the reply quotes.
    pub detail: String,
    /// Show them (or Hide them), then Reply to latest.
    pub actions: Vec<BarActionFfi>,
    /// Whether the newer messages are on show.
    pub newer_shown: bool,
}

impl From<postio_ui::compose::OlderReplyView> for OlderReplyFfi {
    fn from(view: postio_ui::compose::OlderReplyView) -> Self {
        OlderReplyFfi {
            title: view.title,
            detail: view.detail,
            actions: view
                .actions
                .into_iter()
                .map(|action| BarActionFfi {
                    label: action.label,
                    key: action.key,
                })
                .collect(),
            newer_shown: view.newer_shown,
        }
    }
}

/// A message newer than the one a reply answers, shown read-only in the
/// composer (spec 013 FR-022, "Show them").
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewerMessageFfi {
    /// The message.
    pub id: i64,
    /// Who wrote it: their name or address.
    pub who: String,
    /// When: "Tue 22 Sep 15:22".
    pub when: String,
    /// What it says, as text.
    pub text: String,
}

#[uniffi::export]
impl Session {
    /// The messages `ids` as the read-only block under the composer's bar:
    /// in the order given, the ones this machine has bodies for read from
    /// the store and nothing fetched.
    pub fn focus_newer_messages(&self, ids: Vec<i64>) -> Vec<NewerMessageFfi> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Vec::new();
        };
        crate::session::blocking(async move {
            let Ok(connection) = database.connect().await else {
                return Vec::new();
            };
            let mut shown = Vec::with_capacity(ids.len());
            for id in ids {
                let Ok(Some(mut message)) =
                    postio_storage::repository::MessageRepository::new(&connection)
                        .get(postio_model::MessageId::new(id))
                        .await
                else {
                    continue;
                };
                if let postio_session::reading::Body::Ready { body, .. } =
                    postio_session::reading::load_body_or_reason(&connection, message.id, false)
                        .await
                {
                    message.body = body;
                }
                let text = match (message.body.text.as_deref(), message.body.html.as_deref()) {
                    (Some(text), _) if !text.trim().is_empty() => text.to_owned(),
                    (_, Some(html)) => postio_body::render(&postio_body::parse(html)).0,
                    _ => message.preview.clone().unwrap_or_default(),
                };
                shown.push(NewerMessageFfi {
                    id,
                    who: message
                        .primary_from()
                        .map_or_else(|| "Someone".to_owned(), |from| from.display().to_owned()),
                    when: message
                        .best_date()
                        .with_timezone(&chrono::Local)
                        .format("%a %-d %b %H:%M")
                        .to_string(),
                    text,
                });
            }
            shown
        })
    }

    /// The composer's draft, as `FocusComposerDraftWanted { edit }` asked
    /// for it, with what was typed in it. The controller has the engine
    /// change it and answers with `FocusComposerDraft`.
    pub fn focus_composer_draft(&self, edit: DraftEditFfi, draft: crate::DraftFfi) {
        let Some((database, _)) = self.store_and_blobs() else {
            return;
        };
        let Some(draft) = crate::session::blocking(self.rehydrate(&database, &draft)) else {
            return;
        };
        self.focus_driver()
            .input(postio_focus::Input::ComposerDraft(
                edit.into_edit(),
                Box::new(draft),
            ));
    }

    /// A chooser card was clicked: as its number key.
    pub fn focus_forward_form_clicked(&self, form: ForwardFormFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ForwardFormClicked(form.into()));
    }

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
