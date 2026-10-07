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

use super::scheme;
use crate::reader::BlobSource;
use postio_ui::editor::document as editor_document;

/// A fixed, non-`http(s)` base for the editing shell, so edited content is
/// never same-origin with anything real — the same reasoning as the
/// reader's `postio-reader:///`.
///
/// Re-exported from `postio-ui` rather than restated: the document that
/// declares the policy and the view that loads it must agree, and two copies
/// of a security-relevant string are two that can drift (#567).
pub use postio_ui::editor::document::EDITOR_BASE_URI;

/// The bridge script — profile settings plus edit reporting.
///
/// `include_str!` rather than a runtime resource lookup: compiled into the
/// binary is the property ADR 0003's "shipped in the bundle" exists for,
/// and it removes a registration-order dependency from every test that
/// builds an editor. The file lives beside the other bundled assets in
/// `data/`.
const EDITOR_SCRIPT_BODY: &str = include_str!("editor.js");

/// The whole script, table and all.
///
/// The markdown table is *generated* from `postio_ui::editor::markdown`
/// rather than restated in JavaScript, so the set of supported sequences has
/// one source. A hand-written copy in `editor.js` is a copy that drifts, and
/// the thing it would drift from is the contract both frontends implement.
fn editor_script() -> &'static str {
    static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SCRIPT.get_or_init(|| format!("{}{EDITOR_SCRIPT_BODY}", markdown_table_js()))
}

/// `POSTIO_MARKDOWN`, as a JavaScript literal.
fn markdown_table_js() -> String {
    use postio_ui::editor::markdown::{SEQUENCES, Trigger};
    use std::fmt::Write as _;

    let mut out = String::from("const POSTIO_MARKDOWN = [\n");
    for sequence in SEQUENCES {
        let trigger = match sequence.trigger {
            Trigger::LinePrefix => "line_prefix",
            Trigger::Wrapping => "wrapping",
        };
        // The markers are `&'static str` from a table in this workspace, not
        // anybody's input, and every one of them is punctuation -- but they
        // are being written into source, so they are escaped rather than
        // trusted to stay that way.
        let _ = writeln!(
            out,
            "    {{ marker: \"{}\", command: \"{}\", trigger: \"{trigger}\" }},",
            sequence.marker.replace('\\', "\\\\").replace('"', "\\\""),
            sequence.command,
        );
    }
    out.push_str("];\n");
    out
}

/// The script-message channel the bridge reports edits on.
const EDITED_MESSAGE: &str = "postioEdited";

/// The channel a markdown conversion reports on.
///
/// Separate from [`EDITED_MESSAGE`] because it means something the ordinary
/// channel cannot say: *this edit begins a new undo step*. A conversion
/// arrives in the middle of a typing run, and `absorb`'s coalescing would
/// fold it into that run -- so one undo would take back the whole sentence
/// rather than the conversion, and the literal characters the user meant
/// would be gone with it (FR-070).
const CONVERTED_MESSAGE: &str = "postioConverted";

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
        editor_script(),
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
    paint_ground(&view);
    super::web_process::watch(&view);
    // The scheme can change while a draft is open, and the only right answer
    // is a new sheet rather than a new document: reloading would take the
    // caret and the undo history with it (FR-075).
    let manager = adw::StyleManager::default();
    let dark = manager.connect_dark_notify(glib::clone!(
        #[weak]
        view,
        move |_| restyle(&view)
    ));
    let handler = RefCell::new(Some(dark));
    view.connect_destroy(move |_| {
        if let Some(dark) = handler.borrow_mut().take() {
            adw::StyleManager::default().disconnect(dark);
        }
    });
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
    let shell = editor_document::wrap_document(inner_html, presentation());
    view.load_html(&shell, Some(EDITOR_BASE_URI));
}

/// Script that puts the caret after everything in the body.
fn run_caret_end(view: &webkit6::WebView) {
    view.evaluate_javascript(
        "const r = document.createRange(); \
         r.selectNodeContents(document.body); r.collapse(false); \
         const s = window.getSelection(); \
         s.removeAllRanges(); s.addRange(r);",
        None,
        None,
        None::<&gtk::gio::Cancellable>,
        |_| {},
    );
}

/// How the surface should be drawn right now.
///
/// The scheme comes from libadwaita rather than from the engine: a web view
/// resolves `prefers-color-scheme` from its own settings, which is how the
/// editing surface managed to be white inside a dark application.
fn presentation() -> editor_document::Presentation {
    editor_document::Presentation {
        dark: adw::StyleManager::default().is_dark(),
        ..editor_document::Presentation::default()
    }
}

/// Paints the ground on the widget as well as the document.
///
/// The document paints `--r-ground` on `body`, but only once it has parsed,
/// and a web view between one document and the next has nothing to paint
/// from. That interval is the white flash — the reader's `paint_ground`
/// exists for the same reason and is where this was learned.
fn paint_ground(view: &webkit6::WebView) {
    let dark = adw::StyleManager::default().is_dark();
    match editor_document::editor_ground(dark).parse::<gtk::gdk::RGBA>() {
        Ok(ground) => view.set_background_color(&ground),
        Err(error) => glib::g_warning!(
            "postio",
            "could not parse the editor ground colour: {error}"
        ),
    }
}

/// Re-applies the sheet for the current scheme **without reloading**.
///
/// A reload would take the caret and the undo history with it (FR-075), so
/// the sheet is replaced in place: the document keeps its DOM and its
/// selection, and only the `<style>` element's text changes.
pub fn restyle(view: &webkit6::WebView) {
    restyle_with(view, None);
}

/// [`restyle`], with `flow` -- a host column's palette and rhythm -- added
/// after the sheet, where it wins.
fn restyle_with(view: &webkit6::WebView, flow: Option<&str>) {
    paint_ground(view);
    let mut css = editor_document::editor_css(presentation());
    if let Some(flow) = flow {
        css.push_str(flow);
    }
    // `textContent`, not `innerHTML`: a stylesheet is text, and the engine
    // would otherwise be parsing our own CSS as markup looking for entities.
    let script = format!(
        "(() => {{ const s = document.querySelector('style'); \
         if (s) s.textContent = {}; }})()",
        json_string(&css)
    );
    view.evaluate_javascript(&script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
}

/// The host column's palette as CSS, and its rhythm, when the editor reads
/// one ([`Editor::flow_in`]).
fn flow_css(state: &EditorState) -> Option<String> {
    let flow = state.flow.borrow();
    if flow.is_empty() {
        return None;
    }
    let declarations: Vec<String> = flow
        .iter()
        .filter_map(|(variable, probe)| {
            let probe = probe.upgrade()?;
            Some(format!(
                "{variable}:{}",
                crate::body_view::css_colour(&probe.color())
            ))
        })
        .collect();
    Some(format!(
        "\n:root{{{};}}\n{FLOW_CSS}",
        declarations.join(";")
    ))
}

/// Paint the column's ground on the widget too, so the interval before a
/// document parses is the column's colour, not the generated palette's.
fn paint_flow_ground(view: &webkit6::WebView, state: &EditorState) {
    let flow = state.flow.borrow();
    if let Some(probe) = flow
        .iter()
        .find(|(variable, _)| *variable == GROUND)
        .and_then(|(_, probe)| probe.upgrade())
    {
        view.set_background_color(&probe.color());
    }
}

/// The palette variable a column's ground is read into.
pub const GROUND: &str = "--r-ground";

/// `value` as a JavaScript string literal.
///
/// Hand-rolled rather than pulled from a JSON crate: this crate has no JSON
/// dependency and wants none for one function, and what has to be escaped in
/// a double-quoted literal is a short, closed list.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            // U+2028 and U+2029 terminate a line in JavaScript but not in
            // JSON, which is the classic way a valid string becomes a syntax
            // error. CSS can hold them inside a `content:` value.
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other if (other as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
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
    let Some(action) = decision
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
    /// Inside `Strong`.
    pub bold: bool,
    /// Inside `Emphasis`.
    pub italic: bool,
    /// Inside a bulleted list.
    pub bullet_list: bool,
    /// Inside a numbered list.
    pub numbered_list: bool,
    /// Inside a quote block.
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
    /// Whether anything has ever been loaded into the view.
    ///
    /// The first `load_html` on a fresh `WebView` starts a WebKit web process,
    /// which is tens of milliseconds, and it would otherwise all fall on the
    /// first composition somebody writes. See [`Editor::warm`].
    loaded: Cell<bool>,
    /// The caret goes to the end of the body once the document being loaded
    /// is there: a script run before then lands on the page being replaced.
    caret_end_pending: Cell<bool>,
    /// The palette variables a host's column supplies, each read from a
    /// probe its stylesheet colours ([`Editor::flow_in`]); empty for a
    /// surface that keeps the generated palette.
    flow: RefCell<Vec<(&'static str, glib::WeakRef<gtk::Widget>)>>,
}

/// What a host's column adds to the editing sheet (specs/007-postio-focus
/// T221): the text starts at the column's edge, with no inset of its own,
/// and runs on the app colours treatment's rhythm -- a 24px line,
/// paragraphs 12 apart -- so what is written is laid out as it will be
/// read. No colour is written here: the column's own are read from its
/// probes at each restyle.
const FLOW_CSS: &str =
    "\nbody { padding-left: 0; padding-right: 0; line-height: 24px; }\np { margin: 0 0 12px 0; }\n";

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
            editor_script(),
            webkit6::UserContentInjectedFrames::TopFrame,
            webkit6::UserScriptInjectionTime::End,
            &[],
            &[],
        ));
        content.register_script_message_handler(EDITED_MESSAGE, None);
        content.register_script_message_handler(CONVERTED_MESSAGE, None);
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
            loaded: Cell::new(false),
            caret_end_pending: Cell::new(false),
            flow: RefCell::default(),
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

        content.connect_script_message_received(Some(CONVERTED_MESSAGE), {
            let state = state.clone();
            move |_, value| {
                if !value.is_string() {
                    return;
                }
                absorb_as_new_step(&state, &value.to_str());
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

    /// Draw the document in a host's column (specs/007-postio-focus T221):
    /// each of `probes` names a reader palette variable (`--r-ground`,
    /// `--r-ink`, ...) and a widget whose CSS `color` the host's stylesheet
    /// sets to its own token for it, as the open message's body reads its
    /// column (`BodyView::set_ground`, T203, T211). The document is drawn on
    /// the column's ground, in its ink, from its edge; read again whenever
    /// the scheme changes, so light and dark both follow the host.
    pub fn flow_in(&self, probes: Vec<(&'static str, gtk::Widget)>) {
        let first = self.state.flow.borrow().is_empty();
        self.state.flow.replace(
            probes
                .into_iter()
                .map(|(variable, probe)| (variable, probe.downgrade()))
                .collect(),
        );
        self.restyle();
        if !first {
            return;
        }
        // The style manager says the scheme changed before the stylesheet
        // that paints the probes is in place: read them once the main loop
        // has turned, after the plain restyle `view_with` connected.
        let view = self.view.downgrade();
        let state = Rc::downgrade(&self.state);
        let handler = adw::StyleManager::default().connect_dark_notify(move |_| {
            let (view, state) = (view.clone(), state.clone());
            glib::idle_add_local_once(move || {
                if let (Some(view), Some(state)) = (view.upgrade(), state.upgrade()) {
                    restyle_with(&view, flow_css(&state).as_deref());
                    paint_flow_ground(&view, &state);
                }
            });
        });
        let handler = RefCell::new(Some(handler));
        self.view.connect_destroy(move |_| {
            if let Some(handler) = handler.borrow_mut().take() {
                adw::StyleManager::default().disconnect(handler);
            }
        });
        // And whenever the view comes on screen: a probe is only dressed by
        // the host's stylesheet once it is in the host's window, and a
        // draft is loaded before the dialog it is written in is presented.
        // The same once a document has loaded on screen, since one seeded
        // before then read the probes before they were dressed.
        let state = Rc::downgrade(&self.state);
        self.view.connect_map(move |view| {
            if let Some(state) = state.upgrade() {
                restyle_with(view, flow_css(&state).as_deref());
                paint_flow_ground(view, &state);
            }
        });
        let state = Rc::downgrade(&self.state);
        self.view.connect_load_changed(move |view, event| {
            if event == webkit6::LoadEvent::Finished
                && let Some(state) = state.upgrade()
                && state.caret_end_pending.take()
            {
                run_caret_end(view);
            }
            if event == webkit6::LoadEvent::Finished
                && view.is_mapped()
                && let Some(state) = state.upgrade()
            {
                restyle_with(view, flow_css(&state).as_deref());
                paint_flow_ground(view, &state);
            }
        });
    }

    /// Load `inner_html` with the host's column added to the sheet.
    fn seed(&self, inner_html: &str) {
        match flow_css(&self.state) {
            Some(flow) => {
                let shell = editor_document::wrap_document(inner_html, presentation()).replacen(
                    "</style>",
                    &format!("{flow}</style>"),
                    1,
                );
                paint_flow_ground(&self.view, &self.state);
                self.view.load_html(&shell, Some(EDITOR_BASE_URI));
            }
            None => seed(&self.view, inner_html),
        }
    }

    /// The sheet again, with the host's column read afresh.
    fn restyle(&self) {
        restyle_with(&self.view, flow_css(&self.state).as_deref());
        paint_flow_ground(&self.view, &self.state);
    }

    /// Show `document` for editing, forgetting any previous history — a
    /// draft opening, not an edit.
    pub fn load(&self, document: Document) {
        self.state.history.borrow_mut().clear();
        self.state.last_edit.set(None);
        self.state.caret_end_pending.set(false);
        self.seed(&document.editor_html());
        self.state.loaded.set(true);
        *self.state.document.borrow_mut() = document;
    }

    /// Start the editing surface before anybody is waiting for it.
    ///
    /// The first `load_html` on a fresh `WebView` starts a WebKit web process.
    /// Measured here, splitting `Composer::open` into its parts: 28.7ms on the
    /// first open against 0.2ms on every one after it — so without this, the
    /// whole cost falls on the first composition a person writes, which is the
    /// one they notice (#1216).
    ///
    /// Seeds an empty document, which is what a fresh composer holds anyway,
    /// so a later [`load`](Self::load) replaces a blank page rather than
    /// starting one. Does nothing once anything has been loaded — including a
    /// draft somebody is part-way through typing, which a second seed would
    /// throw away.
    pub fn warm(&self) {
        if self.state.loaded.get() {
            return;
        }
        self.load(Document::default());
    }

    /// Whether the editing surface has been started. See [`warm`](Self::warm).
    pub fn is_warm(&self) -> bool {
        self.state.loaded.get()
    }

    /// The document as it now stands. The record; the DOM is its copy.
    pub fn document(&self) -> Document {
        self.state.document.borrow().clone()
    }

    /// Run `handler` after every absorbed edit, undo and redo.
    pub fn connect_changed(&self, handler: impl Fn(&Document) + 'static) {
        self.state.changed.borrow_mut().push(Box::new(handler));
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
        self.seed(&document.editor_html());
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
    let now = Instant::now();
    let within_run = state
        .last_edit
        .get()
        .is_some_and(|last| now.duration_since(last) < state.coalesce);
    absorb_with(state, html, within_run);
}

/// [`absorb`], but this edit always starts its own undo step.
///
/// What a markdown conversion needs. The edit before it is the literal text
/// the user typed — `**loudly**`, markers and all — and that state has to be
/// something one undo can return to (FR-070). Coalescing it into the typing
/// run would make undo take back the sentence instead, and the escape hatch
/// that makes an automatic conversion tolerable would not be there.
fn absorb_as_new_step(state: &Rc<EditorState>, html: &str) {
    absorb_with(state, html, false);
}

fn absorb_with(state: &Rc<EditorState>, html: &str, within_run: bool) {
    let after = parse(html);
    let before = state.document.borrow().clone();
    if after == before {
        return;
    }

    {
        let mut history = state.history.borrow_mut();
        if !(within_run && history.amend(after.clone())) {
            history.record(before, after.clone());
        }
    }
    state.last_edit.set(Some(Instant::now()));

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

    /// Put the caret at the end of the body — where a draft that was left
    /// half written is picked up again. Waits for the document being loaded
    /// when one is on its way.
    pub fn place_caret_end(&self) {
        if self.view.is_loading() || !self.state.loaded.get() {
            self.state.caret_end_pending.set(true);
        } else {
            run_caret_end(&self.view);
        }
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
