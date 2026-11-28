//! The composer's editing surface: the one `WebView` where JavaScript runs (ADR 0003).
//! `enable_javascript_markup` stays off, so a `<script>` in edited or pasted content is inert while
//! the host's injected script runs.
//! Network is closed as in the reader: ephemeral session, one scheme resolving `postio-cid:`
//! locally, a CSP naming no remote origin.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use postio_body::markdown::DocPosition;
use postio_body::{Document, EditHistory, EditStep, Join, Selection, parse};
use webkit6::prelude::*;

use super::scheme;
use crate::reader::BlobSource;
use postio_ui::editor::document as editor_document;

/// Re-exported rather than restated: two copies of a security-relevant string can drift (#567).
pub use postio_ui::editor::document::EDITOR_BASE_URI;

/// One script for every frontend, so the same keystrokes write the same HTML on GTK and the Mac.
pub use postio_ui::compose::editor_script;

const EDITED_MESSAGE: &str = "postioEdited";

/// Separate from `EDITED_MESSAGE` because it means "this edit begins a new undo step": coalescing
/// would fold a conversion into the typing run, and one undo would take back the whole sentence.
const CONVERTED_MESSAGE: &str = "postioConverted";

const FORMAT_MESSAGE: &str = "postioFormat";

const COALESCE: Duration = Duration::from_millis(700);

/// `source` resolves `postio-cid:` references; the view arrives empty and `seed` loads the shell.
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
    // The scheme can change while a draft is open: a new sheet, not a reload, which would take the
    // caret and undo history (FR-075).
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

/// `inner_html` must be canonical-subset markup. Quoted material goes through `postio_body::parse`
/// before it can appear here, which is what makes running script beside it acceptable (ADR 0003).
pub fn seed(view: &webkit6::WebView, inner_html: &str) {
    let shell = editor_document::wrap_document(inner_html, presentation());
    view.load_html(&shell, Some(EDITOR_BASE_URI));
}

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

/// The scheme comes from libadwaita: a web view resolves `prefers-color-scheme` from its own
/// settings, which left the surface white inside a dark app.
fn presentation() -> editor_document::Presentation {
    editor_document::Presentation {
        dark: adw::StyleManager::default().is_dark(),
        ..editor_document::Presentation::default()
    }
}

/// The document paints the ground only once parsed; between documents the view would flash white.
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

/// Replaces the sheet in place: a reload would take the caret and the undo history with it
/// (FR-075).
pub fn restyle(view: &webkit6::WebView) {
    restyle_with(view, None);
}

fn restyle_with(view: &webkit6::WebView, flow: Option<&str>) {
    paint_ground(view);
    let mut css = editor_document::editor_css(presentation());
    if let Some(flow) = flow {
        css.push_str(flow);
    }
    // `textContent`, not `innerHTML`: a stylesheet is text, not markup.
    let script = format!(
        "(() => {{ const s = document.querySelector('style'); \
         if (s) s.textContent = {}; }})()",
        json_string(&css)
    );
    view.evaluate_javascript(&script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
}

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

pub const GROUND: &str = "--r-ground";

/// Hand-rolled: this crate has no JSON dependency and wants none for one function.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            // U+2028 and U+2029 terminate a line in JavaScript but not in JSON.
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

/// Not shared with `reader::view::hardened_settings`: the two profiles must be diffable at a
/// glance.
fn editing_settings() -> webkit6::Settings {
    let settings = webkit6::Settings::new();
    // The one difference: the host's script runs here; markup-borne script still does not.
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

/// A click on a link is an editing gesture (the caret moves into the text), so every link-clicked
/// navigation is refused.
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

type ChangedHandler = Box<dyn Fn(&Document)>;

type FormatWatcher = Box<dyn Fn(FormatState)>;

/// `bold` is true when the selection is inside `Strong`, not when a mode is armed: there are no
/// modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatState {
    pub bold: bool,
    pub italic: bool,
    pub heading_1: bool,
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
    /// The first `load_html` on a fresh `WebView` starts a WebKit web process; see `Editor::warm`.
    loaded: Cell<bool>,
    /// A script run before the document loads lands on the page being replaced.
    caret_end_pending: Cell<bool>,
    flow: RefCell<Vec<(&'static str, glib::WeakRef<gtk::Widget>)>>,
}

/// No colour is written here: the column's own are read from its probes at each restyle.
const FLOW_CSS: &str =
    "\nbody { padding-left: 0; padding-right: 0; line-height: 24px; }\np { margin: 0 0 12px 0; }\n";

/// The DOM is a working copy, never the record (ADR 0004 Q3): every edit is parsed back into the
/// canonical `Document`, and that total, narrowing parse is the sanitisation on the way out. Undo
/// is the document's own, never the widget's.
pub struct Editor {
    view: webkit6::WebView,
    state: Rc<EditorState>,
}

impl Editor {
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::with_coalesce(source, COALESCE)
    }

    /// A fixed window makes coalescing deterministic in tests instead of racing a wall clock.
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
                    heading_1: tokens.contains(&"heading_1"),
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

    pub fn widget(&self) -> &webkit6::WebView {
        &self.view
    }

    /// Each probe names a reader palette variable and a widget whose CSS `color` the host sets;
    /// read again whenever the scheme changes.
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
        // The style manager reports the change before the stylesheet that paints the probes is in
        // place: read them after the main loop turns.
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
        // A probe is dressed by the host's stylesheet only once in the host's window, and a draft
        // is loaded before its dialog is presented;
        // read again once a document has loaded on screen.
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

    fn restyle(&self) {
        restyle_with(&self.view, flow_css(&self.state).as_deref());
        paint_flow_ground(&self.view, &self.state);
    }

    pub fn load(&self, document: Document) {
        self.state.history.borrow_mut().clear();
        self.state.last_edit.set(None);
        self.state.caret_end_pending.set(false);
        self.seed(&document.editor_html());
        self.state.loaded.set(true);
        *self.state.document.borrow_mut() = document;
    }

    /// The first `load_html` starts a WebKit web process, which would otherwise cost the first
    /// composition a person writes (#1216).
    /// Does nothing once anything has been loaded, including a draft part-way through typing.
    pub fn warm(&self) {
        if self.state.loaded.get() {
            return;
        }
        self.load(Document::default());
    }

    pub fn is_warm(&self) -> bool {
        self.state.loaded.get()
    }

    pub fn document(&self) -> Document {
        self.state.document.borrow().clone()
    }

    pub fn connect_changed(&self, handler: impl Fn(&Document) + 'static) {
        self.state.changed.borrow_mut().push(Box::new(handler));
    }

    /// Called only on change, never per keystroke.
    pub fn connect_format_state(&self, handler: impl Fn(FormatState) + 'static) {
        self.state
            .format_watchers
            .borrow_mut()
            .push(Box::new(handler));
    }

    pub fn undo(&self) {
        let mut document = self.state.document.borrow().clone();
        if self
            .state
            .history
            .borrow_mut()
            .undo(&mut document)
            .is_some()
        {
            self.show(document);
        }
    }

    pub fn redo(&self) {
        let mut document = self.state.document.borrow().clone();
        if self
            .state
            .history
            .borrow_mut()
            .redo(&mut document)
            .is_some()
        {
            self.show(document);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.state.history.borrow().can_undo()
    }

    /// `None` when there is no caret, or it is in a carried quote.
    pub fn caret(&self, done: impl FnOnce(Option<DocPosition>) + 'static) {
        self.view.evaluate_javascript(
            "window.postioMarkedBody()",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |outcome| {
                let marked = outcome
                    .map(|value| value.to_str().to_string())
                    .unwrap_or_default();
                done(postio_body::markdown::take_caret(parse(&marked)).1);
            },
        );
    }

    /// One edit on the record: a return from the Markdown view is one undo step however much was
    /// written there (FR-028).
    pub fn show_at(&self, document: Document, at: Option<&DocPosition>) {
        let before = self.state.document.borrow().clone();
        self.state
            .history
            .borrow_mut()
            .record(snapshot_step(&before, &document), Join::New);
        self.state.last_edit.set(None);
        let marked = at.map_or_else(
            || document.clone(),
            |at| postio_body::markdown::put_caret(&document, at),
        );
        self.seed(&marked.editor_html());
        *self.state.document.borrow_mut() = document;
        let current = self.state.document.borrow();
        for handler in self.state.changed.borrow().iter() {
            handler(&current);
        }
    }

    #[doc(hidden)]
    pub fn test_caret(&self) -> Option<DocPosition> {
        self.wait_ready();
        let marked = self.run_blocking("window.postioMarkedBody()");
        postio_body::markdown::take_caret(parse(&marked)).1
    }

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

/// The DOM has no caret in document terms, so a step's selections are the document's start.
fn snapshot_step(before: &Document, after: &Document) -> EditStep {
    EditStep::between(before, after, Selection::default(), Selection::default())
}

/// A free function over the shared state: the script-message handler holds only the `Rc`, so
/// dropping the editor drops the state once WebKit lets go.
fn absorb(state: &Rc<EditorState>, html: &str) {
    let now = Instant::now();
    let within_run = state
        .last_edit
        .get()
        .is_some_and(|last| now.duration_since(last) < state.coalesce);
    if !within_run {
        state.history.borrow_mut().seal();
    }
    absorb_with(state, html, Join::Run);
}

/// A markdown conversion must be something one undo can return to (FR-070): coalescing it into the
/// typing run would make undo take back the whole sentence.
fn absorb_as_new_step(state: &Rc<EditorState>, html: &str) {
    absorb_with(state, html, Join::New);
}

fn absorb_with(state: &Rc<EditorState>, html: &str, join: Join) {
    let after = parse(html);
    let before = state.document.borrow().clone();
    if after == before {
        return;
    }

    {
        state
            .history
            .borrow_mut()
            .record(snapshot_step(&before, &after), join);
    }
    state.last_edit.set(Some(Instant::now()));

    *state.document.borrow_mut() = after;
    let current = state.document.borrow();
    for handler in state.changed.borrow().iter() {
        handler(&current);
    }
}

impl Editor {
    fn run(&self, script: &str) {
        self.view
            .evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }

    /// Test plumbing: the one caller allowed to pump the main context from where it stands.
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

    pub fn place_caret_start(&self) {
        self.run(
            "const r = document.createRange(); \
             r.selectNodeContents(document.body); r.collapse(true); \
             const s = window.getSelection(); \
             s.removeAllRanges(); s.addRange(r);",
        );
    }

    /// Waits for the document being loaded when one is on its way.
    pub fn place_caret_end(&self) {
        if self.view.is_loading() || !self.state.loaded.get() {
            self.state.caret_end_pending.set(true);
        } else {
            run_caret_end(&self.view);
        }
    }

    /// The body is editable once parsed, before `editor.js` runs; wait for the marker the script
    /// sets last (#1716).
    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            let ready = self.run_blocking(
                "document.body && document.body.isContentEditable \
                 && window.postioEditorReady === true ? '1' : '0'",
            );
            if ready == "1" {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[doc(hidden)]
    pub fn test_type(&self, text: &str) {
        self.wait_ready();
        let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
        self.run_blocking(&format!(
            "(() => {{ document.execCommand('selectAll'); \
               document.execCommand('insertText', false, '{escaped}'); \
               return 'typed'; }})()"
        ));
        // The edit report crosses the bridge asynchronously; wait it out so a type-then-read test
        // does not race it.
        let wanted = text.trim().to_owned();
        let deadline = Instant::now() + Duration::from_secs(120);
        while self.document().to_text().trim() != wanted && Instant::now() < deadline {
            while gtk::glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[doc(hidden)]
    pub fn test_eval(&self, script: &str) -> String {
        self.run_blocking(script)
    }

    #[doc(hidden)]
    pub fn test_select(&self, nth: u32, from: u32, to: u32) {
        self.wait_ready();
        // A TreeWalker: formatting splits text nodes, so the `nth` text node is not at a fixed
        // depth.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Heading1,
    BulletList,
    NumberedList,
    QuoteBlock,
}

impl Editor {
    /// Each fires an `input` event, so the edit crosses the bridge like any keystroke and lands on
    /// the same history.
    pub fn format(&self, format: Format) {
        // WebKit dispatches `input` for some editing commands and not others (bold yes,
        // insertUnorderedList no), so it is dispatched here; a duplicate is absorbed.
        let command = match format {
            Format::Bold => "document.execCommand('bold');",
            Format::Italic => "document.execCommand('italic');",
            Format::Heading1 => {
                "if (document.queryCommandValue('formatBlock') === 'h1') { \
                     document.execCommand('formatBlock', false, 'p'); \
                 } else { \
                     document.execCommand('formatBlock', false, 'h1'); \
                 }"
            }
            Format::BulletList => "document.execCommand('insertUnorderedList');",
            Format::NumberedList => "document.execCommand('insertOrderedList');",
            Format::QuoteBlock => {
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

    /// Anything but http, https and mailto is refused here rather than silently dropped by the
    /// parse later.
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

    /// The `src` is built from a `postio_body::ContentId`, so only an id that satisfied its rules
    /// can reach the DOM.
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
        // A paste can be the first gesture into a fresh body; `insertHTML` does nothing without a
        // caret, so fall back to the end.
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
