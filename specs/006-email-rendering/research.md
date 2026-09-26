# Research: Faithful, Readable Email Rendering

**Feature**: `specs/006-email-rendering` | **Date**: 2026-09-26

Four investigations fed this document. Each is summarised where it is used,
not kept separately:

- **The Blitz spike.** `spike/blitz-reader`, #1543 and #1547.
- **Blitz today.** The published crates and upstream `main`, with a C and
  network audit over `cargo tree` of the spike branch.
- **Postio's reader today.** An integration map of `postio-body`,
  `postio-ui`, `postio-gtk`, the registry, the config and the checks.
- **Platform mechanics.** Accessibility, selection, find, tiling, zoom,
  dark-mode colour adaptation, and containment, read from the crate sources in
  the local registry (gtk4 0.11.4, blitz-* 0.3.0-beta.2, anyrender 0.13).

Every decision has the same three parts: **Decision / Rationale /
Alternatives**. **R0 comes first and is open:** the engine is chosen by an
evaluation, and R1–R8 describe the Blitz arm until it concludes. No question from the Technical Context is left open. The
things that remain uncertain are **Risks**, and each one has a named task
that retires it first.

---

## R0 — The engine is decided by an evaluation, not assumed

**Status: open. It gates every engine-specific task** (maintainer,
2026-09-26: *"we still need to evaluate whether we should use blitz or
webkit"*).

R1 onward describe the **Blitz** path. That path came from the spike, and
the spike never measured WebKit on the same terms. So the engine is chosen by
a like-for-like evaluation, run after the engine-neutral work (the corpus,
the reference renders, the metric, and the sanitizer) and before any
engine-specific work.

### The two arms

| | **A — WebKit (improved)** | **B — Blitz** |
|---|---|---|
| Engine | WebKitGTK 6, the shipped hardened `Reader`, one document per conversation (ADR 0032) | `postio-render` as R1–R8 describe, at prototype depth |
| Input | the **same** production pipeline: the Phase 2 sanitizer (classes, the canvas, the hints), then `postio-ui` compose | the same |
| Dark mode | a prototype of R10's rule (classification + contrast repair) in Postio's isolated-world script, the mechanism 001 R3 already allows | a prototype of R10 over Blitz computed styles |
| Affordances | WebKit's own: selection, find (`FindController`), accessibility, zoom (`zoom-level`), printing | the R7/R8 build list (US4 + US6 tasks) |

Both arms are measured by the same instruments on the same machine. Each arm
is a harness under `examples/`, not product code. The losing arm is deleted.

### Criteria

**Gates.** An arm that fails one of these cannot be chosen until the failure
is shown to be fixable within this branch:

| | Gate | How it is measured |
|---|---|---|
| G1 | **Legible (SC-001)**: with the arm's dark-mode prototype, zero text runs below the floor across the theme fixtures, in dark and high contrast | pixel sampling behind every text run (R14), with text-run rects from each engine (A: `Range.getClientRects`; B: the text index) |
| G2 | **No egress (SC-003)**: zero connections across the hostile corpus, unconsented and consented | a loopback listener with a counted control (#1336 discipline) |
| G3 | **Survives hostile mail (SC-004)**: no crash, and no hang past the deadline | the hostile fixtures, each opened in a live reader |

**Scored.** Reported side by side. None of these alone decides:

| | Criterion | Measure |
|---|---|---|
| S1 | Fidelity | `contracts/fidelity-metric.md` over the designed fixtures. **Known bias:** the reference *is* WebKit, so arm A's score measures only sanitizer loss, and arm B's measures sanitizer plus engine loss. The question for B is "≥ 95%?", not "higher than A?" |
| S2 | Cost | web/renderer Pss, process count, first and warm render time, and conversation handover at 2, 10 and 50 messages, using `postio-app`'s `pane_comparison` example (the #1348 method) |
| S3 | Blank frames (FR-029) | frames showing only the ground colour during 50 message-to-message navigations, captured from the widget |
| S4 | Affordance parity | which of US4 and US6 each arm has today, and the task count to close the gap (from `tasks.md`) |
| S5 | Accessibility quality | the body's text, links and headings as a screen reader sees them (accerciser/AT-SPI walk), per arm |
| S6 | Security posture | code that parses hostile input (C/C++ vs memory-safe), process isolation, and whether "no network" is structural or a setting |
| S7 | Maintenance risk | upstream maturity and release cadence; how security updates reach users (distro WebKit vs pinned crates); open upstream gaps (R1) |
| S8 | Platform reach | macOS already renders with WKWebView (ADR 0019); what each arm implies for a shared engine later |

### The decision rule

- **The maintainer decides.** The agent writes the scorecard and a
  recommendation into `docs/notes/<date>-blitz-or-webkit.md`. The note is
  listed in `docs/engineering-notes.md`, and the decision is recorded in
  `spec.md`'s Clarifications.
- **If both arms pass the gates,** the recommendation weighs S1, S3 and S6
  against S2, S4 and S7.
- **Written before either arm runs.** This rule, the criteria and the
  fixtures are committed before any result exists, so the goalposts cannot
  move to fit a result.

### Requirements that assume an engine, and what happens to them

| Requirement | As written (Blitz) | If WebKit is chosen |
|---|---|---|
| FR-001 | no network by construction | amended: network blocked by the engine's settings and CSP, proven by egress tests; the structural claim is withdrawn |
| FR-002 | no script, Postio's own included | amended: sender script refused; Postio's isolated-world script allowed (001 R3) |
| FR-023a | in-process, memory-safe parsers | replaced: rendering in WebKit's sandboxed web process |
| FR-027 / FR-028 | one engine, one surface | unchanged, and already true of WebKit (ADR 0032) |
| SC-006 | no separate rendering process | amended: one shared web process, with memory bounded as #1348 measured |
| R3, R5–R8, contracts `renderer-api.md` and `renderer-graph-checks.md` | Blitz-specific | dropped; `/speckit-plan` re-run for the engine-specific phases |
| US4 / US6 tasks | built | reduced to wiring WebKit's own features to the registry and tests |

Either way, the engine-neutral work stands: R9 (sanitizer), R10 (the
classification rule and the floor), R12 (the app-side fetcher), R13
(references), R16 (Reader view opt-in), the corpus and the metric.

---

## R1 — The engine and how we take it

**Decision.** Use Blitz: `blitz-dom`, `blitz-html`, `blitz-paint` and
`blitz-traits`, all **`=0.3.0-beta.2`**, plus `anyrender =0.13` and
`anyrender_vello_cpu =0.17`, from crates.io with exact pins. No git
dependency and no fork up front.

The behaviour Blitz lacks is closed **on Postio's side**, before the markup
reaches Blitz (R9). A `[patch.crates-io]` fork of **`blitz-dom` only**, pinned
to a git revision, is allowed later for a gap that cannot be closed that way.
It MUST be recorded in this file with the upstream PR it tracks.

**Rationale.**
- **The spike's evidence.** All nine corpus HTML fixtures and all eight legacy
  fixtures laid out correctly once the viewport was set.
- **0.3 final is not out.** Blitz `main` pins `parley` by git revision, so
  taking `main` means taking a git dependency tree. The published beta
  resolves cleanly.
- **The spike's "vendor it" note is partly stale.** Beta.2 already maps
  `width`, `height`, `bgcolor`, `align` (as `text-align`), `hspace`/`vspace`
  and the `body` margin attributes (PR #662). The email gaps that remain are
  `valign` (#508), `cellpadding`/`cellspacing` (PR #503), `<table border>`,
  `table align` as margins, and `<font>`.
- **All of those are attribute-to-style rewrites.** A rewrite in
  `postio-body` is a pure function that is proven red in milliseconds. It also
  benefits the macOS and TUI frontends, which read the same sanitized
  document.

**Alternatives.**
- **Vendor all of Blitz.** Rejected. It takes on a browser engine's
  maintenance for five attribute mappings.
- **Track `main` by git.** Rejected until parley publishes.
- **Keep WebKit and fix dark mode in CSS.** Rejected. It keeps the ~408 MB web
  process and the black inter-document frame. It also keeps privacy as a
  matter of eighteen settings rather than of what is linked (spec FR-001), and
  the spec requires one engine (FR-027).

**Upstream movement worth taking at 0.3 final:**
- relayout reworked into a damage pre-pass (#919, #921);
- stylo 0.21 (#851);
- `nowrap` on cells (#931);
- `line-height: normal` from font metrics (#878);
- `lang` passed to parley (#932);
- CSS `direction` (#928);
- fixed table layout (#852).

None of them is a prerequisite.

---

## R2 — Where the renderer lives

**Decision.** A new workspace crate, **`postio-render`**. It has no GTK, no
network and no C. It takes a composed HTML document plus a **resource table**
and produces a **rendered snapshot** (R7). `postio-gtk` depends on it and
supplies only a widget. `postio-ui`, the FFI and the TUI do **not** depend on
it.

**Rationale.**
- **Principle VII.** A boundary is a check, not a habit. A crate is the unit
  `check-crate-boundaries.py` guards, and the unit whose dependency graph R3
  can prove disconnected. Inside `postio-gtk`, the renderer's graph would be
  mixed with GTK, WebKit (the composer still uses it, per ADR 0039's pending
  migration) and GLib's networking.
- **It is headless.** Layout, pixels and contrast can be asserted in plain
  `cargo test` with no display. That removes the wall 001's plan hit, where
  *"every `getBoundingClientRect` is zero"* on the suite's display. Rendering
  geometry becomes the cheapest layer that can fail, which is where the
  constitution wants rules proven.
- **macOS stays possible.** The macOS frontend reads `postio-ui`'s document
  and renders it in a WKWebView (`postio-ffi/src/session.rs:2187`). A
  GTK-free renderer could serve it later, and nothing here forces that.

**Alternatives.**
- **The renderer in `postio-gtk`, as the spike did.** Rejected: it cannot be
  proven disconnected, and its tests need a display.
- **In `postio-ui`.** Rejected. It would push Blitz's roughly 226-crate graph
  into the FFI and TUI builds, which render differently or not at all.

---

## R3 — Proving "cannot reach the network" and "no C on the content path"

**Decision.** Two mechanical guards, both run by `scripts/check.sh`.

1. **`check-crate-boundaries.py` gains `RULES["postio-render"]`.** It uses the
   existing mechanism, a breadth-first `cargo metadata` walk, over **normal
   and build edges only**. The renderer's own dev-dependencies never ship,
   and its tests legitimately need a socket and `postio-test-support`
   (`tokio`). It bans:
   - GTK and WebKit: `gtk4`, `gtk4-sys`, `webkit6`, `webkit6-sys`, `glib`,
     `gio`;
   - Postio's network and storage crates: `postio-transport`, `postio-sync`,
     `postio-runtime`, `postio-storage`, `io-http`, `pimalaya-stream`;
   - Blitz's own networking: `blitz-net`, `blitz`;
   - network and TLS crates: `reqwest`, `hyper`, `h2`, `ureq`, `curl`,
     `isahc`, `surf`, `rustls`, `native-tls`, `openssl`, `socket2`, `mio`,
     `tokio`, `async-std`.

   Its `why` cites spec FR-001 and FR-023a.
2. **A new `check-renderer-is-memory-safe.py`.** It walks the same resolved
   graph from `postio-render` with resolved features, and fails when any of
   these is true:
   - a package declares `links` outside an allowlist (`rayon-core`,
     `servo_style_crate`, which are uniqueness markers, not native links);
   - a package name ends in `-sys`;
   - `cc`, `cmake` or `bindgen` appears as a build dependency;
   - `image` has `avif-native` (dav1d, which is C) or `avif` (its
     `nasm`/`ravif` path) enabled;
   - `blitz-dom` has `net` or `system-fonts` enabled;
   - `fontique` has `system` enabled.

   The check names its fix when it fails, as every check here does.

**The audit (spike branch, 226 crates in the union of the renderer's
graphs).** The **only** C on the content path is `yeslogic-fontconfig-sys`.
It enters through `blitz-dom/system-fonts` → `parley/system` →
`fontique/system`. `fontique`'s fontconfig backend passes strings and
codepoints from the message into `FcFontSort`, and fontconfig parses font
files through FreeType.

Everything else is Rust:
- HTML: html5ever;
- CSS: stylo, cssparser, selectors;
- fonts: read-fonts and skrifa, with no FreeType;
- shaping: harfrust, not HarfBuzz;
- segmentation: ICU4X;
- SVG: usvg and roxmltree;
- rasterisation: vello_cpu.

Stylo's build script runs `python3` to generate code. That is a build-time
tool, not linked code, and it is allowed.

**Fonts without fontconfig.** Turn off `system-fonts`. Discover installed
font files with **`fontdb`** (already in the graph through usvg; its
fontconfig *configuration* parser is pure Rust), read their bytes, and
register them into a fontique `Collection`. Register the bundled faces
(ADR 0023's `FACES`) first. Generic families (serif, sans-serif, monospace)
and per-script fallbacks (Latin, CJK, Arabic, Hebrew, Devanagari, emoji) are
set explicitly from what `fontdb` found, with bundled faces as the floor.
Discovery runs once, off the UI thread, at first reader use. That satisfies
FR-009 without C.

**Why not a runtime socket test alone.** Observing no connection proves
nothing about a path that was not exercised. The graph check proves the path
cannot exist. A loopback-listener test is still kept (T-level) as the
belt-and-braces observation that SC-003 asks for.

**Alternatives.**
- **cargo-deny `bans` workspace-wide.** Rejected: `postio-sync`
  legitimately networks. The proof has to be rooted at one crate.
- **`fontconfig-dlopen`.** Rejected: it changes how fontconfig is linked, not
  whether C runs.

---

## R4 — Images: formats, and decode limits

**Decision.**
- **Formats.** `postio-render` depends on `image` directly with
  `default-features = false` and features `png`, `jpeg`, `gif` and `webp`.
  Those resolve to png, zune-jpeg, gif and image-webp, all Rust. Cargo feature
  unification turns them on for `blitz-dom`'s own `image` dependency.
- **Refused formats.** Not `avif` (its path pulls in assembly and C), `tiff`,
  `exr`, `bmp` or `ico`. Those are uncommon in mail and each is extra attack
  surface. An image in a refused format becomes a sized placeholder, and the
  refusal is counted.
- **Limits.** They are enforced in the **resource table**, before Blitz sees
  any bytes:
  - a header-only dimension probe (`image::ImageReader::into_dimensions`);
  - at most **8,192 px** on either side, **40 megapixels**, and **16 MiB**
    encoded;
  - animated GIFs render their first frame only; no animation (reduced
    motion, and cost).

  An over-limit image becomes a placeholder of its declared size (FR-024).

**Rationale.** The spike could not decode **any** raster image. `blitz-dom`
takes `image` with `default-features = false` and nothing in the spike's
graph enabled a format, so `ImageHandler::parse` failed on every `cid:` PNG.
Nine fixtures with no inline raster image hid it. #1501, "inline images reach
WebKit as one whole buffer with no ceiling", is the same risk in the current
reader.

**Alternatives.**
- **Decode limits via `image::Limits` inside Blitz.** Rejected: Blitz calls
  the decoder itself, and a limit it does not pass cannot be relied on.
- **Probing in the table.** Chosen: it is ours and testable.

---

## R5 — SVG must not read local files

**Decision.**
- **Inline `<svg>` in a message stays stripped** by the sanitizer, as it is
  today. The reason is recorded in the refusal list: *no-script* and
  *containment*. SVG carries its own script, animation and foreign-object
  surface, and mainstream webmail strips it too.
- **SVG as an image resource** (a `cid:` part of type `image/svg+xml`, or a
  `data:image/svg+xml` URI) is **re-serialised by the resource table**. It is
  parsed with `usvg` using an `image_href_resolver` that resolves only
  `data:` raster images and nothing else, then written back with usvg's
  writer. Blitz then parses a document that no longer names any external
  file.
- `blitz-paint`'s `svg` feature is turned on, so that these images actually
  paint. The spike had it on in `blitz-dom` and off in `blitz-paint`, so SVGs
  parsed and never drew.

**Rationale.** usvg's `default_string_resolver`
(`usvg-0.48.1/src/parser/image.rs:85`) calls `std::fs::read` on any
`<image href="/abs/path">`. `blitz-dom`'s `parse_svg_image` uses the default
`Options`. So an SVG in a message can pull a local file into the render and,
through its pixels, into anything that reads the snapshot. It never leaves
the machine, but it is still a read the user never asked for.

**Alternatives.** A `blitz-dom` fork that sets the resolver. That is
equivalent, but it costs a fork. Re-serialising in the table needs none.

---

## R6 — Threading, containment and the 400 ms bound

**Decision.** Each reader owns **one render thread**, built with a
**64 MiB stack**. That thread owns every Blitz `Document`: parse, style,
layout, recording the display list, and building the text index. The UI
thread never touches Blitz.

- **Requests.** Each request carries a **generation number**. A result whose
  generation is stale is dropped.
- **Panics.** Every request runs inside
  `std::panic::catch_unwind(AssertUnwindSafe(..))`. After a panic, the
  document is **dropped, never reused**, and the message falls back to plain
  text (FR-023). The panic hook logs the code location and the message id,
  **never the payload**, because logs carry no content.
- **Deadline.** The UI starts a timer when it sends a request (spec
  FR-023). Its length is **injected**: `postio-render` exports
  `DEFAULT_RENDER_DEADLINE = 400 ms`, and that is what production uses.
  `BodyView` takes the deadline as a construction parameter, so a test can
  choose its own:
  - a test **about** the deadline injects a tiny one and holds the render with
    a delay hook that the test releases. That is deterministic, with no wall
    clock in the assertion;
  - every **other** widget or app test injects
    `postio_test_support::scaled(DEFAULT_RENDER_DEADLINE)`, so a debug build
    on a busy CI runner does not fall back by accident.
    `check-test-deadlines-scale.py` already enforces this for the tests'
    own deadlines.

  If the timer fires first:
  - the UI shows the plain-text fallback with its notice;
  - it bumps the generation, so the late result is ignored;
  - it marks the thread **tainted**. A tainted thread is abandoned when it
    finishes: its next document goes to a fresh thread, and the old thread
    runs to completion, detached, and exits.

  There is no way to kill a running thread. The input caps exist so that a
  runaway render is rare.
- **Input caps before Blitz sees the document.** At most **50,000 elements**,
  a nesting depth of **256** (deeper subtrees are flattened), and **2 MiB** of
  composed HTML per message. A message over a cap renders its plain-text
  alternative with a notice. These are checked in `postio-body` while
  sanitizing, as a pure function.
- **The UI never waits.** While a render is outstanding, the previous frame
  stays on screen. A new conversation shows its header, and its body area
  keeps its last paint or shows the ground colour. No frame is blank (FR-029).
  The body appears when the snapshot arrives.
- **`panic = "unwind"` stays.** The workspace sets no `panic` in any profile,
  so it unwinds today. It MUST NOT change to `abort`, which would turn any
  Blitz bug into an app crash. `check-renderer-is-memory-safe.py` asserts the
  profile setting too.

**Rationale.**
- `catch_unwind` is sound only if the unwound state is discarded, and a
  `Document` holds `RefCell`s and stylo caches that may be mid-mutation.
- Blitz, stylo and taffy have no cancellation hook, so a deadline plus
  discard is the only honest bound.
- Stack overflow from deeply nested markup is not a panic and cannot be
  caught. The depth cap plus a large stack is the defence.

**Alternatives.**
- **Render on the main thread.** Rejected: a 400 ms layout would freeze the
  interface (FR-023).
- **A render subprocess.** Rejected by the maintainer (clarification
  2026-09-26, FR-023a): memory-safe code is the containment.

**Risk R6-a.** Whether `BaseDocument` is `Send` is unverified: `Node` has
`unsafe impl Send`, and `HtmlDocument` is not `Send`. The design does not
need it to be, because the document never leaves its thread. Phase 2's risk tasks (P2-risks) assert what *is* sent (the snapshot) with
`static_assertions`.

---

## R7 — The rendered snapshot: the UI never queries the engine

**Decision.** The render thread returns an immutable, `Send`
**`RenderedDocument`** (see `data-model.md`):
- the **display list**, recorded once with `anyrender::recording::Scene`
  through `blitz_paint::paint_scene`;
- the laid-out **size**;
- the **text index**: every text cluster in reading order, with its string
  range, rectangle, the node it came from, its message, and its style roles;
- **link rectangles** and their targets;
- **message extents** (the box of each message's canvas);
- **fold (summary) rectangles**;
- the **canvas** of each message and its **presentation** (R10);
- the render's **counts**.

Everything interactive runs on the UI thread **against the snapshot**:
- hit-testing, selection, copy;
- find, and find highlights;
- the accessibility text;
- link hover and activation;
- the rail's current message and scroll-to-message.

Blitz's own selection state is not used. Selection is a pair of positions in
the text index, and its highlight is drawn as an overlay (R8).

**Rationale.**
- It keeps interaction under 16 ms whatever the render thread is doing.
- It makes every one of those features a pure function over data, testable in
  `postio-render` without GTK.
- It stops the widget depending on Blitz's API surface. The spec asks that
  the engine be swappable if a better one appears; this contract is what
  makes that true.
- Selection, find and accessibility share **one serializer**, the text
  index, so "copies as text in reading order" (FR-017) and "reading order for
  the screen reader" (FR-020) cannot disagree.

**The reading-order serializer** walks the laid-out tree:
- tab between table cells, newline at row and block boundaries;
- skips `display: none`, `visibility: hidden` and zero-size content, which
  covers hidden preheaders;
- takes `alt` text for images.

`get_selected_text`, Blitz's own, joins inline roots with a single space and
flattens tables. It is not used.

**Alternatives.** A `Document` shared behind a mutex and queried from the UI
thread. Rejected: it couples every gesture to the render thread's progress
and to Blitz's API.

---

## R8 — Painting: a GtkScrollable over tiles

**Decision.** The widget, `postio-gtk`'s `BodyView`, implements
**`gtk::Scrollable`**. `vadjustment.upper` is the document height × zoom, and
`page_size` is the allocation height. It is never placed in a GtkViewport at
full height, which breaks at GL's maximum texture size (often 16,384 px).

- **Tiles.** The display list is rasterised into **tiles 512 logical px
  tall**, at the surface's **fractional** scale
  (`native().surface().scale()`). This happens on a small **raster pool** off
  the UI thread; the recording is plain data, so no document is touched.
- **Snapshot.** `snapshot()` appends only the tiles that intersect
  `[value − page, value + 2·page]`. Selection and find highlights are
  `append_color` rectangles over the tiles, so changing a selection
  re-rasterises nothing.
- **Tile cache.** Tiles live in an LRU capped at **64 MiB** per reader.
  Buffers are pooled, textures are built with `gdk::MemoryTextureBuilder`,
  and the same texture is kept across frames so the GPU upload cache hits.
- **No blank frames.** A missing tile draws a **quarter-scale tile**
  stretched into place: the whole document rasterised once at scale 0.25,
  capped at 16 MiB. If even that is missing, it draws the canvas colour.
  Prefetch follows the scroll direction.
- **Memory math.** One tile at 900 px width and scale 2 is
  1,800 × 1,024 × 4 B ≈ 7.4 MB. A 1,000 px viewport is about 2 tiles, so the
  cap holds about 8 tiles. Memory depends on the viewport, not on the
  message's height (FR-022).

**Rationale.**
- The spike's single texture was capped at 30,000 px (about 96 MB) and
  clipped anything longer. That violates FR-022 outright.
- `paint_scene` already takes a window and culls against it
  (`blitz-paint/src/render.rs:230-397`). Recording once and rasterising many
  times avoids re-walking the tree per tile.

**Alternatives.**
- **Re-running `paint_scene` per tile.** Rejected: it touches the document
  from the raster pool.
- **A GL renderer (vello on wgpu).** Deferred. The CPU raster measured
  4–8 ms per document, and a second GPU context inside GTK's is the
  flicker-prone arrangement ADR 0034 records.

---

## R9 — The sanitizer keeps what it can (`postio-body`)

**Decision.** `postio-body` changes from *removing* to *preserving or
translating*. Every remaining removal is a row in one enumerable refusal list
with its reason (spec FR-005, 001 FR-019b). The changes:

| Change | Today | After |
|---|---|---|
| `class`, `id` | dropped by ammonia's default attribute set (#1545) | kept. A sender value starting `postio-` is refused, for containment (FR-025): a sender element must not wear Postio's chrome classes. `id` is also rewritten to be unique per message (`m<scope>-<id>`), and `href="#id"` and `<label for>` follow. |
| `<body>`/`<html>` `bgcolor`, `background` colour, `text`, `link`, `vlink`, `alink`, `style` | lost when the fragment is cleaned | lifted onto the message container as the **canvas** (FR-006). `link` becomes a scoped `a { color }` rule. |
| `<font color face size>` | tag unwrapped, attributes lost | rewritten to `<span style="color; font-family; font-size">`. `size` 1–7 maps to the HTML legacy keyword scale (FR-007). |
| `valign` | kept, but Blitz ignores it (#508) | `vertical-align` on the cell |
| `cellpadding` / `cellspacing` | kept, but Blitz ignores it (PR #503) | `padding` on every cell / `border-spacing` on the table |
| `<table border=N>` | kept, draws nothing | `border` on the table and its cells, as browsers do |
| `table align=center` | only `text-align` | `margin-inline: auto` |
| `img align=left/right` | not mapped | `float` |
| `background="…"` on `table`/`td` | dropped | a `background-image` rule, `url()` resolved through the same `cid:` and remote-consent rewriting as `src`. A non-consented remote URL is refused, for privacy. |
| `<meta name="color-scheme">` | removed with `<meta>` | its value is carried as `data-postio-color-scheme` on the container, which is what R10 reads |
| `<title>` | stripped (PR #1546) | unchanged |
| inline `<svg>` | stripped | unchanged; refusal reason recorded (R5) |
| input caps (R6) | none | nodes, depth and size, checked here |

**Rationale.** Every row is a pure transformation of markup, provable in
milliseconds in `postio-body`, which has 49 unit tests in 0.00 s today. The
sanitizer is shared with the macOS frontend (through `postio-ui` → FFI) and
the TUI, and with reply quoting (ADR 0033), so fidelity improves for every
frontend, not only this renderer.

Class collisions **between** messages cannot happen, because `styles.rs`
already scopes every sender rule under its own message's container.

**Alternatives.** Map the attributes inside a `blitz-dom` fork. Rejected, per
R1: this is cheaper, and it is ours.

**Stale comments fixed on the way:**
- `sanitize.rs:9-13`, which says sender style is stripped;
- `reader.css:24-26` and `118-121`;
- `document.rs:609-611`, which says `class` is kept, when it is not.

---

## R10 — Theme: one source, one rule, one floor

**Decision.**

**Source (FR-011).** The widget reads `adw::StyleManager` (`dark`,
`high-contrast`) and passes both into every render request. Blitz's
`Viewport.color_scheme` is set from them, so a sender's
`@media (prefers-color-scheme: dark)` matches exactly when the app is dark.
A change to either triggers a re-render. The old frame stays until the new
one arrives. Nothing else decides the scheme. The editor bug
(`editor.rs:185`), where a web view resolved the scheme from its own
settings, cannot recur, because there is no web view.

**Classification (FR-013), a pure function per message.** It runs on the
render thread after the first style pass, over computed styles:

1. **SenderDark.** The app is dark, and the message declares dark support:
   its carried `color-scheme` includes `dark`, or one of its scoped rules is
   inside `@media (prefers-color-scheme: dark)`. It renders as styled, and the
   contrast floor still applies.
2. **Designed.** Any element inside the message other than its container has
   a non-transparent `background-color` or any `background-image`, or the
   canvas colour has a relative luminance below **0.9**. In dark mode it
   renders as **paper**: the canvas (the sender's colour, or white if none) is
   painted on the container as a card with the reader's radius and inset, and
   the sender's colours are untouched.
3. **Adaptable.** Everything else, including an everyday reply that sets only
   a white page and black text. The canvas becomes the theme's reading ground,
   and every text colour is **repaired** to meet the floor.

In light mode every message renders as styled on its own canvas, with the
contrast floor still applied.

**Contrast repair (FR-012).** For each text run, find the **painted
background**:
- walk the ancestors in the laid-out tree, compositing
  `background-color × opacity` over the canvas until the result is opaque;
- if an ancestor has a `background-image` (**unknown ground**), use the mean
  colour of the decoded image as the ground.

If the contrast is below **4.5:1** (**7:1** in high contrast), binary-search
**OKLCH L only** toward the far end until the floor is met. The floor has no
large-text allowance: spec FR-012 applies it to every run of text, and
WCAG's lower 3:1 for large text is deliberately not taken. Keep C and h, and reduce C
only if the colour leaves gamut.

The repaired colours are applied as **overrides on the render thread's
document** (`color` set on the element with the highest precedence), followed
by one restyle. That is a two-pass render: the second pass is measured, and
counted in the render counts.

**Darken (FR-013a).** For a message presented as Designed, the darken
command:
- remaps each background by OKLab `L → 0.12 + (1 − L) × 0.18`, which puts it
  in [0.12, 0.30] with hue kept;
- remaps borders the same way;
- then runs contrast repair on the text.

**Images are never touched (FR-015).** An image inside a darkened or
adaptable message keeps a backing of its intended canvas colour, painted
behind it and inset to its box.

**The design dependency (FR-016).** The dark reader palette must be
**designed**, not derived (#1588). `reader_dark_roles`
(`postio-ui/src/tokens.rs:684`) today takes neutral-900 for the ground.
Settling the palette is a design call an agent can make, so it goes to
`/ux-architect` → `/gtk-design` against the design canvas. It is task P5-palette in the plan.
The renderer consumes whatever roles it produces, and nothing waits on it
except FR-016's own acceptance test.

**Rationale.**
- **Apple Mail** honours the sender's `color-scheme` and leaves designed mail
  alone.
- **Outlook.com's `fixContrast()`** flips CIELAB L\* with a\*/b\* kept, stores
  the originals, and makes the change undoable.
- **Fastmail** historically gave HTML mail a white "paper".
- **Thunderbird 140** recolours, with a per-message off switch.

The chosen rule is the maintainer's (clarifications 2026-09-26): paper for
designed mail, with an undoable darken, and adaptation with a contrast floor
for everything else. OKLCH is the perceptual space in which an L-only change
keeps hue.

**Alternatives.**
- **Full inversion of every message**, as Gmail on mobile does. Rejected by
  the maintainer.
- **CSS filter inversion.** Rejected: it inverts images, which FR-015
  forbids.

---

## R11 — Zoom

**Decision.**
- **Mechanism.** User zoom is Blitz's `Viewport.zoom`. The device scale is
  `hidpi_scale`, taken from the surface's fractional scale. The two are never
  folded together. `Viewport::scale() = hidpi × zoom` gives the device pixel
  ratio, and the CSS viewport width is `width / scale`. So `@media (width)`
  sees the narrower width, and designs re-flow the way browser page zoom does
  (FR-021a).
- **Steps.** 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300
  (FR-021b).
- **Keyboard and command zoom** re-render with the top of the viewport
  anchored: the text-index cluster at the top is found before the render, and
  its new position is scrolled to afterwards.
- **Pointer and pinch zoom** anchor the cluster under the pointer (FR-021c).
  `GtkGestureZoom` drives a **transient visual scale** on the existing tiles
  during the gesture. On release it snaps to the nearest step and re-renders
  at that zoom. The stretched tiles stay on screen until the new ones land,
  so there is no blank frame.
- **Ctrl+scroll** uses `GtkEventControllerScroll` with the Control modifier,
  one step per notch.
- **Registry commands:** `ZoomIn`, `ZoomOut` and `ZoomReset` (contract
  `registry-commands.md`).
- **Persistence.** A new **`[reader]`** section in `config.toml` with
  `zoom = 100`, stored as a percentage and clamped to the nearest step on
  load. It is added the way `sender_avatars` was (the six-edit path in the
  integration map). It is not per sender (FR-021d).
- **Indicator.** A small overlay on the reading pane shows the level when it
  is not 100%, with a reset button (FR-021e). At 100% it is hidden.

**Alternatives.** Scaling the rasterised tiles only. Rejected: text would
blur, and layout would not re-flow.

---

## R12 — Remote images for allowed senders, fetched by the app

**Decision.** A **`RemoteImageFetcher`** in `postio-runtime`, the layer that
already owns I/O, built on `io-http` + `pimalaya-stream` over
`postio-transport`'s TLS. It is called by the reader's owner only when
`remote_images_for(sender)` is `Allowed`, or on "show once". It is marked
`POSTIO-CONSENT:` for `check-no-silent-tracking.py`.

The fetch policy:
- only the `http(s)` image URLs of **that message**, taken from its sanitized
  document;
- `GET` only, with no cookies, no `Referer`, no `Origin`, and a fixed
  `User-Agent` that does not identify Postio or its version;
- at most **3** redirects, each still `http(s)`;
- the response must sniff as an **allowed image format** (R4). Any other
  response is dropped;
- at most **16 MiB** per image, a **10 s** timeout, and **4** concurrent
  fetches per message.

Results go into an **in-memory, per-session cache** keyed by URL. They are
never written to disk and never persisted. "Show once" leaves no trace after
the session.

**How images arrive (FR-026).** The first render happens with placeholders
at their declared sizes. Each arrival adds bytes to the message's resource
table and triggers one re-render, coalesced to at most one per animation
frame. With no network the fetches fail quietly and the placeholders stay
(001 FR-032).

Allowing a sender fetches **images only**. CSS `url()` for fonts or imports
stays refused (FR-025). `background` images of an allowed sender are images
and are fetched.

**Rationale.** The renderer stays disconnected (FR-001). The network lives
where Postio's network already lives and is already audited. Today WebKit
fetches through an ephemeral `NetworkSession` whose request headers Postio
never controlled; this puts every header in Postio's hands.

**Alternatives.** Turning on `blitz-net`. Rejected: it puts a network stack
in the renderer's graph, which is exactly what FR-001 forbids.

---

## R13 — Reference renders and the fidelity metric (SC-002)

**Decision.**

**Capture.** A capture tool, `crates/postio-gtk/examples/capture_reference.rs`,
must run while WebKit is still the reader's engine. It is Phase 1's first
task, before anything is removed. It renders each designed corpus message
from its **unsanitized** HTML body:
- in a WebKitGTK view with network, JavaScript and remote loads off;
- `cid:` parts served from the fixture itself;
- at **800 CSS px** wide, scale 1, light scheme;
- with the bundled fonts installed as the default families.

It writes one PNG per fixture to `crates/postio-test-support/data/reference/`.
They are checked in. The tool needs a display and is run by hand. Each PNG is
committed with a line in `reference/README.md` stating the WebKitGTK version
that produced it.

**The metric.** It is defined, with its constants, **before** the first
comparison runs (spec SC-002), in `contracts/fidelity-metric.md`. In short:
- Both images are cut into **16 × 16 px cells**, each reduced to its mean
  OKLab colour.
- A fixture **matches** when **≥ 92%** of cells are within **ΔE_OK ≤ 0.08**,
  **and** the document heights agree within **±8%**.
- Cell means wash out the font differences between the two engines, which
  are not the question. Columns, blocks, backgrounds and images are the
  question, and they survive the averaging.
- The check runs in `postio-render`'s tests headlessly. That is possible
  because our renderer needs no display.

**Rationale.** The maintainer chose a browser-engine reference
(clarification 2026-09-26). A per-pixel diff would fail on font rasterisation
alone. A structural comparison of layout boxes would need the reference
engine's box tree, which a PNG does not carry. Cell-mean comparison measures
what the spec asks: "columns in the same places, images in place at intended
size, colours as specified".

**Alternatives.**
- Pixel diff: rejected, since fonts make it fail.
- Perceptual hashing: rejected as too coarse to see a column collapse.
- SSIM: plausible, but harder to explain in a failure message than "these
  cells differ".

---

## R14 — Testing strategy

**Decision.** Rules are proven at the cheapest layer that can fail them:

| Layer | Proves | Runs as |
|---|---|---|
| `postio-body` unit | every sanitizer row in R9, input caps, the refusal list | `--lib`, milliseconds |
| `postio-render` unit + `tests/` | classification, contrast repair, darken, the text index and reading order, selection and find over the snapshot, link targets, message extents, zoom re-flow, image limits, SVG lock-down, panic and deadline fallback, **SC-001 contrast over the corpus by sampling the rasterised pixels behind every text cluster**, **SC-002 fidelity**, SC-009 zoom sweep | headless, no display |
| checks | FR-001/FR-023a graph proofs (R3) | `scripts/check.sh` |
| `postio-gtk/tests` (gtk_suite) | the widget: gestures to selection, the clipboard, the scroll adjustment, the tile cache bound, no blank frame on scroll or theme switch, `AccessibleText` contents and extents, the zoom indicator | headless compositor |
| `postio-app/tests/app_suite` | wiring: a message opened from the list is painted, a theme switch repaints, allow → images arrive over a loopback listener, find and zoom commands reach the pane | headless |

**SC-001 is asserted on pixels, not on computed styles.** For each text
cluster, the test samples the rasterised background around the glyph box,
excluding the glyph pixels, and compares it with the cluster's final colour.
That is "the colour actually painted behind it", and it catches a repair that
computed the wrong ancestor.

**What happens to today's WebKit tests.** The integration map counted 21
`gtk_reader` sub-cases, about 66 gtk_suite reader, rail and conversation
cases, and about 29 app_suite cases.
- **Behaviour assertions are ported** to `BodyView`: rendering, notices,
  anchors, scroll, the rail's current message, reply targets, view original,
  and search highlighting.
- **WebKit-mechanics assertions are deleted, each with its replacement named
  in the commit:**
  - web-process counting → SC-006's "no rendering process" plus render counts;
  - CSP and JavaScript refusal → the R3 graph proof plus the
    no-script-in-snapshot test;
  - `webkit_probe` computed-style probes → `postio-render` style assertions.
- **Every loopback egress proof is kept and re-pointed at the new reader.**
  That includes #1336's style-borne beacon.

**Measurement tier.** The fidelity comparison over the whole corpus and the
zoom sweep are plain tests unless they exceed the landing budget. If they do,
they move to the nightly tier with a `POSTIO-MEASUREMENT:` marker, never
under `#[ignore]`.

---

## R15 — Script-free behaviour the reader used to get from JavaScript

**Decision.**
- **The rail's current message** (001 FR-034/FR-035): the message with the
  greatest visible area, computed from the snapshot's `message_extents` and
  the vadjustment. `postio-ui/src/reader/rail.rs` already holds the rule. The
  script observer (`RAIL_HANDLER`, #1367) is deleted.
- **Scroll-to-message and page up/down**: vadjustment arithmetic over
  `message_extents` and the page size. The `#pos-N` markers and the
  `SCROLL_REPORTER` script go.
- **Anchor restore** (`Place`): the anchor is stored as a text-index
  position, not a DOM script.
- **Verb links** (`postio-allow`, `-reply`, `-forward`, `-continue`): the
  link handler intercepts them by scheme, as `decide_policy` does today.
- **`<details>` folds**, used for quotes and for thread messages. Blitz's UA
  sheet styles `details`/`summary`. Toggling is done by Postio: a click on a
  summary rectangle flips `open` on the render thread's document and
  re-lays out.
- **In-document chrome** loses its WebKit-only styling:
  `::-webkit-details-marker` in thread.css is replaced by standard
  `summary::marker` or `list-style`.

**Risk R15-a.** How far Blitz supports `<details>` toggling. Retired by the
risk-retirement tasks of Phase 2. If toggling is unsupported, the fold becomes a
Postio-owned attribute, changed the same way.

**Constitution effect.** Postio's own script in the reader disappears
entirely. 001's R3 exception (*"JavaScript is enabled for Postio's own
injected observer"*) is retired, and Principle VI becomes simpler to state.

---

## R16 — Reader view becomes opt-in

**Decision.**
- `sheet_for` / `suits_reader_view` stop deciding how a message opens: every
  message opens `Original` (spec FR-031).
- A new registry command, **`ToggleReaderView`** (`mod+shift+o`), reduces the
  focused message with `reader_view::reduce`. `View original` (`mod+o`)
  returns from it.
- `reads_as_bulk` survives for what else uses it, the unsubscribe banner, and
  no longer sets the default.

**Rationale.** The maintainer's answer (clarification 2026-09-26). Keeping
Reader view one keystroke away preserves 001 US4 scenario 6.

---

## R17 — What happens to WebKit

**Decision.** The **reader** stops using WebKit entirely when this branch
lands (FR-027):
- `reader/scheme.rs`, the reader's `web_process` watch, `hardened_settings`,
  `build_view`, the script handlers and `Reader::view()` are deleted;
- `conversation.rs` stops importing `webkit6`, replacing `connect_load_changed`
  with the snapshot-arrived signal.

The **composer** still uses WebKit (`postio-gtk/src/editor.rs`) until
ADR 0039's native composer lands. So `webkit6` stays a `postio-gtk`
dependency, and SC-006 is measured as "opening the **reader** starts no
rendering process".

**macOS and TUI.** They are unaffected except by the R9 sanitizer changes,
which improve them. Their own renderers (WKWebView, text) are out of scope.

---

## R18 — Governance owed by this branch

**Decisions.**

- **A new ADR, 0042: *The reading renderer is disconnected and memory-safe*.**
  This is a boundary other work must obey, so it outlives this feature. It
  records two rules:
  - no network crate, no C and no system fontconfig in `postio-render`'s
    graph;
  - remote bytes enter only through the app's fetcher.

  It is kept to those rules. The feature's reasoning stays in this spec,
  following CLAUDE.md's "a spec and an ADR are not both needed" rule.
- **ADR 0032 is amended.** Its decision, one document per conversation,
  stands. Its mechanism, "in one `WebView`", is superseded by 0042 and
  `postio-render`.
- **ADR 0003's reader statements** (script off, network off, as WebView
  settings) are amended to point at 0042.
- **Constitution Principle VI and CLAUDE.md** say *"the reader's WebKit view
  has JavaScript and network off"*. That becomes *"the reader's renderer
  cannot run script or reach the network"*. It is a PATCH amendment to the
  constitution: a clarification that strengthens the guarantee, and 0042's
  check enforces it. It goes in this branch's PR with the Sync Impact Report
  updated.
- **`PRODUCT.md` §21** gets the same wording change.
- **#1547 and #1545** are settled by this branch. The PR body names them.
  **#1543** is the spike, and closes with the PR. **#1588** is settled by
  task P5-palette. **#1501** is settled by R4's limits. These are PR-body references,
  never closing keywords in commits.
