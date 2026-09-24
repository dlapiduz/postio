# Data model: Postio in the terminal

Phase 1 for [plan.md](./plan.md). Only what is new or changed; Postio's domain
types (`postio-model`) are unchanged.

## Persisted

### `drafts.body_markdown` (new column)

| Field | Type | Meaning |
|---|---|---|
| `body_markdown` | `TEXT NULL` | The Markdown the user typed in the terminal composer, exactly. NULL when the draft was last saved by a frontend that does not author Markdown (GTK, macOS). |

- **Written by**: the terminal composer, together with `body_text` (the same
  Markdown, the text part as it will be sent, FR-021) and `body_html` (the
  generated HTML part), in one write.
- **Cleared by**: any save from the GTK composer, which writes NULL, so a draft
  edited in GTK reopens in the terminal from its HTML.
- **Read by**: the terminal composer on reopen. It uses `body_markdown` when
  it is non-NULL, and otherwise `markdown::from_document(parse(body_html))`.
- Existing rows take NULL. No migration beyond the column (no backwards
  compatibility).

### Remote-image allowlist (moved, reformatted)

It was a GTK `KeyFile` at `$XDG_STATE_HOME/postio/remote-images.ini`
(`crates/postio-gtk/src/reader/allowlist.rs:28`). It becomes a host-owned
TOML file at the same directory, `remote-images.toml`, read and written only
by the daemon. Fields: a list of sender addresses allowed to load remote
images. An existing `.ini` is not read.

### `config.toml` additions

| Key | Type | Default | Meaning |
|---|---|---|---|
| `[tui.colors].<role>` | colour name or `#rrggbb` | terminal palette | Overrides one colour role (see contracts/tui-surface.md §Colour roles) |
| `[tui].preview` | `"split" \| "toggle"` | `"toggle"` | How the composer shows the rendered preview |
| `[tui].mouse` | bool | `true` | Capture the mouse (off leaves the terminal's own selection) |

Keys stay under `[keys]` keyed by command id, shared with GTK. There is no
terminal keymap section.

## Runtime (not persisted)

### Host

The one process that owns the store.

- `wiring: postio_session::Wiring`
- `clients: Map<ClientId, ClientState>`
- `notifier: Option<ClientId>`: the elected client that delivers the next
  notification (R1c)
- **Lifecycle**: `Starting` (store opening) → `Serving` → `Draining` (no
  clients; grace timer running) → `Stopped`. A new client during `Draining`
  returns it to `Serving`. `Stopped` releases Turso's lock.

### ClientState

| Field | Meaning |
|---|---|
| `id` | Assigned on handshake |
| `kind` | `Gtk \| Tui \| Ffi \| Test`: used for notifier election and the diagnostic label |
| `build` | Exact build id; must equal the host's, or the handshake is refused |
| `undo: UndoStack` | Per-client (R1a); coalescing 1 s and expiry 600 s as today |
| `events` | That connection's `EventHub::subscribe("client:<kind>:<id>")` stream |

### Terminal session

The running `postio-tui`, which holds no mail of its own.

| Field | Meaning |
|---|---|
| `caps: TerminalCaps` | `colour: None \| Ansi16 \| Ansi256 \| TrueColor`, `keyboard_enhancement: bool`, `mouse: bool`, `background: Light \| Dark \| Unknown`, reserved `graphics: Option<Protocol>` (next iteration) |
| `size` | Columns and rows; drives which panes are shown |
| `layout` | Which panes are *requested* vs *shown* (ADR 0024's split: the window decides what is shown, never what was asked for) |
| `focus` | Sidebar, List, Reader, Composer, Palette, Search, Dialog |
| `list: ListWindow<Row>` + `selection: SelectionState` + cursor | From `postio-ui`, unchanged |
| `reader: Option<RenderedConversation>` | |
| `composers: Vec<Composer>` | One is in the reading pane; others are tabs (the terminal's pop-out, FR-003) |

### RenderedConversation / RenderedMessage

The reader's model: styled lines built once per message and width, then
windowed for drawing.

| Field | Meaning |
|---|---|
| `message_id` | |
| `header` | From `postio_ui::reader::header::MessageHeader`, sanitised |
| `blocks: Vec<RBlock>` | `Lines(Vec<StyledLine>)`, `Fold { id, summary, folded: bool, inner }`, `Image { identity, alt, bytes_len, reserved: (w, h) }`, `Attachment { part, name, size }` |
| `links: Vec<LinkSpan>` | Position, visible text, full target (FR-014) |
| `held_back` | Counts of blocked remote images and trackers, from `Sanitized` |
| `width` | The width these lines were built for; a resize rebuilds lazily |

**Invariant**: every string in a `StyledLine` passed through
`postio_ui::terminal::sanitize`. The type makes that the only constructor
(`SafeText`).

### Composer

| Field | Meaning |
|---|---|
| `draft_id` | Existing draft identity |
| `headers` | To, Cc, Bcc (shown on demand), Subject, identity |
| `markdown: TextArea` | The user's text |
| `quoted: Option<Quoted>` | The opaque reply or forward quote, shown read-only and foldable, kept or removed whole |
| `attachments: Vec<AttachmentRef>` | Blob id, name, size, disposition (`Attachment \| Inline { content_id }`) |
| `preview_visible` | |
| `dirty_since` | For the autosave debounce (1500 ms, as GTK) |

**Derived on save and send**:
`document = to_document(markdown) ++ quoted`, then
`html = render(document).1` (omitted when `document.is_plain_text()`), then
`text = markdown ++ quoted.text() with "> "`.

### PasteClassification

The output of `postio_ui::paste::classify(text)`, a pure function.

- `items: Vec<PasteItem>`, where `PasteItem = File(PathBuf) | Unreadable { path, reason } | Text(String)`
- A token becomes `File` only if it resolves to an existing, readable regular
  file. It is `Unreadable` if it clearly names a path (`file://`, absolute,
  `~/`) that fails, and `Text` otherwise.

### Colour roles

`Text`, `Dim`, `Accent`, `Selection`, `Focus`, `Unread`, `Flagged`, `Link`,
`Quote`, `Code`, `Error`, `Warning`, `Success`. Each resolves as follows:
config override, else the true-colour accent (only `Selection` and `Focus`),
else an ANSI palette index, else, under `NO_COLOR`, an attribute (bold,
reverse, underline, dim).
