//! The composer's editing surface: the one `WebView` where JavaScript runs.
//!
//! ADR 0003's licence, restated at the place it applies: script that arrived
//! in a message never executes anywhere, and Postio's own bundled script is
//! not message content. This view exists to run *that* script — the editing
//! bridge — over a `contenteditable` document, and nothing else changes: the
//! reader keeps JavaScript off, and this profile keeps every other door the
//! reader closes closed too. `enable_javascript_markup` stays **off**, which
//! is the setting that makes the distinction mechanical rather than
//! disciplinary — a `<script>` tag inside edited or pasted content is inert
//! markup here, while the host's own injected script runs.
//!
//! Network is closed by construction, the same three ways the reader closes
//! it: an ephemeral [`NetworkSession`], a [`WebContext`] whose only
//! registered scheme resolves `postio-cid:` from the local blob store, and a
//! CSP on the editing shell that names no remote origin. The dialect the
//! surface emits is pinned by `tests/gtk_editable_dialect.rs`; the paragraph
//! separator and `styleWithCSS` settings that dialect depends on are applied
//! here, by an injected script, so no later caller can forget them.
//!
//! [`NetworkSession`]: webkit6::NetworkSession
//! [`WebContext`]: webkit6::WebContext

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use postio_body::{Document, EditHistory, parse};
use webkit6::prelude::*;

use crate::reader::scheme::{self, BlobSource};

/// A fixed, non-`http(s)` base for the editing shell, so edited content is
/// never same-origin with anything real — the same reasoning as the
/// reader's `postio-reader:///`.
pub const EDITOR_BASE_URI: &str = "postio-editor:///";

/// The CSP the editing shell carries: no remote origin can be named, styles
/// stay inline (the shell's own), and images resolve only through the local
/// blob scheme.
const EDITOR_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:";

/// The bridge script — profile settings plus edit reporting.
///
/// `include_str!` rather than a runtime resource lookup: compiled into the
/// binary is the property ADR 0003's "shipped in the bundle" exists for,
/// and it removes a registration-order dependency from every test that
/// builds an editor. The file lives beside the other bundled assets in
/// `data/`.
const EDITOR_SCRIPT: &str = include_str!("../data/editor.js");

/// The script-message channel the bridge reports edits on.
const EDITED_MESSAGE: &str = "postioEdited";

/// How long after an edit the next one still amends the same undo step.
///
/// What makes a typing run one `Ctrl+Z` rather than one per keystroke; the
/// pause that ends a run is a human pause, so the default is human-sized.
const COALESCE: Duration = Duration::from_millis(700);

/// Build the editing view: JavaScript on for the host, everything else the
/// reader's lockdown list closes, closed.
///
/// `source` resolves `postio-cid:` references — pasted inline images, once
/// #341 lands them in the blob store. The view arrives empty; [`seed`] loads
/// the editing shell.
pub fn editing_view(source: Rc<dyn BlobSource>) -> webkit6::WebView {
    let content = webkit6::UserContentManager::new();
    content.add_script(&webkit6::UserScript::new(
        EDITOR_SCRIPT,
        webkit6::UserContentInjectedFrames::TopFrame,
        webkit6::UserScriptInjectionTime::End,
        &[],
        &[],
    ));
    view_with(&content, source)
}

/// The shared assembly: session, context, scheme, settings, policy.
fn view_with(
    content: &webkit6::UserContentManager,
    source: Rc<dyn BlobSource>,
) -> webkit6::WebView {
    let network_session = webkit6::NetworkSession::new_ephemeral();
    network_session.set_persistent_credential_storage_enabled(false);

    let context = webkit6::WebContext::new();
    scheme::register(&context, source);

    let view = webkit6::WebView::builder()
        .web_context(&context)
        .network_session(&network_session)
        .user_content_manager(content)
        .settings(&editing_settings())
        .hexpand(true)
        .vexpand(true)
        .build();
    view.add_css_class("postio-editor-view");
    view.set_accessible_role(gtk::AccessibleRole::TextBox);
    view.connect_decide_policy(handle_decide_policy);
    view
}

/// Load the editing shell around `inner_html`, which must already be
/// canonical-subset markup (a `Document`'s `to_html`, or empty).
///
/// The shell is Postio's own markup, not message content — quoted material
/// goes through `postio_body::parse` *before* it can appear here, which is
/// what makes running script beside it acceptable at all (ADR 0003,
/// hardening requirement 2).
pub fn seed(view: &webkit6::WebView, inner_html: &str) {
    let shell = format!(
        "<!doctype html><html><head>\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{EDITOR_CSP}\">\
         </head><body contenteditable=\"true\">{inner_html}</body></html>"
    );
    view.load_html(&shell, Some(EDITOR_BASE_URI));
}

/// The reader's lockdown list with exactly one line changed.
///
/// Deliberately not shared with `reader::view::hardened_settings`: the two
/// profiles must be *diffable at a glance*, and a shared function with a
/// flag would hide the one difference that matters inside a parameter.
fn editing_settings() -> webkit6::Settings {
    let settings = webkit6::Settings::new();
    // The one difference: the host's script runs here. Markup-borne script
    // still does not — that line is identical to the reader's on purpose.
    settings.set_enable_javascript(true);
    settings.set_enable_javascript_markup(false);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_javascript_can_access_clipboard(false);
    settings.set_enable_html5_database(false);
    settings.set_enable_html5_local_storage(false);
    settings.set_enable_page_cache(false);
    settings.set_enable_media(false);
    settings.set_enable_media_stream(false);
    settings.set_enable_mediasource(false);
    settings.set_enable_encrypted_media(false);
    settings.set_enable_webrtc(false);
    settings.set_enable_webgl(false);
    settings.set_enable_webaudio(false);
    settings.set_enable_fullscreen(false);
    settings.set_enable_developer_extras(cfg!(debug_assertions));
    settings
}

/// Nothing navigates an editor.
///
/// The reader lets a user's own click leave for the browser; here a click on
/// a link is an editing gesture — the caret moves into the link text — so
/// every link-clicked navigation is refused outright, and everything else
/// (the initial `load_html`) proceeds as WebKit intends.
fn handle_decide_policy(
    _view: &webkit6::WebView,
    decision: &webkit6::PolicyDecision,
    kind: webkit6::PolicyDecisionType,
) -> bool {
    if kind != webkit6::PolicyDecisionType::NavigationAction {
        return false;
    }
    let Some(mut action) = decision
        .downcast_ref::<webkit6::NavigationPolicyDecision>()
        .and_then(|decision| decision.navigation_action())
    else {
        return false;
    };
    if action.navigation_type() == webkit6::NavigationType::LinkClicked {
        decision.ignore();
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// The bridge
// ---------------------------------------------------------------------------

/// What a change handler receives: the document as it now stands.
type ChangedHandler = Box<dyn Fn(&Document)>;

struct EditorState {
    document: RefCell<Document>,
    history: RefCell<EditHistory>,
    changed: RefCell<Vec<ChangedHandler>>,
    last_edit: Cell<Option<Instant>>,
    coalesce: Duration,
}

/// The editing surface with its document attached: Document in, WebKit's
/// dialect out, Document again.
///
/// The DOM is a working copy and never the record (ADR 0004 Q3): every edit
/// the bridge script reports is parsed straight back into the canonical
/// [`Document`], and that parse — total, narrowing — is the sanitisation on
/// the way out that hardening requirement 5 demands. Undo is the document's
/// own ([`EditHistory`]), never the widget's, with a typing run coalesced
/// into one step by [`EditHistory::amend`] inside a human-sized pause.
pub struct Editor {
    view: webkit6::WebView,
    state: Rc<EditorState>,
}

impl Editor {
    /// An editor over `source` for its `postio-cid:` images.
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::with_coalesce(source, COALESCE)
    }

    /// As [`new`](Self::new), choosing the typing-run window — what the
    /// tests use to make coalescing deterministic instead of racing a
    /// wall clock.
    pub fn with_coalesce(source: Rc<dyn BlobSource>, coalesce: Duration) -> Self {
        let content = webkit6::UserContentManager::new();
        content.add_script(&webkit6::UserScript::new(
            EDITOR_SCRIPT,
            webkit6::UserContentInjectedFrames::TopFrame,
            webkit6::UserScriptInjectionTime::End,
            &[],
            &[],
        ));
        content.register_script_message_handler(EDITED_MESSAGE, None);

        let view = view_with(&content, source);
        let state = Rc::new(EditorState {
            document: RefCell::new(Document::new()),
            history: RefCell::new(EditHistory::new()),
            changed: RefCell::new(Vec::new()),
            last_edit: Cell::new(None),
            coalesce,
        });

        content.connect_script_message_received(Some(EDITED_MESSAGE), {
            let state = state.clone();
            move |_, value| {
                if !value.is_string() {
                    return;
                }
                absorb(&state, &value.to_str());
            }
        });

        Editor { view, state }
    }

    /// The widget to embed. The pane owns layout; the editor owns content.
    pub fn widget(&self) -> &webkit6::WebView {
        &self.view
    }

    /// Show `document` for editing, forgetting any previous history — a
    /// draft opening, not an edit.
    pub fn load(&self, document: Document) {
        self.state.history.borrow_mut().clear();
        self.state.last_edit.set(None);
        seed(&self.view, &document.to_html());
        *self.state.document.borrow_mut() = document;
    }

    /// The document as it now stands. The record; the DOM is its copy.
    pub fn document(&self) -> Document {
        self.state.document.borrow().clone()
    }

    /// Run `handler` after every absorbed edit, undo and redo.
    pub fn connect_changed(&self, handler: impl Fn(&Document) + 'static) {
        self.state.changed.borrow_mut().push(Box::new(handler));
    }

    /// Step back one typing run. `Ctrl+Z`.
    pub fn undo(&self) {
        let Some(document) = self.state.history.borrow_mut().undo() else {
            return;
        };
        self.show(document);
    }

    /// Step forward again.
    pub fn redo(&self) {
        let Some(document) = self.state.history.borrow_mut().redo() else {
            return;
        };
        self.show(document);
    }

    /// Whether `Ctrl+Z` has anywhere to go.
    pub fn can_undo(&self) -> bool {
        self.state.history.borrow().can_undo()
    }

    /// Put `document` on screen and on record, ending any typing run —
    /// the shared tail of undo and redo.
    fn show(&self, document: Document) {
        self.state.last_edit.set(None);
        seed(&self.view, &document.to_html());
        *self.state.document.borrow_mut() = document;
        let current = self.state.document.borrow();
        for handler in self.state.changed.borrow().iter() {
            handler(&current);
        }
    }
}

/// Fold one reported edit into the record.
///
/// A free function over the shared state, not a method: the script-message
/// handler holds only the `Rc`, never a whole `Editor`, so dropping the
/// editor drops the state as soon as WebKit lets go of the closure.
fn absorb(state: &Rc<EditorState>, html: &str) {
    let after = parse(html);
    let before = state.document.borrow().clone();
    if after == before {
        return;
    }

    let now = Instant::now();
    let within_run = state
        .last_edit
        .get()
        .is_some_and(|last| now.duration_since(last) < state.coalesce);
    {
        let mut history = state.history.borrow_mut();
        if !(within_run && history.amend(after.clone())) {
            history.record(before, after.clone());
        }
    }
    state.last_edit.set(Some(now));

    *state.document.borrow_mut() = after;
    let current = state.document.borrow();
    for handler in state.changed.borrow().iter() {
        handler(&current);
    }
}
