//! Focus's reader document on the Mac (specs/009-focus-macos T062, T063).
//!
//! The same treated document GTK's Focus reader draws -- sanitised,
//! classified, drawn in app colours or on paper -- with the two things a
//! WKWebView cannot do for itself done here: the contrast guard on kept
//! colours (`postio_body::treatment::guard_kept_colours`), and the
//! geometry of the Mac's message window (M1). Swift hands the HTML to a
//! hardened web view and draws the render-mode line from the words.

use postio_body::treatment::Treatment;
use postio_ui::reader::document::{
    Absent, Sheet, absent_html, body_html_treated, decode_caveat, document_for_treated,
    render_mode_words, wrap_document,
};

use crate::Session;

/// A body's treatment (specs/007-postio-focus T210).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TreatmentFfi {
    /// The app's colours and fonts; the sender's taken away.
    AppColours,
    /// The sender's own layout, on a sheet of paper.
    Paper,
}

impl From<TreatmentFfi> for Treatment {
    fn from(treatment: TreatmentFfi) -> Self {
        match treatment {
            TreatmentFfi::AppColours => Treatment::AppColours,
            TreatmentFfi::Paper => Treatment::Paper,
        }
    }
}

impl From<Treatment> for TreatmentFfi {
    fn from(treatment: Treatment) -> Self {
        match treatment {
            Treatment::AppColours => TreatmentFfi::AppColours,
            Treatment::Paper => TreatmentFfi::Paper,
        }
    }
}

/// The render-mode line's words: "App colours · sender colours and fonts
/// removed · Show original O".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RenderModeWordsFfi {
    /// The treatment, named.
    pub title: String,
    /// Why, quietly.
    pub detail: String,
    /// The switch to the other treatment, beside its key.
    pub action: String,
    /// Whether to offer "Always for this sender".
    pub offer_always: bool,
    /// That button's words, while it is offered.
    pub always: Option<String>,
}

/// One message as Focus's message window draws it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FocusReaderDocumentFfi {
    /// The whole document, for a hardened web view. Never empty: a missing
    /// body is a state plate.
    pub html: String,
    /// What a blocked render held back, or `None`.
    pub notice: Option<crate::ReaderNoticeFfi>,
    /// What to say about a body that lost something in decoding.
    pub caveat: Option<String>,
    /// The treatment it is drawn in.
    pub treatment_shown: TreatmentFfi,
    /// What the rule chose, before any choice the person made.
    pub treatment_classified: TreatmentFfi,
    /// The render-mode line, for an HTML body; `None` for plain text, which
    /// has no other treatment.
    pub render_mode: Option<RenderModeWordsFfi>,
    /// The sender, as "Always for this sender" names them.
    pub sender: Option<String>,
    /// The treatment remembered for the sender, if any.
    pub sender_choice: Option<TreatmentFfi>,
    /// The message window's width beside the main window (M1).
    pub window_width: i32,
    /// The content column in it, for the treatment shown.
    pub column_width: i32,
    /// Whether Label, Move and Delete fold into More at that width.
    pub folds_into_more: bool,
    /// The least a paper body is scaled to fit its column.
    pub paper_floor: f64,
}

#[uniffi::export]
impl Session {
    /// `message` as Focus's message window draws it, beside a main window
    /// `main_width` points wide: in `chosen` when the person switched it
    /// (`O`), else as the sender's remembered choice or the rule says.
    pub fn focus_reader_document(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        chosen: Option<TreatmentFfi>,
        main_width: i32,
    ) -> FocusReaderDocumentFfi {
        crate::session::blocking(self.focus_reader_answers(message, remote, chosen, main_width))
    }

    /// `message`'s raw RFC 822 source, every byte as the server sent it:
    /// what `v` shows in place of the content (M4). Read from this machine
    /// when it is here; otherwise fetched from the server on this call, the
    /// person having asked for these bytes by name -- so never from the main
    /// actor.
    pub fn raw_source(&self, message: i64) -> Result<Vec<u8>, crate::SessionError> {
        let client = self
            .client()
            .ok_or_else(|| crate::SessionError::StoreUnavailable {
                message: "The store is closed.".to_owned(),
            })?;
        crate::session::blocking(client.raw_source(message.into())).map_err(|error| {
            crate::SessionError::StoreUnavailable {
                message: error.to_string(),
            }
        })
    }

    /// One of the faces the document names over `postio-font:`, by its
    /// name, or `None` for any name that is not one of them. Compiled in,
    /// so no path and no network is ever involved: what a web view's
    /// `postio-font:` handler answers from (ADR 0023).
    pub fn reader_font(&self, name: String) -> Option<Vec<u8>> {
        postio_ui::reader::document::font_bytes(&name).map(<[u8]>::to_vec)
    }

    /// Always draw `sender`'s mail in `treatment`, or forget the choice with
    /// `None`: "Always for this sender". Saved beside the remote-image
    /// grants, where GTK's reader keeps it.
    pub fn always_treatment(&self, sender: String, treatment: Option<TreatmentFfi>) {
        let path = self.allow_list_path();
        let mut list = postio_ui::allowlist::RemoteImageAllowList::load_from(&path);
        list.set_treatment(&sender, treatment.map(Into::into));
        if let Err(error) = list.save_to(&path) {
            tracing::error!(%error, "a sender's treatment could not be saved");
        }
    }
}

impl Session {
    async fn focus_reader_answers(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        chosen: Option<TreatmentFfi>,
        main_width: i32,
    ) -> FocusReaderDocumentFfi {
        let geometry =
            postio_ui::focus_dialog::Geometry::for_platform(postio_config::paths::Platform::Apple);
        let window_width = geometry.dialog_width(main_width);
        let answer = |html: String, treated: Option<postio_ui::reader::document::Treated>| {
            let shown = treated.map_or(Treatment::AppColours, |treated| treated.shown);
            FocusReaderDocumentFfi {
                html,
                notice: None,
                caveat: None,
                treatment_shown: shown.into(),
                treatment_classified: treated
                    .map_or(Treatment::AppColours, |treated| treated.classified)
                    .into(),
                render_mode: None,
                sender: None,
                sender_choice: None,
                window_width,
                column_width: geometry.column_width(window_width, shown),
                folds_into_more: geometry.folds_into_more(window_width),
                paper_floor: postio_body::treatment::PAPER_FIT_FLOOR,
            }
        };
        let plate = |absent: Absent| {
            answer(
                wrap_document(
                    &absent_html(absent),
                    postio_body::RemoteImages::Blocked,
                    Sheet::Theme,
                ),
                None,
            )
        };
        let remote = postio_body::RemoteImages::from(remote);
        let Some((database, _blobs)) = self.store_and_blobs() else {
            return plate(Absent::Missing);
        };
        let Ok(connection) = database.connect().await else {
            return plate(Absent::Missing);
        };
        let offline = self.is_offline();
        let (body, encoding_problems) = match postio_session::reading::load_body_or_reason(
            &connection,
            message.into(),
            offline,
        )
        .await
        {
            postio_session::reading::Body::Ready {
                body,
                encoding_problems,
            } => (body, encoding_problems),
            postio_session::reading::Body::Absent(state) => return plate(state),
        };
        let sender = postio_storage::repository::MessageRepository::new(&connection)
            .get(postio_model::ids::MessageId::new(message))
            .await
            .ok()
            .flatten()
            .and_then(|row| {
                row.from
                    .first()
                    .map(|address| address.address.to_lowercase())
            });
        let remembered = sender.as_deref().and_then(|sender| {
            postio_ui::allowlist::RemoteImageAllowList::load_from(&self.allow_list_path())
                .treatment_for(sender)
        });
        let rendered =
            body_html_treated(&body, remote, chosen.map(Into::into).or(remembered), None);
        let treated = rendered.treated;
        let shown = treated.map_or(Treatment::AppColours, |treated| treated.shown);
        // The guard the renderer runs on Linux, run on the markup: the Mac's
        // web view keeps a sender's colour only where it reads.
        let html = match shown {
            Treatment::AppColours => postio_body::treatment::guard_kept_colours(
                &rendered.html,
                &postio_body::treatment::SURFACES,
            ),
            Treatment::Paper => rendered.html.clone(),
        };
        let notice = self
            .held_back_notice(&connection, message, rendered.held_back, remote)
            .await;
        let document = flow(
            &document_for_treated(&html, &rendered.styles, remote, shown),
            shown,
        );
        FocusReaderDocumentFfi {
            notice,
            caveat: decode_caveat(encoding_problems).map(str::to_owned),
            render_mode: treated
                .and_then(|treated| render_mode_words(treated, remembered))
                .map(|words| RenderModeWordsFfi {
                    title: words.title.to_owned(),
                    detail: words.detail.to_owned(),
                    action: words.action.to_owned(),
                    offer_always: words.offer_always,
                    always: words
                        .offer_always
                        .then(|| postio_ui::reader::document::ALWAYS_FOR_SENDER.to_owned()),
                }),
            sender_choice: remembered.map(Into::into),
            sender,
            ..answer(document, treated)
        }
    }
}

/// The reader palette in the Mac's own semantic colours, which WebKit
/// resolves against the web view's appearance: the column's ground is the
/// window's, the ink and hairlines are the platform's, and the accent is
/// whatever the person chose (the Mac pack's section 6). Paper forces the
/// light appearance on its web view, so its sheet reads these as light.
const MAC_FLOW_PALETTE: &str = ":root { --flow-ground: transparent; \
    --r-ink: -apple-system-label; --r-ink-secondary: -apple-system-secondary-label; \
    --r-dim: -apple-system-tertiary-label; --r-accent: -apple-system-control-accent; \
    --r-hairline: -apple-system-separator; --r-hairline-strong: -apple-system-grid; }\n";

/// `document` as the message window's column flows it (T207): the column's
/// ground and the Mac's palette, and for correspondence no frame -- the
/// rules GTK's flowing reader adds, from the one place they are written.
fn flow(document: &str, shown: Treatment) -> String {
    use postio_ui::reader::document::{FLOW_CSS, FLOW_FLAT_CSS};
    let mut css = String::from(MAC_FLOW_PALETTE);
    css.push_str(FLOW_CSS);
    if shown == Treatment::AppColours {
        css.push_str(FLOW_FLAT_CSS);
    }
    document.replacen("</style>", &format!("{css}</style>"), 1)
}
