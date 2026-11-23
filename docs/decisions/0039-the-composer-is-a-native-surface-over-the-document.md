# ADR 0039 — The composer is a native surface over the document

- **Status:** Accepted (2026-09-18). Not built: the composer still edits in a WebKitGTK view (`postio-widgets` `composer/`). Replaces ADR 0003 Q2 (`contenteditable`); the rest of ADR 0003 stands.
- **Decision:** The composer's editing surface becomes a native text view over `postio_body::Document` (`GtkTextView` in `postio-widgets`, `NSTextView` in the Swift frontend). The editing algebra moves into `postio-body` as pure functions on `Document`. No `contenteditable`, no bundled script, no script bridge, no browser engine in the compose path.

## Rules
- `postio-body` gains `EditIntent` and `apply(&Document, EditIntent) -> Document`, beside `EditHistory`. It stays a pure leaf. `Document` is six block kinds and seven inline kinds (ADR 0004 Q2); every edit is a total function over that set, tested at the `--lib` tier.
- The buffer is a projection of `Document`. It never holds the record (ADR 0004 Q3). Nesting lives in `Document`; the buffer renders margins and bullets.
- Paste normalises through `postio_body::parse`.
- Inline images are `GtkTextChildAnchor` holding a `GtkPicture` over blob bytes.
- Spell-check is `libspelling`'s adapter on the buffer. Accessibility is `GtkAccessibleText`.
- `postio_ui::editor::markdown` stays an input convenience, not a source format. The WYSIWYG decision (ADR 0003) is not reopened.
- ADR 0003's hardening requirements 1, 3 and 4 hold vacuously. Quoted content is sanitised before insertion, output is parsed into `Document`, and replies carry nothing outward.
- Removing the WebView composer deletes the WebKit editor (`composer/editor.rs`), the script-message handlers, `evaluate_javascript` formatting, `editable_dialect.rs` and webkit6. One surface, never two behind a flag.
- Sequence: (1) `apply` in `postio-body`, proven without a UI; (2) route the existing composer's edits through it; (3) build the `GtkTextView` surface beside it; (4) delete the WebView in one commit. Steps 1 and 2 stand alone.

## Why
- ADR 0003's reasons for `contenteditable` are mostly uncollected: native undo is off (ADR 0004 Q5), spell-check was never enabled, and the markdown dialect is re-implemented in Rust and generated into the script.
- A text widget is not a second HTML engine. The WebView costs about 408 MB of helper processes and a second GL surface, keeps the script posture, and blocks macOS compose (ADR 0019). The algebra ports because Swift links `postio-body`.
- Rejected: a Blitz composer (no `contenteditable`; a rich-text engine from scratch), keeping WebKit for the composer, `GtkTextBuffer` as the record, third-party rich-text frameworks.
- This removes the last WebKitGTK link from the Linux app.

## Revisit when
- The buffer-offset to `Document` mapping is unstable under IME preedit. Spike typing CJK into a list item before step 3 is locked.
- `libspelling` fights a re-rendered buffer.

## Waiting on this
- Spec 014 Part B: the rich surface's typed sequences (its `SEQUENCES` table is the contract), drawing every construct, the new toolbar buttons, the formatting, table and code-language pickers, Paste as Markdown, the Download and attach chip, and `line_break`. Until then GTK and the Mac author those constructs in the Markdown source view. Tables in a native text view need a spike beside the IME.
