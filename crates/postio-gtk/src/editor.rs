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

/// The channel the bridge reports the caret's formatting on — what a
/// toolbar toggle reflects, named after the same registry ids it serves.
const FORMAT_MESSAGE: &str = "postioFormat";

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

/// What a format watcher receives: the caret's formatting as reported.
type FormatWatcher = Box<dyn Fn(FormatState)>;

/// The formatting in force where the caret sits, as the surface reports it.
///
/// What a toolbar toggle shows: `bold` is true when the selection is inside
/// `Strong`, not when a mode is armed — there are no modes, only the
/// document under the caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub bullet_list: bool,
    pub numbered_list: bool,
    pub quote_block: bool,
}

struct EditorState {
    document: RefCell<Document>,
    history: RefCell<EditHistory>,
    changed: RefCell<Vec<ChangedHandler>>,
    last_edit: Cell<Option<Instant>>,
    coalesce: Duration,
    format: Cell<FormatState>,
    format_watchers: RefCell<Vec<FormatWatcher>>,
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
        content.register_script_message_handler(FORMAT_MESSAGE, None);

        let view = view_with(&content, source);
        let state = Rc::new(EditorState {
            document: RefCell::new(Document::new()),
            history: RefCell::new(EditHistory::new()),
            changed: RefCell::new(Vec::new()),
            last_edit: Cell::new(None),
            coalesce,
            format: Cell::new(FormatState::default()),
            format_watchers: RefCell::new(Vec::new()),
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

        content.connect_script_message_received(Some(FORMAT_MESSAGE), {
            let state = state.clone();
            move |_, value| {
                if !value.is_string() {
                    return;
                }
                let report = value.to_str();
                let tokens: Vec<&str> = report.split_whitespace().collect();
                let format = FormatState {
                    bold: tokens.contains(&"bold"),
                    italic: tokens.contains(&"italic"),
                    bullet_list: tokens.contains(&"bullet_list"),
                    numbered_list: tokens.contains(&"numbered_list"),
                    quote_block: tokens.contains(&"quote_block"),
                };
                if state.format.replace(format) == format {
                    return;
                }
                for watcher in state.format_watchers.borrow().iter() {
                    watcher(format);
                }
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
        seed(&self.view, &document.editor_html());
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

    /// The formatting in force where the caret sits, as last reported.
    pub fn format_state(&self) -> FormatState {
        self.state.format.get()
    }

    /// Run `handler` whenever the caret's formatting changes — the toolbar's
    /// reflection channel. Called only on change, never per keystroke.
    pub fn connect_format_state(&self, handler: impl Fn(FormatState) + 'static) {
        self.state
            .format_watchers
            .borrow_mut()
            .push(Box::new(handler));
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
        seed(&self.view, &document.editor_html());
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

impl Editor {
    /// Fire-and-forget script in the surface. Host code only.
    fn run(&self, script: &str) {
        self.view
            .evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }

    /// Script in the surface, waited out by pumping the main context.
    ///
    /// Test plumbing and nothing else — the composer's cursor assertions
    /// need a synchronous answer, and a test is the one caller allowed to
    /// pump from where it stands.
    fn run_blocking(&self, script: &str) -> String {
        let result: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let slot = result.clone();
        self.view.evaluate_javascript(
            script,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                let value = outcome
                    .map(|value| value.to_str().to_string())
                    .unwrap_or_default();
                *slot.borrow_mut() = Some(value);
            },
        );
        let deadline = Instant::now() + Duration::from_secs(120);
        while result.borrow().is_none() && Instant::now() < deadline {
            while gtk::glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        result.borrow_mut().take().unwrap_or_default()
    }

    /// Put the caret at the start of the body — where a reply is written,
    /// above the quote and above the signature.
    pub fn place_caret_start(&self) {
        self.run(
            "const r = document.createRange(); \
             r.selectNodeContents(document.body); r.collapse(true); \
             const s = window.getSelection(); \
             s.removeAllRanges(); s.addRange(r);",
        );
    }

    /// Pump until the editing shell is loaded and editable.
    ///
    /// `load` is asynchronous where the old `GtkTextBuffer` was not; the
    /// blocking test helpers gate on this so a test that loads-then-types
    /// keeps meaning what it meant.
    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            let ready =
                self.run_blocking("document.body && document.body.isContentEditable ? '1' : '0'");
            if ready == "1" {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The caret's character offset into the body's text, for assertions.
    #[doc(hidden)]
    pub fn caret_offset(&self) -> i32 {
        self.wait_ready();
        self.run_blocking(
            "(() => { const s = window.getSelection(); \
               if (s.rangeCount === 0) return '0'; \
               const r = s.getRangeAt(0).cloneRange(); \
               r.selectNodeContents(document.body); \
               r.setEnd(s.getRangeAt(0).startContainer, s.getRangeAt(0).startOffset); \
               return String(r.toString().length); })()",
        )
        .parse()
        .unwrap_or(0)
    }

    /// Replace the body's content the way typing would — through the
    /// editing machinery, so it registers as an edit.
    #[doc(hidden)]
    pub fn test_type(&self, text: &str) {
        self.wait_ready();
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
        self.run_blocking(&format!(
            "(() => {{ document.execCommand('selectAll'); \
               document.execCommand('insertText', false, '{escaped}'); \
               return 'typed'; }})()"
        ));
        // The edit report crosses the bridge asynchronously; a test that
        // types and immediately reads the record raced it when the surface
        // was a synchronous GtkTextBuffer. Wait the report out, so the old
        // tests keep meaning what they meant.
        let wanted = text.trim().to_owned();
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.document().to_text().trim() != wanted && Instant::now() < deadline {
            while gtk::glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Script against the surface, for assertions about the rendered DOM.
    #[doc(hidden)]
    pub fn test_eval(&self, script: &str) -> String {
        self.run_blocking(script)
    }

    /// Select `from..to` inside the first paragraph's text node, for tests
    /// that format a selection rather than a caret.
    #[doc(hidden)]
    pub fn test_select(&self, nth: u32, from: u32, to: u32) {
        self.wait_ready();
        // A TreeWalker rather than `firstChild.firstChild`: freshly typed
        // text sits as a bare text node until a block gesture wraps it, and
        // formatting splits one node into several, so the `nth` text node
        // is wherever it is, not at a fixed depth.
        self.run_blocking(&format!(
            "(() => {{ const walker = document.createTreeWalker( \
                 document.body, NodeFilter.SHOW_TEXT); \
               let text = walker.nextNode(); \
               for (let i = 0; i < {nth}; i++) text = walker.nextNode(); \
               if (!text) return 'no text'; \
               const sel = window.getSelection(); \
               sel.setBaseAndExtent(text, {from}, text, {to}); \
               return 'selected'; }})()"
        ));
    }
}

/// A formatting gesture, as the registry's composer commands express them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Toggle bold on the selection.
    Bold,
    /// Toggle italic on the selection.
    Italic,
    /// Toggle a bulleted list at the caret's block.
    BulletList,
    /// Toggle a numbered list at the caret's block.
    NumberedList,
    /// Toggle a quote block at the caret's block.
    QuoteBlock,
}

impl Editor {
    /// Apply a formatting toggle to the current selection.
    ///
    /// Each runs the editing command whose output the dialect contract test
    /// pins, and each fires an `input` event, so the edit crosses the bridge
    /// like any keystroke and lands on the same history.
    pub fn format(&self, format: Format) {
        // WebKit dispatches `input` for some editing commands and not others
        // (bold yes, insertUnorderedList no), so the report is dispatched
        // here rather than trusted — a duplicate is absorbed as a no-change.
        let command = match format {
            Format::Bold => "document.execCommand('bold');",
            Format::Italic => "document.execCommand('italic');",
            Format::BulletList => "document.execCommand('insertUnorderedList');",
            Format::NumberedList => "document.execCommand('insertOrderedList');",
            Format::QuoteBlock => {
                // formatBlock toggles nothing on its own; the toggle is ours.
                "if (document.queryCommandValue('formatBlock') === 'blockquote') { \
                     document.execCommand('formatBlock', false, 'p'); \
                 } else { \
                     document.execCommand('formatBlock', false, 'blockquote'); \
                 }"
            }
        };
        self.run(&format!(
            "{command} document.dispatchEvent(new Event('input'));"
        ));
    }

    /// Turn the selection into a link to `href` — or, with nothing selected,
    /// insert the address as its own link text.
    ///
    /// The scheme gate matches the canonical subset: anything but http,
    /// https and mailto is refused here rather than silently dropped by the
    /// parse later, so the caller can say so.
    pub fn create_link(&self, href: &str) -> bool {
        let allowed = ["http://", "https://", "mailto:"]
            .iter()
            .any(|scheme| href.starts_with(scheme));
        if !allowed {
            return false;
        }
        let escaped = href
            .replace('\\', "")
            .replace('\'', "%27")
            .replace('"', "%22");
        self.run(&format!(
            "(() => {{ const sel = window.getSelection(); \
               if (sel.rangeCount === 0) return; \
               if (sel.isCollapsed) {{ \
                 const a = document.createElement('a'); \
                 a.href = '{escaped}'; a.textContent = '{escaped}'; \
                 sel.getRangeAt(0).insertNode(a); \
                 document.dispatchEvent(new Event('input')); \
               }} else {{ \
                 document.execCommand('createLink', false, '{escaped}'); \
                 document.dispatchEvent(new Event('input')); \
               }} }})()"
        ));
        true
    }

    /// Put an inline image at the caret — the tail of a paste or drop whose
    /// bytes are already in the blob store under `content_id`.
    ///
    /// The `src` is built from a [`postio_body::ContentId`], so only an id
    /// that satisfied its rules can ever reach the DOM; the shell's CSP and
    /// the `postio-cid:` scheme handler take it from there.
    pub fn insert_image(&self, content_id: &postio_body::ContentId, alt: &str) {
        let mut img = String::from("<img src=\"");
        img.push_str(&postio_body::editor_image_src(content_id));
        img.push_str("\" alt=\"");
        for c in alt.chars() {
            match c {
                '&' => img.push_str("&amp;"),
                '<' => img.push_str("&lt;"),
                '>' => img.push_str("&gt;"),
                '"' => img.push_str("&quot;"),
                other => img.push(other),
            }
        }
        img.push_str("\">");
        let escaped = img.replace('\\', "\\\\").replace('\'', "\\'");
        // A paste can be the first gesture into a fresh body, before any
        // click or keystroke has given the document a caret — insertHTML
        // silently does nothing without one, so fall back to the end.
        self.run(&format!(
            "(() => {{ const sel = window.getSelection(); \
               if (sel.rangeCount === 0) {{ \
                 const range = document.createRange(); \
                 range.selectNodeContents(document.body); \
                 range.collapse(false); \
                 sel.addRange(range); \
               }} \
               document.execCommand('insertHTML', false, '{escaped}'); \
               document.dispatchEvent(new Event('input')); }})()"
        ));
    }
}
