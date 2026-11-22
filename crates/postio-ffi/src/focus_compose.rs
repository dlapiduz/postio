//! Writing and replying at the boundary. The controller decides which composer opens and when to
//! save; the draft's words stay the Mac's, saved with `save_draft` on `FocusSaveDraft`.

use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ComposerKindFfi {
    New,
    Reply,
    ReplyAll,
    Forward,
    Draft,
    /// An unsent reply shown in a thread: the event's `message` is the
    /// draft's own id, and `draft(id)` is what to show.
    ThreadDraft,
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
            Kind::ThreadDraft { .. } => ComposerKindFfi::ThreadDraft,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardFormFfi {
    This,
    Thread,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftEditFfi {
    ForwardThis,
    ForwardThread,
    ForwardAttachments,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChooserCardFfi {
    pub form: ForwardFormFfi,
    pub key: Option<String>,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChooserFfi {
    pub heading: String,
    pub cards: Vec<ChooserCardFfi>,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarActionFfi {
    pub label: String,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OlderReplyFfi {
    pub title: String,
    pub detail: String,
    pub actions: Vec<BarActionFfi>,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewerMessageFfi {
    pub id: i64,
    pub who: String,
    pub when: String,
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

    /// The composer's draft as `FocusComposerDraftWanted { edit }` asked, with what was typed. The
    /// controller answers with `FocusComposerDraft`.
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

    /// `kept` when the draft was written, `false` when nothing was worth keeping (or it was sent),
    /// `error` the sentence when it could not be saved.
    pub fn focus_draft_saved(&self, composition: u64, kept: bool, error: Option<String>) {
        let saved = match error {
            Some(sentence) => Err(sentence),
            None => Ok(kept),
        };
        self.focus_driver()
            .input(postio_focus::Input::DraftSaved { composition, saved });
    }
}
