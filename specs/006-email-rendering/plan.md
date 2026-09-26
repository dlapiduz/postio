# Implementation Plan: Faithful, Readable Email Rendering

**Branch**: `feature/email-rendering` | **Date**: 2026-09-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/006-email-rendering/spec.md`

## Summary

The spec asks for three things:
- mail that is **legible in every theme**, and the dark-on-dark bug is gone;
- mail that **looks as its sender built it**;
- privacy guaranteed by a renderer that **cannot** reach the network or run
  script, not by deleting markup.

**The approach.**
- **A new crate, `postio-render`, wraps Blitz** (0.3.0-beta.2, exact pins,
  from crates.io). It is GTK-free, has no networking crate, and has no C
  anywhere in its dependency graph. Two checks prove it (R2, R3).
- **One render thread per reader.** It owns the engine and returns an
  immutable snapshot: a display list, a text index, link, message and fold
  boxes, and counts. Everything interactive runs over that snapshot on the
  UI thread (R6, R7).
- **A GTK widget, `BodyView`, paints the snapshot as tiles in a
  `GtkScrollable`** and supplies selection, find, links, accessibility and
  zoom (R8, R11).
- **The sanitizer in `postio-body` stops deleting and starts translating.**
  It keeps `class` and `id`, lifts the `<body>` canvas, and rewrites the
  legacy attributes Blitz lacks into inline style (R9).
- **Dark mode is one pure classification per message plus a contrast floor
  enforced on painted pixels:** Paper, Adapted, SenderDark or Darkened (R10).
- **Remote images for allowed senders are fetched by the app** in
  `postio-runtime` and handed to the renderer as bytes (R12).
- **WebKit leaves the reader entirely.** The composer keeps it until ADR 0039
  lands (R17).

**The engine is not yet decided.** Everything above describes the Blitz arm.
Research **R0** sets out a like-for-like evaluation of Blitz against WebKit,
with gates, scored criteria and a decision rule written before either arm
runs. It sits between the engine-neutral work and the engine-specific work,
and the maintainer decides from its scorecard. If WebKit is chosen, R0 lists
the four requirements that are amended and the plan is re-run for the
engine-specific phases.

All other technical unknowns are resolved in [research.md](./research.md).
If Blitz is chosen, three risks remain, each retired by a named task at the
start of its Foundational phase:
- R6-a: which snapshot types are `Send`;
- R15-a: `<details>` toggling;
- the image and SVG paint path the spike never exercised.

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml` (edition 2024)

**Primary Dependencies**:
- New in `postio-render`:
  - `blitz-dom`, `blitz-html`, `blitz-paint` and `blitz-traits`
    `=0.3.0-beta.2`, with `default-features = false`;
  - `blitz-dom` features `floats` and `svg`, and **not** `system-fonts` or
    `net`;
  - `blitz-paint` feature `svg`;
  - `anyrender =0.13` and `anyrender_vello_cpu =0.17`;
  - `parley 0.11` without `system`;
  - `fontdb` (pure Rust);
  - `image 0.25` with `png`, `jpeg`, `gif` and `webp` only;
  - `usvg 0.48` for SVG re-serialisation.
- Existing, reused:
  - GTK 4.22 through `gtk4` 0.11.4 (`v4_20`; `AccessibleTextImpl` needs
    `v4_14`/`v4_16`);
  - libadwaita 1.9 (`adw::StyleManager`);
  - `io-http` + `pimalaya-stream` over `postio-transport` TLS, for the image
    fetcher.

**Storage**: none new. One config key, `[reader] zoom`. The remote-image cache
is in memory only.

**Testing**:
- `cargo test --lib` for `postio-body` and `postio-render` rules;
- `cargo nextest run -p postio-render --test <suite>`, **headless with no
  display**, for layout, pixels, contrast, fidelity and egress;
- `gtk_suite` for the widget;
- `app_suite` for wiring;
- two new checks under `scripts/checks/`.

**Target Platform**: Linux, Wayland first. Nothing here precludes the macOS
frontend, which keeps its WKWebView and gains the sanitizer improvements.

**Project Type**: desktop application, a Rust workspace of 24 crates; this
adds a 25th.

**Performance Goals**:
- a warm message on screen within the 16 ms interaction budget, measured as
  counts: one render per request and at most 2 style passes;
- no blank frame on navigation, theme switch, zoom or scroll;
- a theme switch or zoom step shown within one frame of its render arriving.

**Constraints**:
- 400 ms render bound (`DEFAULT_RENDER_DEADLINE`, injectable, and scaled
  in tests), then plain-text fallback;
- no C on the content path;
- no network in the renderer's graph;
- tile memory at most 64 MiB per reader, independent of message height;
- input caps of 50,000 elements, depth 256, and 2 MiB per message;
- image limits of 8,192 px per side, 40 megapixels, and 16 MiB.

**Scale/Scope**:
- the spec: 6 user stories, 39 functional requirements (FR-001 to FR-031,
  plus FR-013a, FR-021a–f and FR-023a), 9 success criteria;
- the crates touched:
  - `postio-body` (sanitizer);
  - `postio-ui` (document, reader view default, render counter, tokens);
  - `postio-render` (new);
  - `postio-gtk` (widget, conversation wiring, WebKit removal from the
    reader);
  - `postio-core` (8 commands);
  - `postio-config` (`[reader]`);
  - `postio-runtime` (fetcher);
  - `postio-app` (wiring);
  - `postio-ffi` (settings mirror);
  - `scripts/checks`.

## Constitution Check

*GATE: must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Local-first, the UI never awaits the network | **Pass** | Rendering happens off the UI thread, and the previous frame stays until the snapshot lands (R6). Remote images render as placeholders first and arrive by re-render (R12). Offline, nothing waits (001 FR-032). |
| II. The keyboard is a system | **Pass, with work** | 8 new registry commands, with ids fixed in `contracts/registry-commands.md`. Keys come from the registry; `docs/keybindings.md` is regenerated and the golden table updated by hand. Ctrl+scroll, pinch and the indicator's reset button all dispatch the same commands. |
| III. Search is navigation | **N/A** | In-message find is not the query language. It is a folded substring match over one document's text, and introduces no second query language. |
| IV. Test-first | **Pass** | Every rule lands red first at the cheapest layer: sanitizer rows in `postio-body` (milliseconds), then classification, repair, the text index and fidelity in `postio-render`. The renderer is headless, so geometry and pixels are assertable without a display. That removes the wall 001's plan recorded. SC-001 is asserted on **painted pixels**, not computed styles (R14). |
| V. Performance as counts | **Pass, with work** | `RenderCounts` joins 001's render counter. Budgets are asserted as renders per request, style passes, tiles held and bytes held, not wall-clock. The 400 ms bound is a containment deadline, not a gated timing. |
| VI. Privacy is a feature | **Pass, strengthened; amendment owed** | The guarantee moves from WebKit settings to what is linked, and is proven by the graph checks (R3). Postio's own reader script disappears (R15), which retires 001 R3's exception. Remote image fetches carry no cookies, `Referer`, `Origin` or identifying User-Agent, and are cached in memory only. **Owed on this branch:** Principle VI's wording ("the reader's WebKit view has JavaScript and network off" → "the reader's renderer cannot run script or reach the network") as a PATCH amendment with the Sync Impact Report, plus CLAUDE.md, `PRODUCT.md` §21 and ADR 0003 (R18). |
| VII. Boundaries are enforced | **Pass, with a new boundary** | `postio-render` is GTK-free, network-free and C-free, enforced by `RULES["postio-render"]` and `check-renderer-is-memory-safe.py`. `postio-gtk` gains no SQL or protocol. The fetcher lives in `postio-runtime`, beside the existing I/O. **Pimalaya first:** the fetcher uses `io-http`/`pimalaya-stream`, which the workspace already uses for OAuth and discovery, so no new HTTP stack is introduced. |
| No backwards compatibility | **Pass** | The reader's WebKit path is deleted, not flagged (FR-027). Bodies re-render from source, and there is nothing to migrate. |
| One fact, one home | **Pass** | This spec inherits 001's FR-019 to FR-032 by citation. ADR 0042 holds only the boundary rule that outlives the feature (R18), and ADR 0032 is amended rather than restated. |

**Gate result: PASS.** There are no unjustified violations. The two "with
work" rows are deliverables, not exceptions.

### Post-design re-check (after Phase 1)

Re-evaluated against `data-model.md`, `contracts/` and `quickstart.md`:

- **Principle V is satisfiable.** `RenderCounts` gives every budget a count,
  and `contracts/renderer-api.md` guarantee 9 makes "one render, at most two
  style passes" an assertion.
- **Principle VI is strengthened in two places the first pass missed:**
  - SVG as an image could read local files through usvg's default resolver.
    This is closed by re-serialising SVG in the resource table (R5).
  - The spike's fontconfig path put message text through C. It is closed by
    `FontSet` over `fontdb` (R3).
- **Principle II has two contexts that share keys**, `mod+shift+g` and
  `mod+shift+o`, which are Composer-only today. `command_registry.rs` is the
  arbiter, and a fallback key is named for each.
- **No new violation.** Still PASS.

## Project Structure

### Documentation (this feature)

```text
specs/006-email-rendering/
├── spec.md              # the what and why, clarified 2026-09-26
├── plan.md              # this file
├── research.md          # R1–R18: decisions, rationale, alternatives, risks
├── data-model.md        # sanitized message, resource table, snapshot, text index, presentation
├── quickstart.md        # runnable validation per story
├── contracts/
│   ├── renderer-api.md          # postio-render's surface and its 9 guarantees
│   ├── renderer-graph-checks.md # the two checks proving FR-001 / FR-023a
│   ├── fidelity-metric.md       # SC-002's reference renders and constants, fixed up front
│   ├── registry-commands.md     # 8 commands and the [reader] config section
│   └── remote-image-fetch.md    # the app-side fetcher's policy
├── checklists/requirements.md
└── tasks.md             # /speckit-tasks, not created here
```

### Source Code (repository root)

```text
crates/
├── postio-render/                    # NEW: GTK-free, network-free, C-free
│   ├── src/
│   │   ├── lib.rs                    # Renderer, RenderRequest, RenderedDocument
│   │   ├── thread.rs                 # render thread, generations, catch_unwind, taint
│   │   ├── fonts.rs                  # FontSet: bundled faces + fontdb, explicit fallbacks
│   │   ├── resources.rs              # the closed table, R4 probing and limits, R5 SVG re-serialisation
│   │   ├── theme.rs                  # classification, painted-ground walk, OKLCH repair, darken
│   │   ├── snapshot.rs               # display-list recording, low-res fallback, boxes, counts
│   │   ├── text_index.rs             # reading-order serializer, hit, word/line, rects, find
│   │   └── tile.rs                   # rasterize_tile (pure, any thread)
│   └── tests/
│       ├── reference/                # WebKit reference PNGs + README + MISMATCHES.md
│       ├── fidelity.rs  contrast.rs  presentation.rs  hostile.rs
│       ├── egress.rs  text_index.rs  zoom.rs
├── postio-body/src/
│   ├── sanitize.rs                   # R9: keep class/id, lift canvas, refusal list, caps
│   └── hints.rs                      # NEW: presentational attribute → inline style
├── postio-ui/src/reader/
│   ├── document.rs                   # canvas on the container; no WebKit-only CSS; FR-031 default
│   └── (tokens.rs)                   # designed dark reader roles (task P5-palette)
├── postio-gtk/
│   ├── src/body_view/                # NEW: BodyView widget
│   │   ├── mod.rs                    # GtkScrollable, request/deadline, theme and scale watch
│   │   ├── tiles.rs                  # raster pool, LRU 64 MiB, low-res fallback
│   │   ├── interact.rs               # gestures, selection, clipboard/primary, links, folds
│   │   ├── find.rs                   # find bar and highlight overlay
│   │   ├── zoom.rs                   # steps, anchors, pinch transient scale, indicator
│   │   └── a11y.rs                   # AccessibleTextImpl over TextIndex
│   ├── src/reader/view.rs            # Reader keeps its chrome; its body becomes BodyView
│   ├── src/reader/scheme.rs          # DELETED (the reader's schemes)
│   ├── src/conversation.rs           # webkit6 import gone; snapshot-arrived replaces load-changed
│   └── examples/capture_reference.rs # NEW: the one-shot WebKit reference capture (Phase 1)
├── postio-core/src/{command.rs,registry.rs}   # 8 commands
├── postio-config/src/{lib.rs,reader.rs}       # [reader] zoom
├── postio-runtime/src/remote_images.rs        # NEW: RemoteImageFetcher (POSTIO-CONSENT)
└── postio-model/tests/corpus/                 # rendering-corpus fixtures via /add-fixture
scripts/checks/
├── check-crate-boundaries.py                  # + RULES["postio-render"]
└── check-renderer-is-memory-safe.py           # NEW
docs/decisions/0042-the-reading-renderer-is-disconnected-and-memory-safe.md   # NEW
```

**Structure decision.** The one new crate is `postio-render`, because a
dependency graph can only be proven clean at a crate root (R2, R3).
Everything else lands in the crate that already owns that concern:
- markup in `postio-body`;
- documents and tokens in `postio-ui`;
- widgets in `postio-gtk`;
- commands in `postio-core`;
- I/O in `postio-runtime`.

## Phases

Each phase is a run of commits on `feature/email-rendering`, red first. The
branch lands **once**, as a single pull request reviewed against the spec
(constitution, *Spec-driven work*). The order is forced by two facts:
- **the WebKit reference renders must be captured before WebKit leaves the
  reader;**
- **the engine is chosen (R0) only after the engine-neutral work exists**,
  so that both arms render the same improved input.

**Tasks Phases 1–3 come before everything below** (the numbering in `tasks.md`):
- **Phase 1**: the corpus, the references and the metric.
- **Phase 2, engine-neutral groundwork**: the sanitizer keeps `class` and
  `id`, lifts the canvas, carries `color-scheme`, walks the refusal list,
  translates legacy hints, and opens every message in its original layout.
  This improves today's WebKit reader too.
- **Phase 3, the evaluation**: both arms as harnesses, the gates G1–G3, the
  scores S1–S8, a scorecard note, and the maintainer's decision.

The plan's Phases 2–9 below (tasks Phases 4–11) are the **Blitz** arm's plan. If WebKit is chosen, they are
re-planned before any of them starts.

**Phase 1 — Evidence before engine** (SC-002 prerequisites)
- **P1-corpus.** Build the rendering corpus with `/add-fixture`, all
  synthetic with reserved domains:
  - designed newsletters (multi-column, button, hero image by `cid:`);
  - transactional mail;
  - legacy-client mail (`<font>`, `<center>`, body attributes, `valign`,
    `cellpadding`, `<table border>`);
  - a white-page correspondence reply;
  - a dark-aware campaign (`color-scheme` plus `@media` dark);
  - hostile mail (the existing beacon and escaping-styles fixtures, plus
    deep nesting, a 40,000 px message, a malformed PNG, an SVG that names
    `/etc/…`, and every `url()` vector);
  - international text (RTL, CJK, emoji).
- **P1-capture.** `capture_reference.rs`, and check in the PNGs and the
  README.
- **P1-metric.** Encode `contracts/fidelity-metric.md` as a comparison
  function with its own unit tests, still with no candidate renderer.

**Phase 2 — The crate and its proofs** (FR-001, FR-003, FR-004, FR-023a,
FR-024)
- **P2-checks.** Both checks, red first: prove each bites against a
  deliberately bad `Cargo.toml` in a scratch copy, then add `postio-render`
  clean.
- **P2-risks.** Retire the three risks before building on them:
  - `static_assertions` on the snapshot types (R6-a);
  - a `cid:` PNG actually paints (the spike's missing formats);
  - an SVG naming a local file paints without reading it (R5);
  - `<details>` toggling (R15-a).
- **P2-fonts, P2-resources, P2-thread.** `FontSet` over `fontdb`; the
  resource table with probing and limits; the render thread with
  generations, `catch_unwind`, taint and caps. Egress test.

**Phase 3 — The sanitizer keeps what it can** (US2; FR-005 to FR-010,
FR-031)
- **P3-sanitize.** R9's table row by row in `postio-body`, one test per row,
  with the refusal list walked. The stale comments are fixed as their own
  commits.
- **P3-reader-view.** Every message opens in its original layout, and
  `ToggleReaderView` is added (R16).

**Phase 4 — Renderer core and fidelity** (US2, US3; SC-002, SC-004)
- **P4-snapshot.** Display-list recording, message, link and fold boxes,
  counts, and the low-res fallback.
- **P4-text.** The text index and reading-order serializer. Hit, word, line,
  rects and find as pure functions.
- **P4-fidelity.** Run the metric over the corpus. The first run records
  MISMATCHES.md and fixes the metric constants once if they are wrong, per
  the contract. Then fix the gaps: sanitizer hints first, and a `blitz-dom`
  patch only if unavoidable, recorded in R1.
- **P4-hostile.** The hostile suite: every fixture contained or `FellBack`
  within 400 ms.

**Phase 5 — Theme and legibility** (US1; FR-011 to FR-016, SC-001)
- **P5-classify.** Presentation per message, as a pure function over
  computed styles, with a table of fixtures and the expected variant for
  each.
- **P5-repair.** The painted-ground walk and OKLCH L-only repair. The SC-001
  pixel-sampling test is red first, against the spike-equivalent path.
- **P5-paper-darken.** The paper card, darken and its inverse, and image
  backing (FR-015).
- **P5-palette.** Designed dark reader roles through `/ux-architect` →
  `/gtk-design` (#1588), consumed as `Theme.palette`.

**Phase 6 — The widget** (US4; FR-017 to FR-020, FR-022, FR-028, FR-029)
- **P6-view.** `BodyView` as a `GtkScrollable`, with tiles, the LRU, the
  raster pool and the low-res fallback. Assert tile bytes against the 40,000
  px fixture.
- **P6-interact.** Selection, copy and primary selection, links (hover
  target, keyboard focus, schemes), folds, and verbs.
- **P6-find.** The find bar and three commands.
- **P6-a11y.** `AccessibleTextImpl` (contents, caret, selection, extents),
  verified through GTK's test AT context.
- **P6-scroll.** The rail's current message, scroll-to-message and page
  up/down from `message_extents`. The rail's script and scroll markers are
  deleted.

**Phase 7 — Zoom** (US6; FR-021 to FR-021f, SC-009)
- **P7-zoom.** Steps, anchors, Ctrl+scroll, pinch with a transient scale,
  the indicator, the three commands, and `[reader] zoom` through the
  six-edit config path. The SC-009 sweep.

**Phase 8 — Allowed senders' images** (US5; FR-025, FR-026)
- **P8-fetch.** `RemoteImageFetcher`, following
  `contracts/remote-image-fetch.md`. The loopback tests run in both
  directions, plus headers and no-prefetch.

**Phase 9 — The switch** (FR-027, FR-030, SC-006, SC-008)
- **P9-wire.** `Reader`'s body becomes `BodyView`. Change `conversation.rs`
  and `window.rs` (`view()` → `widget()`, and snapshot-arrived instead of
  `load_changed`).
- **P9-port.** Port the behaviour tests and delete the WebKit-mechanics
  tests, naming each replacement (R14).
- **P9-delete.** Delete the reader's WebKit path: `scheme.rs`, the web-process
  watch for the reader, `hardened_settings`, the script handlers, and
  `Reader::view()`.
- **P9-docs.** Write ADR 0042, amend ADR 0032 and ADR 0003, make the
  constitution PATCH amendment, and update CLAUDE.md and `PRODUCT.md` §21.
- **P9-soak.** The maintainer's week in dark mode (SC-008). Each report
  becomes a fixture.

**Merge condition** (FR-030): User Stories 1–6 are complete, and every
quickstart scenario passes.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| A 25th crate, `postio-render` | FR-001 and FR-023a must be **proven**, and a dependency graph is proven only at a crate root. It also makes rendering headless-testable. | Keeping it in `postio-gtk` mixes the graph with GTK, WebKit and GIO networking, which makes the proof impossible, and needs a display for every test. |
| A dedicated render thread per reader | The 400 ms bound and "never block the UI" together need layout off the UI thread, and Blitz has no cancellation. | Main-thread layout freezes the app on hostile input. A subprocess was rejected by the maintainer (FR-023a). |
| Postio-owned selection, find and a11y over a snapshot instead of Blitz's selection | Interaction must stay under 16 ms while a render runs. Copy, find and the screen reader must share one reading order. Blitz's `get_selected_text` flattens tables. | Querying the engine couples every gesture to the render thread and to Blitz's API. |
| Tiled painting | FR-022: no height cut-off, and memory independent of height. | The spike's single texture clipped at 30,000 px and cost about 96 MB. |
