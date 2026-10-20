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
    /// The one quiet sentence under the sender when they may not be who
    /// they say -- failed authentication, or a display name that is another
    /// address (#1928) -- or `None`, which is nearly all mail. Worded by
    /// `postio_ui::reader::authenticity`, judged by the host with the
    /// account's provider.
    pub sender_warning: Option<String>,
    /// The unsubscribe banner over list mail, or `None`: GTK's notice band
    /// (`postio_ui::unsubscribe::banner`), which personal mail and mail on
    /// its way out never get. Reading it records nothing; only
    /// `activateUnsubscribe` does (#1585).
    pub unsubscribe: Option<crate::UnsubscribeOfferFfi>,
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

/// A message as the window opens on it: what surrounds the body, and the
/// body, from one read of the store (#1609, #1589).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FocusMessageOpenFfi {
    /// The chrome: subject, position, sender block, verbs, attachments.
    pub view: crate::FocusMessageViewFfi,
    /// The body, with the facts its render paid for.
    pub document: FocusReaderDocumentFfi,
}

/// The message window's column, at the window's own width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FocusMessageColumnFfi {
    /// The content column, for the treatment shown.
    pub column_width: i32,
    /// Whether Label, Move and Delete fold into More at that width.
    pub folds_into_more: bool,
}

/// The column of a message window `window_width` wide, for a body shown as
/// `treatment`: what the window reads at once the person resizes it (M1).
/// The document's `column_width` is the window's as it opened.
#[uniffi::export]
pub fn focus_message_column(window_width: i32, treatment: TreatmentFfi) -> FocusMessageColumnFfi {
    let geometry =
        postio_ui::focus_dialog::Geometry::for_platform(postio_config::paths::Platform::Apple);
    FocusMessageColumnFfi {
        column_width: geometry.column_width(window_width, treatment.into()),
        folds_into_more: geometry.folds_into_more(window_width),
    }
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
        crate::session::blocking(async {
            let reading = self.reading_of(message).await;
            self.focus_reader_answers(reading.as_ref(), remote, chosen, main_width)
                .await
        })
    }

    /// `message` as the window opens on it, shown from row `index` of
    /// `total`: [`focus_message_view`](Self::focus_message_view) and
    /// [`focus_reader_document`](Self::focus_reader_document) together, from
    /// **one** read. Asked separately, each read and decoded the body for
    /// itself, so every open paid for it twice (#1609). The two single
    /// calls stay for what changes one half alone: `O`, a grant, a label.
    pub fn focus_message_open(
        &self,
        message: i64,
        index: u32,
        total: u32,
        remote: crate::RemoteImagesFfi,
        chosen: Option<TreatmentFfi>,
        main_width: i32,
    ) -> FocusMessageOpenFfi {
        let row = self.focus_row_of(message);
        crate::session::blocking(async {
            let reading = self.reading_of(message).await;
            FocusMessageOpenFfi {
                view: self
                    .focus_message_answers(message, index, total, row, reading.as_ref())
                    .await,
                document: self
                    .focus_reader_answers(reading.as_ref(), remote, chosen, main_width)
                    .await,
            }
        })
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
    /// `None`: "Always for this sender". Kept on the person the sender is
    /// (ADR 0048), so every address of theirs is drawn the same way; GTK's
    /// reader reads the same place.
    pub fn always_treatment(&self, sender: String, treatment: Option<TreatmentFfi>) {
        let Some(client) = self.client() else {
            return;
        };
        let treatment: Option<Treatment> = treatment.map(Into::into);
        if let Err(error) = crate::session::blocking(
            client.set_treatment(&sender, treatment.map(Treatment::attribute_value)),
        ) {
            tracing::error!(%error, "a sender's treatment could not be saved: {error}");
        }
    }
}

impl Session {
    /// The treatment the person chose to always draw `sender`'s mail in, if
    /// they did.
    async fn sender_treatment(&self, sender: &str) -> Option<Treatment> {
        let client = self.client()?;
        let chosen = client.treatments().await.ok()?;
        chosen
            .into_iter()
            .find(|(address, _)| address == sender)
            .and_then(|(_, treatment)| Treatment::from_attribute(&treatment))
    }

    /// The document for the message `reading` read, or the plate for why
    /// there is none. No store read of its own: the body, the row and the
    /// send state are all on `reading`, which the caller read once.
    async fn focus_reader_answers(
        &self,
        reading: Option<&postio_client::protocol::Reading>,
        remote: crate::RemoteImagesFfi,
        chosen: Option<TreatmentFfi>,
        main_width: i32,
    ) -> FocusReaderDocumentFfi {
        use postio_client::protocol::Body;
        let geometry =
            postio_ui::focus_dialog::Geometry::for_platform(postio_config::paths::Platform::Apple);
        let window_width = geometry.dialog_width(main_width);
        let answer = |html: String, treated: Option<postio_ui::reader::document::Treated>| {
            let shown = treated.map_or(Treatment::AppColours, |treated| treated.shown);
            FocusReaderDocumentFfi {
                html,
                notice: None,
                caveat: None,
                sender_warning: None,
                unsubscribe: None,
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
        let Some(reading) = reading else {
            return plate(Absent::Missing);
        };
        let (body, encoding_problems) = match &reading.body {
            Body::Ready {
                body,
                encoding_problems,
            } => (body, *encoding_problems),
            Body::Partial => return plate(Absent::Partial),
            Body::Offline => return plate(Absent::Offline),
            Body::Missing => return plate(Absent::Missing),
            Body::Empty => return plate(Absent::Empty),
            Body::ForeignDraft => return plate(Absent::ForeignDraft),
            Body::Failed => return plate(Absent::Failed),
        };
        let row = reading.row.as_deref();
        let from = row.and_then(|row| row.from.first());
        let sender = from.map(|address| address.address.to_lowercase());
        // The send state is the banner's gate (#1525): mail on its way out
        // is never offered a way to leave a list.
        let unsubscribe = row.and_then(|row| {
            postio_ui::unsubscribe::banner(
                reading.send_state,
                row.list_id.as_deref(),
                row.promoted.map(|said| said.unsubscribe_offered),
                &row.from,
            )
            .map(|offer| crate::UnsubscribeOfferFfi {
                list_identifier: offer.list_identifier,
                summary: offer.summary,
                action: postio_ui::unsubscribe::ACTION.to_owned(),
            })
        });
        let remembered = match sender.as_deref() {
            Some(sender) => self.sender_treatment(sender).await,
            None => None,
        };
        let rendered = body_html_treated(body, remote, chosen.map(Into::into).or(remembered), None);
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
        let notice = self.held_back_notice(rendered.held_back, remote, from);
        let document = flow(
            &document_for_treated(&html, &rendered.styles, remote, shown),
            shown,
        );
        FocusReaderDocumentFfi {
            notice,
            caveat: decode_caveat(encoding_problems).map(str::to_owned),
            sender_warning: reading.sender_warning.clone(),
            unsubscribe,
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
        // WebKit resolves the system's colours, and its own form controls,
        // as dark only for a page that says it can be drawn so; without
        // this the ink was the light appearance's black on a dark window.
        // Paper says nothing: its web view is light whatever the window.
        css.push_str(":root { color-scheme: light dark; }\n");
        css.push_str(FLOW_FLAT_CSS);
    }
    document.replacen("</style>", &format!("{css}</style>"), 1)
}
