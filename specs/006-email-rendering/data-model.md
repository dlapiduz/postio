# Data Model: Faithful, Readable Email Rendering

Almost nothing here is persisted. The only stored value this feature adds is
`reader.zoom` in `config.toml`. Everything else is an in-memory structure,
and its "validation rules" are the invariants tests hold it to. Which crate
owns each entity is part of the model, because the boundaries are enforced
(Principle VII).

---

## Sanitized message (`postio-body`)

This is what the sanitizer produces for one message. It is shared by every
frontend and by reply quoting (ADR 0033).

| Field | Meaning |
|---|---|
| `html` | The body, contained under `div.postio-body[data-postio-message=<scope>]` |
| `scoped_css` | The sender's `<style>` rules, rewritten under the container (`styles.rs`) |
| `canvas` | `Canvas` lifted from `<body>`/`<html>` (see below) |
| `color_scheme` | The sender's declared `color-scheme` (`light`, `dark`, `light dark`), carried from `<meta>`, or none |
| `refusals` | `Vec<Refusal>`: what was removed, and why |
| `resource_refs` | Every `cid:`, `data:` and `http(s)` resource the body names, with its declared size |
| `over_cap` | `Option<Cap>`: set when an input cap (R6) was exceeded, so the caller renders the plain-text alternative |

**Invariants.**
- A sender `class` or `id` that begins with `postio-` never survives.
- Every `id` is unique per message, and fragment links follow it.
- No `<svg>`, `<script>`, `<iframe>`, `<object>`, `<embed>`, `<form>`
  controls or `<title>` text survives.
- No remote URL survives unless consent was given.
- Every removal appears in `refusals`.

### Canvas

| Field | Meaning |
|---|---|
| `background` | The colour from `bgcolor`, `style="background(-color)"` or a `body`/`html` rule. `None` means the sender set none |
| `text` | The colour from `text=` or `color:` on the page |
| `link` | The colour from `link=`, as a scoped `a` rule |

### Refusal (the refusal list, spec Key Entity)

| Field | Meaning |
|---|---|
| `what` | `Element(name)`, `Attribute(name)`, `Property(name)`, `AtRule(name)`, `Resource(kind)` |
| `reason` | `Containment`, `Privacy` or `NoScript`. There is no other variant: "easier" is not a reason (001 FR-019b) |

The table of refusals is `const` and walked by a test. It extends
`REFUSED`, `REFUSED_AT_RULES` and `REFUSED_UNITS`, which already exist, to
elements and attributes.

---

## Composed document (`postio-ui`)

This is unchanged in shape: `document_for` / `conversation_document` still
compose one document per conversation, with one container per message
(001 T019–T021).

It changes in two ways:
- Nothing in it depends on WebKit. The CSP `<meta>` stays, harmless and still
  used by the macOS WKWebView. `::-webkit-details-marker` goes.
- `Sheet` no longer defaults bulk mail to Reader view (R16).

---

## Resource table (`postio-render`)

This is the closed set of resources one render may use (spec FR-003, Key
Entity). It is built by the caller before the render, and it is immutable
during it.

| Entry | Key | Source |
|---|---|---|
| Inline part | `(scope, content-id)` or `(scope, content-location)` | the message's own MIME parts, via `BlobSource::resolve_in` |
| Embedded image | the `data:` URI itself | the markup |
| Remote image | `(scope, url)` | only the `RemoteImageFetcher` cache, only for consented senders |
| Bundled font | `postio-font:<face>` | ADR 0023's `FACES` |
| System font | family name | `fontdb` discovery (R3), registered once per process |

**Invariants.**
- A lookup outside the table resolves to nothing and increments
  `counts.unresolved`. It never errors, and it is never silent.
- An entry for scope A is never returned to a reference from scope B
  (FR-004).
- Raster entries were probed: format allowed, and dimensions and bytes within
  R4's limits. An entry that failed the probe is stored as
  `Placeholder { width, height }`.
- SVG entries are re-serialised with only `data:` raster references kept
  (R5).

---

## Render request (UI thread → render thread)

| Field | Meaning |
|---|---|
| `generation` | Monotonic per reader. A result with a stale generation is dropped |
| `document` | The composed HTML |
| `resources` | The resource table |
| `viewport` | `width_px` (logical), `hidpi_scale` (fractional surface scale), `zoom` (a step) |
| `theme` | `Theme { dark: bool, high_contrast: bool, palette: ReaderPalette }` |
| `darkened` | The set of message scopes the user asked to darken (FR-013a) |
| `open_folds` | The `<details>` the user has toggled, by stable fold id |
| `reader_view` | The set of message scopes shown reduced (R16) |

---

## Rendered document (render thread → UI thread)

This is immutable and `Send` (R7), and it is the only thing the widget reads.

| Field | Meaning |
|---|---|
| `generation` | Copied from the request |
| `size` | The laid-out width and height in CSS px at this zoom |
| `display_list` | `anyrender::recording::Scene`, recorded once and rasterised per tile |
| `low_res` | The whole document rasterised at 0.25 scale, capped at 16 MiB. It is the blank-frame fallback |
| `text` | `TextIndex` (below) |
| `links` | `Vec<LinkBox { rect, target: LinkTarget }>` |
| `messages` | `Vec<MessageBox { scope, rect, presentation }>`, in document order |
| `folds` | `Vec<FoldBox { id, summary_rect, open }>` |
| `counts` | `RenderCounts` (below) |
| `outcome` | `Rendered`, or `FellBack { reason }` (panic, deadline, over cap). On fallback the document is the plain-text alternative, rendered with the reader palette |

### TextIndex

This is the one serializer shared by copy, find and accessibility (R7).

| Field | Meaning |
|---|---|
| `text` | The whole document's text in reading order: tabs between cells, newlines at rows and blocks, image `alt` included, hidden content excluded |
| `clusters` | `Vec<Cluster { range: Range<usize> (chars into text), rect, scope, color, painted_ground, large: bool }>` |

**Invariants.**
- Clusters are sorted by `range.start` and are non-overlapping.
- Every visible glyph belongs to exactly one cluster.
- `painted_ground` is the colour R10's ancestor walk computed. For every
  cluster, `contrast(color, painted_ground) ≥ floor(theme, large)`, where the
  floor is 4.5 or 3 normally and 7 or 4.5 in high contrast. SC-001's pixel
  test checks this claim against the raster.

### Presentation (per message, spec Key Entity "theme adaptation rule")

| Variant | When (R10) | Canvas | Text colours |
|---|---|---|---|
| `Styled` | the light theme | the sender's canvas, or the reader ground | as authored, repaired to the floor |
| `SenderDark` | dark, and the sender declares dark support | as authored under the dark scheme | as authored, repaired to the floor |
| `Paper` | dark, and Designed | a card of the sender's canvas, or white | as authored, repaired to the floor |
| `Darkened` | dark, Designed, and the user asked | backgrounds remapped into OKLab L ∈ [0.12, 0.30] | repaired to the floor |
| `Adapted` | dark, and neither of the above | the reader ground | repaired to the floor |

**Transitions.**
- `Paper ⇄ Darkened` only by the darken command, per message, and not
  persisted.
- A theme change re-classifies every message.
- Nothing else changes a presentation.

### RenderCounts (Principle V)

These are the counts the budgets are gated on, not timings:
- `renders`
- `style_passes`, which is at most 2: the first pass, plus the repair
  restyle
- `nodes`
- `text_clusters`
- `repaired_runs`
- `resources_resolved`
- `resources_unresolved`
- `images_placeholdered`
- `display_list_commands`

They join the render counter in `postio-ui/src/test_support/` (001 T029/T030)
next to `renders issued`, `surfaces created` and `bytes per document`.

---

## Selection and find (UI thread, over `TextIndex`)

| Entity | Fields | Invariants |
|---|---|---|
| `Selection` | `anchor: usize`, `focus: usize` (chars into `TextIndex.text`) | `copy()` returns `text[min..max]` exactly |
| `FindState` | `query`, `matches: Vec<Range<usize>>`, `current: Option<usize>` | Matching uses a case- and diacritic-folded copy of `text`, with an offset map back to it. Highlights are rects from the clusters that intersect each match |

Both survive a re-render at the same generation lineage (a zoom or theme
change) by char offsets, because the text is unchanged (FR-021f).

---

## Tile cache (UI thread, `postio-gtk`)

| Field | Meaning |
|---|---|
| `tile_height` | 512 logical px |
| `scale` | The surface's fractional scale × zoom |
| `tiles` | An LRU of `(generation, index) → gdk::Texture` |
| `budget` | 64 MiB per reader. The sum of tile bytes never exceeds it |

**Invariant.** The bytes held are bounded by the budget plus `low_res`,
independently of `size.height` (FR-022, SC-006).

---

## Reader preferences (persisted, `postio-config`)

```toml
[reader]
zoom = 100   # percent; one of 50 67 75 80 90 100 110 125 150 175 200 250 300
```

- A value off the step list clamps to the nearest step on load. It does not
  error: the config file is user-edited.
- The default is 100.
- It is added through the six-edit path `sender_avatars` used: the field with
  a serde default, `patch_*`, the `docs/config.md` drift entry, a settings
  control, the FFI mirror, and `ConfigChanged`.

No other state from this feature is persisted. Darken, open folds, reader
view and "show once" images all live only as long as the session.

---

## Remote image cache (`postio-runtime`)

| Field | Meaning |
|---|---|
| key | the URL |
| value | `Fetched { bytes, format }`, or `Failed { reason }` |
| lifetime | the process. Never written to disk |

**Invariant.** An entry exists only for a URL named by a message whose sender
was `Allowed`, or that was shown once, at the time of the fetch.
