//! Writing mail: a draft crosses, not an editor (#1272).
//! Addresses cross as text, as typed (`Ada Norwood <ada@example.com>`); `postio_model::address`
//! parses them.

use postio_model::address;
use postio_model::draft::{Draft, DraftKind};
use postio_model::message::MessageBody;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftKindFfi {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl From<DraftKind> for DraftKindFfi {
    fn from(kind: DraftKind) -> Self {
        match kind {
            DraftKind::New => Self::New,
            DraftKind::Reply => Self::Reply,
            DraftKind::ReplyAll => Self::ReplyAll,
            DraftKind::Forward => Self::Forward,
        }
    }
}

impl From<DraftKindFfi> for DraftKind {
    fn from(kind: DraftKindFfi) -> Self {
        match kind {
            DraftKindFfi::New => Self::New,
            DraftKindFfi::Reply => Self::Reply,
            DraftKindFfi::ReplyAll => Self::ReplyAll,
            DraftKindFfi::Forward => Self::Forward,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftFfi {
    /// The row id, or `0` before the first save; `save` is idempotent on it, so it must come back.
    pub id: i64,
    pub account: i64,
    pub kind: DraftKindFfi,
    /// The address it comes from, rendered — `Mara Ostwald <mara@…>`. Read-only for now.
    pub from: String,
    /// `To`, as typed: `Ada Norwood <ada@example.com>, bo@example.com`.
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    /// For a rich draft, the `text/plain` alternative derived from the document: rich mail always
    /// sends both parts.
    pub body: String,
    /// Canonical, not the editor's DOM: what `postio_body::parse` makes of the working copy (ADR
    /// 0004 Q3).
    /// `None` for a plain draft; kept when the switch goes off.
    pub body_html: Option<String>,
    /// Decides what leaves: `text/html` plus a `text/plain` fallback, or `text/plain` alone.
    pub rich: bool,
    pub in_reply_to: Option<i64>,
    /// The message a forward was made from; the forward chooser rebuilds the draft from it.
    #[uniffi(default = None)]
    pub forwarded_from: Option<i64>,
    pub path: String,
    pub attachments: Vec<AttachmentFfi>,
    /// "Remind if no reply": epoch milliseconds, `None` for none.
    #[uniffi(default = None)]
    pub remind_at: Option<i64>,
    /// The labels sending applies to its conversation; a reply starts with its thread's.
    #[uniffi(default = [])]
    pub labels: Vec<DraftLabelFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftLabelFfi {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentFfi {
    pub id: i64,
    pub filename: String,
    pub mime_type: String,
    /// How big, said the way a person thinks about it — `1.8 MB`.
    pub size: String,
}

/// A picture put into a draft's body (#1571): the draft with the part on it, and the script that
/// draws the picture at the caret.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InlineImageFfi {
    pub draft: DraftFfi,
    pub script: String,
}

/// What will be sent, in the composer footer's words: `html + text/plain`, or `text/plain,
/// format=flowed`.
/// A free function: `#[uniffi::method]` in a `Record` impl compiles but generates nothing in Swift.
#[uniffi::export]
pub fn outgoing_shape(rich: bool) -> String {
    postio_ui::compose::outgoing_shape(rich)
}

pub(crate) fn to_ffi(draft: &Draft, from: String, path: String) -> DraftFfi {
    DraftFfi {
        id: draft.id.get(),
        account: draft.account_id.into(),
        kind: draft.kind.into(),
        from,
        to: render(&draft.to),
        cc: render(&draft.cc),
        bcc: render(&draft.bcc),
        subject: draft.subject.clone(),
        body: draft.body.text.clone().unwrap_or_default(),
        // The editing shell's form: stored `cid:` is `postio-cid:` to the surface, whose handler
        // answers
        // that scheme (#1571).
        body_html: draft
            .body
            .html
            .as_deref()
            .map(|html| postio_body::parse(html).editor_html()),
        // The stored flag, not `html.is_some()`: a plain draft may keep its marks (#1271).
        rich: draft.rich,
        in_reply_to: draft.in_reply_to.map(|id| id.get()),
        forwarded_from: draft.forwarded_from.map(|id| id.get()),
        path,
        attachments: draft
            .attachments
            .iter()
            .map(|attachment| AttachmentFfi {
                id: attachment.id.get(),
                filename: attachment
                    .filename
                    .clone()
                    .unwrap_or_else(|| "(unnamed)".to_owned()),
                mime_type: attachment.mime_type.clone(),
                size: postio_ui::format::human_size(attachment.size),
            })
            .collect(),
        remind_at: draft.remind_at.map(|at| at.timestamp_millis()),
        labels: draft
            .labels
            .iter()
            .map(|id| DraftLabelFfi {
                id: id.get(),
                name: String::new(),
            })
            .collect(),
    }
}

/// `None` until more than one person is on the message.
/// A free function: arithmetic over the composer's unsaved fields.
#[uniffi::export]
pub fn recipient_summary(draft: DraftFfi) -> Option<String> {
    postio_ui::compose::recipient_summary(
        parse(&draft.to).len(),
        parse(&draft.cc).len(),
        parse(&draft.bcc).len(),
    )
}

/// `base` carries what the frontend does not edit (kind, ancestor, reserved `Message-ID`); the
/// account is the one field it may change.
pub(crate) fn from_ffi(base: Draft, edited: &DraftFfi) -> Draft {
    let mut draft = base;
    // Another account moves the draft to it, and its identity goes with the account.
    let account = postio_model::AccountId::new(edited.account);
    if edited.account > 0 && draft.account_id != account {
        draft.account_id = account;
        draft.identity_id = None;
    }
    draft.to = parse(&edited.to);
    draft.cc = parse(&edited.cc);
    draft.bcc = parse(&edited.bcc);
    draft.subject = edited.subject.clone();
    draft.body = body_of(edited);
    draft.rich = edited.rich;
    draft.remind_at = edited
        .remind_at
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);
    draft.labels = edited
        .labels
        .iter()
        .map(|label| postio_model::LabelId::new(label.id))
        .collect();
    draft
}

/// The body a draft is stored with. A rich body is narrowed to the `Document` dialect and rendered
/// back out, never kept as the editor's DOM (ADR 0004 Q3); its `text/plain` is derived from the
/// same document, never taken from `edited.body`.
fn body_of(edited: &DraftFfi) -> MessageBody {
    let marks = edited.body_html.as_deref().unwrap_or_default();

    if !edited.rich {
        // The plain field wins unless it is empty and there are marks, which the Rich/Plain switch
        // leaves behind (#1293); otherwise an empty message goes out.
        let text = if edited.body.is_empty() && !marks.is_empty() {
            postio_body::render(&postio_body::parse(marks)).0
        } else {
            edited.body.clone()
        };
        return MessageBody {
            text: Some(text),
            // Kept: the switch is on the document, and `send_draft` stops the marks reaching the
            // wire (#1271).
            html: edited.body_html.clone(),
        };
    }

    // Rich with no document yet: built from the plain field (`Session::rich_html_of`), or its words
    // are lost.
    let document = if marks.is_empty() {
        postio_body::Document::from_text_quoting(&edited.body)
    } else {
        postio_body::parse(marks)
    };
    let (text, html) = postio_body::render(&document);
    MessageBody {
        text: Some(text),
        html: Some(html),
    }
}

/// `Ada Norwood <ada@example.com>, bo@example.com`.
fn render(addresses: &[postio_model::EmailAddress]) -> String {
    addresses
        .iter()
        .map(|address| address.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse(text: &str) -> Vec<postio_model::EmailAddress> {
    address::parse_list(text)
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ComposeError {
    #[error("{message}")]
    Refused { message: String },
}

/// The markup narrowed to the dialect, and the sentence (`postio_body::Lost::summary`) saying what
/// it could not hold.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PastedFfi {
    pub html: String,
    pub text: String,
    /// What was lost, as one sentence; `None` when nothing was, so a paste is silent unless it cost
    /// something.
    pub dropped: Option<String>,
}

/// `when` is epoch milliseconds, which goes back into `Session::sendDraftLater`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SchedulePresetFfi {
    pub label: String,
    /// When that is, as epoch milliseconds.
    pub when: i64,
}

/// Recomputed on each open so the times are never stale; the rule is
/// `postio_ui::compose::schedule_presets`'s.
#[uniffi::export]
pub fn schedule_presets() -> Vec<SchedulePresetFfi> {
    postio_ui::compose::schedule_presets(chrono::Local::now())
        .into_iter()
        .map(|preset| SchedulePresetFfi {
            label: preset.label.to_owned(),
            when: preset.when.timestamp_millis(),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuoteFoldFfi {
    pub written: String,
    pub quote: String,
    /// What the fold says: "On 2026-09-26, Lena Park wrote · 18 quoted
    /// lines".
    pub summary: String,
}

/// `None` when `body` ends in no quote.
#[uniffi::export]
pub fn fold_quote(body: String) -> Option<QuoteFoldFfi> {
    postio_ui::compose::fold_quote(&body).map(|fold| QuoteFoldFfi {
        written: fold.written,
        quote: fold.quote,
        summary: fold.summary,
    })
}

/// What the composer's title says it is: "New message", "Reply to all".
#[uniffi::export]
pub fn composer_title(kind: DraftKindFfi) -> String {
    postio_ui::compose::title(kind.into()).to_owned()
}

/// What will be sent, counted: "Plain text · 58 words".
#[uniffi::export]
pub fn draft_summary(draft: DraftFfi) -> String {
    let base = Draft::new(postio_model::AccountId::new(draft.account));
    let mut built = from_ffi(base, &draft);
    // What will be sent: a draft turned Plain keeps its marks for the way
    // back, and `send_draft` drops them, so the footer must too.
    if !built.rich {
        built.body.html = None;
    }
    postio_ui::compose::summary(&built)
}

/// "Draft saved locally 16:12", for a save that landed at `at` (epoch
/// milliseconds), in the local zone.
#[uniffi::export]
pub fn draft_saved_words(at: i64) -> String {
    let at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at).unwrap_or_default();
    postio_ui::compose::saved_at(at)
}

/// What the reminder verb says in full: "Remind if no reply", and the day
/// once one is chosen ("Remind if no reply · Tue 29 Sep"); `at` in epoch
/// milliseconds.
#[uniffi::export]
pub fn remind_meaning(at: Option<i64>) -> String {
    postio_ui::compose::remind_meaning(
        at.and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis),
    )
}

#[uniffi::export]
pub fn remind_presets() -> Vec<SchedulePresetFfi> {
    postio_ui::schedule::remind_presets(postio_ui::clock::now())
        .into_iter()
        .map(|(label, when)| SchedulePresetFfi {
            label: label.to_owned(),
            when: when.timestamp_millis(),
        })
        .collect()
}
