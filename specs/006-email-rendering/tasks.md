# Tasks: Faithful, Readable Email Rendering

**Input**: Design documents from `specs/006-email-rendering/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md),
[research.md](./research.md), [data-model.md](./data-model.md),
[contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: **Included and non-negotiable** (Constitution IV). Every `[TEST]`
task is written first and must be **observed failing** before the task after
it. A test that was never red is tightened until it visibly constrains the
behaviour. It is never proven by re-breaking code that works.

**Workflow**: This is spec-driven, so there are **no issues** (CLAUDE.md,
*Spec-driven work*). The work happens in `~/src/postio-worktrees/email-rendering`
on `feature/email-rendering`, with one commit per task. Commits end
`Refs: specs/006-email-rendering` plus the task id. They never contain a
closing keyword. The branch lands once, with `scripts/issue-land.sh --detach`.

Work discovered along the way that is not in this list:
- **under ~10 minutes**: fix it here, as its own commit;
- **larger**: file it with `scripts/issue-file.sh`.

**Where things are tested.** Test at the cheapest layer that can fail:
- **sanitizer rules**: `cargo test -p postio-body --lib`;
- **rendering rules and pixels**: `cargo nextest run -p postio-render --test <suite>`,
  which is headless and needs no display;
- **the widget**: `crates/postio-gtk/tests/gtk_suite/`. A new case is a
  module plus a row in its `CASES` table;
- **wiring**: `crates/postio-app/tests/app_suite/`, again a module plus a row
  in `main.rs`'s `CASES`.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel, because it touches different files and depends
  on no incomplete task;
- **[Story]**: US1 to US6 from spec.md;
- **[TEST]**: must be seen red before the next task.

---

## Phase 1: Setup (evidence before engine)

**Purpose**: the corpus, the WebKit reference renders and the fidelity
metric. The reference renders **must** be captured while WebKit is still the
reader's engine, so this phase comes before anything is removed (research
R13).

- [X] T001 Confirm the worktree `~/src/postio-worktrees/email-rendering` is on `feature/email-rendering`, with `main` recorded in `$(git rev-parse --git-dir)/postio-base`. Run `scripts/install-nextest.sh` and `scripts/install-shims.sh`
- [X] T002 Add the designed-mail corpus fixtures using `/add-fixture`, each with its loader-table row, categories and README row in `crates/postio-model/tests/corpus/`. All are synthetic, with reserved domains and invented brands:
  - `html-designed-three-column.eml`, a three-column campaign with a button and a `cid:` hero image;
  - `html-transactional-receipt.eml`, a receipt with a totals table;
  - `html-responsive-media.eml`, which stacks under `@media (max-width: 600px)`;
  - `html-class-styled.eml`, whose `<style>` targets its own classes and ids (#1545)
- [X] T003 Add the legacy-client fixtures with `/add-fixture` in `crates/postio-model/tests/corpus/`:
  - `html-legacy-font-center.eml`: `<font color face size>`, `<center>`, and body `bgcolor`, `text` and `link`;
  - `html-legacy-table-attrs.eml`: `valign`, `cellpadding`, `cellspacing`, `<table border=1>`, `table align=center`, `img align=right`, and `background=` on a cell
- [X] T004 Add the theme fixtures with `/add-fixture` in `crates/postio-model/tests/corpus/`:
  - `html-white-page-reply.eml`: a desktop-client reply with body `bgcolor=#ffffff` and `color:#000`, and no other backgrounds;
  - `html-dark-aware.eml`: `<meta name="color-scheme" content="light dark">` plus class-based `@media (prefers-color-scheme: dark)` rules;
  - `html-dark-text-no-background.eml`: `color:#222` with no background anywhere;
  - `html-illegible-sender-dark.eml`: sender dark rules that set `#333` text on `#222`;
  - `html-transparent-logo.eml`: a `cid:` PNG with alpha, drawn for a white page
- [X] T005 Add the hostile fixtures with `/add-fixture` in `crates/postio-model/tests/corpus/`:
  - `html-deep-nesting.eml`: 400 nested `<div>`s;
  - `html-very-tall.eml`: about 40,000 px of content, with a unique last line;
  - `html-malformed-image.eml`: a truncated `cid:` PNG;
  - `html-oversized-image.eml`: a `cid:` PNG header declaring 20,000 × 20,000;
  - `html-svg-local-file.eml`: a `cid:` `image/svg+xml` part whose `<image href="file:///…">` names a local path;
  - `html-every-url-vector.eml`: `url()` in `background`, `list-style-image`, `cursor`, `border-image`, `content`, `@import`, `@font-face`, and a conditional `@media`;
  - `html-script-forms.eml`: `<script>`, `onload`, `javascript:` hrefs and a `<form>`

  The existing `html-tracking-pixel-remote-images.eml` and `html-escaping-styles.eml` are reused
- [X] T006 Add the international fixtures with `/add-fixture` in `crates/postio-model/tests/corpus/`:
  - `html-rtl-mixed.eml`: Arabic and Hebrew paragraphs, with Latin inline;
  - `html-cjk-emoji.eml`: Chinese, Japanese and Korean text, plus emoji
- [X] T007 Write the one-shot capture tool `crates/postio-gtk/examples/capture_reference.rs`, following `contracts/fidelity-metric.md` § Reference renders. It renders each designed fixture's **unsanitized** HTML in a WebKitGTK view with network, JavaScript and remote loads off, and serves `cid:` parts from the fixture itself. It uses 800 CSS px, scale 1, the light scheme, and the bundled faces as default families. The output is one PNG per fixture in `crates/postio-test-support/data/reference/`
- [X] T008 Run T007 by hand on the desktop (`cargo run -p postio-gtk --example capture_reference`). Check in the PNGs and `crates/postio-test-support/data/reference/README.md`, with one line per fixture giving the WebKitGTK version (`pkg-config --modversion webkitgtk-6.0`) and the date
- [X] T009 [TEST] Write the metric's own tests in `crates/postio-test-support/src/fidelity.rs`, each checked against `contracts/fidelity-metric.md`:
  - an image compared with itself matches;
  - a copy with one 70 px column filled with the background does **not** match;
  - a copy with ±1 px glyph jitter, simulated by a 1 px blur, matches;
  - a copy 10% taller does not match;
  - the diff image outlines exactly the changed cells
- [X] T010 Implement the metric in `crates/postio-test-support/src/fidelity.rs`: 16 px cells, OKLab means, ΔE_OK ≤ 0.08 on ≥ 92% of cells, heights within ±8%, and the diff-image writer. The constants are the contract's, with no others

**Checkpoint**: The corpus and references are checked in, and the metric is
proven on synthetic images. Nothing renders yet.

---

## Phase 2: Engine-neutral groundwork

**Purpose**: the sanitizer and document changes that improve rendering **whichever engine is chosen**. Both arms of the evaluation render this same input (research R0), and the change also improves today's WebKit reader. None of it touches an engine.

- [ ] T011 [TEST] In `crates/postio-body/src/sanitize.rs` unit tests, assert:
  - `class` and `id` survive;
  - a sender `class="postio-latest"` or `id="postio-x"` is refused with reason `Containment`;
  - `id`s are rewritten `m<scope>-<id>` per message, and `href="#id"` and `<label for>` follow;
  - two messages both using `id="header"` get distinct ids (FR-005, #1545)
- [ ] T012 Implement T011 in `crates/postio-body/src/sanitize.rs`: allow `class` and `id` in the ammonia builder, and add the attribute filter that refuses the `postio-` prefix and rewrites ids
- [ ] T013 [TEST] In `crates/postio-body/src/sanitize.rs` unit tests, assert:
  - `<body bgcolor="#fff" text="#222" link="#06c" style="background:#fafafa">` and `<html style=…>` produce a `Canvas { background, text, link }`;
  - `link` becomes a scoped `a { color }` rule;
  - `<meta name="color-scheme" content="light dark">` produces `color_scheme = LightDark` (FR-006)
- [ ] T014 Implement canvas lifting and `color-scheme` carrying in `crates/postio-body/src/sanitize.rs`, and emit the canvas in `crates/postio-ui/src/reader/document.rs` `contain_body_in`: as a style on `div.postio-body` plus `data-postio-color-scheme`. Add a `postio-ui` unit test asserting the emitted attributes
- [ ] T015 [TEST] In `crates/postio-body/src/sanitize.rs`, walk a new `const REFUSALS` table and assert:
  - every entry is actually refused by the sanitizer, one fixture snippet per entry;
  - every reason is `Containment`, `Privacy` or `NoScript`;
  - the existing `REFUSED`, `REFUSED_AT_RULES` and `REFUSED_UNITS` are reachable through it;
  - inline `<svg>` is listed with its reason (FR-005, 001 FR-019b)
- [ ] T016 Implement the `Refusal` type and `REFUSALS` table in `crates/postio-body/src/sanitize.rs`, and record every removal in the sanitized message's `refusals`
- [ ] T017 [TEST] [P] In `crates/postio-body/src/hints.rs` unit tests, one test per row of research R9's hint table:
  - `<font color face size=1..7>` → `<span style>`, using the legacy size scale;
  - `valign` → `vertical-align`;
  - `cellpadding` → `padding` on every cell;
  - `cellspacing` → `border-spacing`;
  - `<table border=N>` → table and cell borders;
  - `table align=center` → `margin-inline:auto`;
  - `img align=left/right` → `float`;
  - `background="cid:x"` on `td` → `background-image` resolved through the `cid:` rewriting;
  - `background="http://…"` from an unconsented sender → refused with reason `Privacy`
- [ ] T018 Implement `crates/postio-body/src/hints.rs` and call it from `crates/postio-body/src/sanitize.rs` before ammonia runs (FR-007)
- [ ] T019 [TEST] In `crates/postio-ui/src/reader/document.rs` unit tests, `body_html_in` for `html-newsletter.eml` (which `reads_as_bulk`) returns the **original** layout, with no reader-view reduction (FR-031)
- [ ] T020 Make every message open `Original` in `crates/postio-ui/src/reader/document.rs` (`sheet_for`, `suits_reader_view`). `reads_as_bulk` stays for the unsubscribe banner

**Checkpoint**: The sanitizer preserves and translates instead of deleting, and every message opens in its original layout. The spec's fidelity input is fixed before either engine is judged.

---

## Phase 3: Engine evaluation, Blitz or WebKit (decision gate)

**Purpose**: choose the engine on evidence (research R0). **No engine-specific task in any later phase starts until T029 records the maintainer's decision.** Evaluation harnesses live under `examples/`. They are measurement tools, not product code, so they are not test-first. The losing arm's harness is deleted in T029.

- [ ] T021 Write the evaluation protocol before either arm runs: `docs/notes/2026-09-26-blitz-or-webkit.md`, listed in `docs/engineering-notes.md`. It states:
  - gates G1–G3 and scores S1–S8, copied from research R0;
  - the fixtures each criterion uses;
  - the machine;
  - the decision rule.

  Commit it **before** any result exists, so that nothing moves to fit a result
- [ ] T022 Build arm A's harness, `crates/postio-gtk/examples/eval_webkit.rs`. It renders every designed, theme and hostile fixture through the **production pipeline** (Phase 2 sanitizer → `postio-ui` compose) in the shipped hardened `Reader`, at 800 px in light, dark and high contrast, to PNG. For each text run it also writes `{rect, color}` using Postio's isolated-world script (`Range.getClientRects`, `getComputedStyle`). It includes a prototype of research R10's classification and contrast repair in that script. It needs a display and is run by hand
- [ ] T023 Build arm B's harness, `crates/postio-render/examples/eval_blitz.rs`. It is a prototype-depth `postio-render`: the `blitz-*` `=0.3.0-beta.2` crates with image formats and SVG on, and fonts through `fontdb` (research R1, R3, R4). It renders the same fixtures, through the same pipeline, in the same themes, to PNG, and writes `{rect, color}` per text run from Blitz's layout. It includes a prototype of R10 over computed styles. It is headless. It depends on the `postio-render` skeleton, so do the skeleton task first (the first task of Phase 4) and nothing else from Phase 4
- [ ] T024 Score the gates for both arms, and write the results into the note:
  - **G1 (legibility):** pixel sampling behind every text run, from T022 and T023 output, counted before and after each arm's repair prototype;
  - **G2 (egress):** a loopback listener with a counted control, across the hostile fixtures, unconsented and consented;
  - **G3 (survival):** each hostile fixture opened in a live reader for arm A, and through the renderer for arm B. Record any crash, or any render longer than 400 ms
- [ ] T025 Score S1 (fidelity) for both arms with `postio_test_support::fidelity` against the reference PNGs, and write per-fixture results into the note. State the known bias: the reference is WebKit
- [ ] T026 Score S2 (cost) and S3 (blank frames) for both arms, using `postio-app`'s `pane_comparison` example extended with a Blitz arrangement: Pss, processes, first and warm render, handover at 2, 10 and 50 messages, and ground-only frames over 50 navigations. Use the #1348 method (`docs/notes/2026-09-08-what-a-thread-costs-in-two-panes.md`)
- [ ] T027 Score S4 (affordance parity) and S5 (accessibility quality):
  - an inventory of US4 and US6 against each arm: built in, or tasks still to build, counted from this file;
  - an AT-SPI walk of one message's body per arm with `accerciser` or the `atspi` crate, recording what a screen reader receives
- [ ] T028 Score S6 (security posture), S7 (maintenance risk) and S8 (platform reach), each with evidence:
  - for security: which code parses hostile input in each arm, and whether "no network" is structural or a setting, citing the `cargo tree` audit for Blitz and the WebKit sandbox for WebKit;
  - for maintenance: the upstream release cadence and open gaps;
  - for platform reach: what each arm means for macOS.

  Finish the note with the scorecard and a recommendation
- [ ] T029 Put the scorecard to the maintainer and record the decision as a Clarifications bullet in `specs/006-email-rendering/spec.md`.
  - **If Blitz:** mark research R0 decided; continue with Phase 4 below; promote T023's harness only as far as tasks ask; delete `crates/postio-gtk/examples/eval_webkit.rs`.
  - **If WebKit:** amend FR-001, FR-002, FR-023a and SC-006 as R0's table says. Re-run `/speckit-plan` and `/speckit-tasks` for the engine-specific phases, keeping every completed task. Delete `crates/postio-render` and T023's harness.

**Checkpoint**: The engine is decided and written down, and the spec,
plan and tasks agree with the decision.

---

## Phase 4: Foundational, Blitz arm (blocking prerequisites)

> **This phase and everything after it is the Blitz arm's plan.** It starts only if T029 records Blitz. If it records WebKit, these phases are re-planned first.

**⚠️ No user story starts before this phase is complete.** It builds:
- the crate and its two proofs;
- retirement of the three risks;
- fonts, resources and the render thread;
- the snapshot and the text index;
- the sanitizer changes every story needs (`class`/`id`, the canvas, the
  refusal list, the caps);
- a minimal `BodyView` that paints the snapshot.

### The crate and its proofs (FR-001, FR-023a)

- [ ] T030 Create the crate skeleton: `crates/postio-render/Cargo.toml` (edition 2024, no dependencies yet) and `crates/postio-render/src/lib.rs` with a crate doc stating it is GTK-free, network-free and C-free (spec FR-001, FR-023a). Add `crates/postio-render` to both `members` and `default-members` in the root `Cargo.toml`
- [ ] T031 Add the renderer's dependencies to `crates/postio-render/Cargo.toml` **exactly as the spike had them**:
  - `blitz-dom`, `blitz-html`, `blitz-paint` and `blitz-traits` `=0.3.0-beta.2`, with `default-features = false`;
  - `blitz-dom` features `floats`, `system-fonts` and `svg`;
  - `anyrender =0.13` and `anyrender_vello_cpu =0.17`.

  Add a stub in `crates/postio-render/src/lib.rs` that names one item from each dependency, so that `scripts/checks/check-unused-deps.py` stays green. **Do not commit yet**: T031 to T033 are one commit. The next task's check must see the spike's graph red
- [ ] T032 [TEST] Write `scripts/checks/check-renderer-is-memory-safe.py` to `contracts/renderer-graph-checks.md` § 2. It covers:
  - `links` outside the allowlist;
  - `-sys` packages;
  - `cc`, `cmake` or `bindgen` as a build dependency;
  - `image` decoder features;
  - `blitz-dom` `net`/`system-fonts`, and `fontique`/`parley` `system`;
  - `blitz-paint` without `svg`;
  - `panic = "abort"` in any profile.

  Each failure names its fix. **Observe it red** against T031's uncommitted graph: it must name `yeslogic-fontconfig-sys` and `blitz-paint` without `svg`
- [ ] T033 Fix `crates/postio-render/Cargo.toml` to the plan's feature set so that T032 goes green:
  - `blitz-dom` features `floats` and `svg` only;
  - `blitz-paint` feature `svg`;
  - `parley` without `system`;
  - `image` `0.25` with `png`, `jpeg`, `gif` and `webp`;
  - `fontdb`;
  - `usvg =0.48`

  Then commit T031 to T033 **together**, as one commit in which every check is green. The red state was observed but is never committed (CLAUDE.md: every commit is green for the crates it touches)
- [ ] T034 [TEST] Add `RULES["postio-render"]` to `scripts/checks/check-crate-boundaries.py`, using the banned list and `why` from `contracts/renderer-graph-checks.md` § 1. Walk normal and build edges only: add the `edges` key the contract describes, so the renderer's dev-dependencies (a socket and `postio-test-support`) are exempt. **Observe it bite**: add a scratch `tokio` dev-dependency to `crates/postio-render/Cargo.toml`, see the check fail naming `tokio`, then remove the scratch line. Committed green
- [ ] T035 [TEST] In `crates/postio-render/src/lib.rs`, declare `RenderRequest`, `RenderedDocument`, `TextIndex`, `MessageBox`, `LinkBox`, `FoldBox`, `Presentation`, `RenderCounts` and `FallbackReason`, following `data-model.md`. Add `static_assertions::assert_impl_all!(RenderedDocument: Send, Sync)`. Observe it red while the display-list field is a Blitz type, and green once the field is `anyrender::recording::Scene` (retires risk R6-a)

### Retire the image, SVG and `<details>` risks

- [ ] T036 [TEST] In `crates/postio-render/tests/images.rs`, render `inline-image-cid.eml` through a minimal `Renderer` path. Assert that the pixels inside the image's box are the image's own colours, not the placeholder. This reproduces the spike's unseen defect: no decoder format was ever enabled
- [ ] T037 Make T036 green: `image` features from T033, and the `cid:` resource resolving through a first minimal version of `crates/postio-render/src/resources.rs`
- [ ] T038 [TEST] In `crates/postio-render/tests/images.rs`, the test writes a temporary PNG of solid `#ff00ff`, then renders `html-svg-local-file.eml` with the SVG's `href` pointed at that path. Assert that no `#ff00ff` pixel is painted, and that the SVG's own shapes are painted (research R5)
- [ ] T039 Implement SVG re-serialisation in `crates/postio-render/src/resources.rs`: usvg parse with an `image_href_resolver` that keeps only `data:` rasters, then usvg's writer. Enable `blitz-paint/svg`
- [ ] T040 [TEST] In `crates/postio-render/tests/details.rs`, assert three things:
  - a closed `<details>` paints only its `<summary>`;
  - a request whose `open_folds` names that fold paints the body;
  - the two display lists differ only below the summary (risk R15-a)
- [ ] T041 Implement fold toggling in `crates/postio-render/src/lib.rs`: set `open` on the named `<details>` before layout, with the fold id taken from a stable attribute stamped by `postio-ui/src/reader/thread.rs`. If Blitz cannot toggle, use the Postio-owned attribute fallback in research R15, and record which was used

### Fonts, resources, the render thread

- [ ] T042 [TEST] In `crates/postio-render/tests/fonts.rs`, assert:
  - the bundled faces resolve by family name;
  - `serif`, `sans-serif` and `monospace` resolve;
  - shaping `html-cjk-emoji.eml` and `html-rtl-mixed.eml` produces **no glyph id 0** (tofu);
  - `FontSet` construction reads no file outside the fontdb-discovered set and the bundled bytes
- [ ] T043 Implement `crates/postio-render/src/fonts.rs`. `FontSet` registers ADR 0023's `FACES` first, then the `fontdb` discovery, with explicit generic families and per-script fallbacks for Latin, CJK, Arabic, Hebrew, Devanagari and emoji. It is built once, off the UI thread
- [ ] T044 [TEST] In `crates/postio-render/src/resources.rs` unit tests, assert:
  - a scope-A `cid:` never resolves for a scope-B reference (FR-004);
  - an unknown key resolves to nothing and increments `resources_unresolved`;
  - an image over 8,192 px a side, over 40 MP or over 16 MiB becomes `Placeholder { w, h }`;
  - `tiff`, `bmp`, `ico` and `avif` bytes become placeholders;
  - an animated GIF yields its first frame (FR-024, research R4)
- [ ] T045 Implement the resource table in `crates/postio-render/src/resources.rs`: keys from `data-model.md`, header-only dimension probes, and placeholders
- [ ] T046 [TEST] In `crates/postio-render/tests/thread.rs`, test containment and generations:
  - a request whose resource provider panics (a `#[cfg(test)]` hook) returns `outcome = FellBack { Panicked }` and does not unwind into the test;
  - the next request succeeds;
  - a stale-generation result is never delivered;
  - after `abandon(g)` on a request held by a test delay hook, the next request is served by a new thread
- [ ] T047 Implement `crates/postio-render/src/thread.rs` (research R6): one thread with a 64 MiB stack, `catch_unwind(AssertUnwindSafe)`, the document dropped after a panic, generations, taint and replacement, and `fallback()` for plain text. Also a panic hook that logs location and message id only, never the payload
- [ ] T048 [TEST] In `crates/postio-body/src/sanitize.rs` unit tests, check the input caps: a body over 50,000 elements, deeper than 256, or over 2 MiB sets `over_cap` with the cap named. In `crates/postio-render/tests/thread.rs`, a sanitized message with `over_cap` yields `FellBack { OverCap }` without Blitz parsing it (`counts.nodes == 0`)
- [ ] T049 Implement the caps in `crates/postio-body/src/sanitize.rs` and the short-circuit in `crates/postio-render/src/thread.rs`

### Snapshot, text index, tiles, egress

- [ ] T050 [TEST] In `crates/postio-render/tests/snapshot.rs`, render a three-message `postio-ui` conversation document and assert:
  - one `MessageBox` per container, in document order, with non-overlapping rects;
  - `LinkBox` targets are only `External` (http, https or mailto), `Verb` or `Fragment`;
  - `FoldBox`es exist for each thread `<details>`;
  - `counts.renders == 1` and `counts.style_passes ≤ 2`
- [ ] T051 Implement `crates/postio-render/src/snapshot.rs`: record the display list once with `blitz_paint::paint_scene` into `anyrender::recording::Scene`, build the message, link and fold boxes, rasterise the whole document once at 0.25 scale (capped at 16 MiB) as `low_res`, and fill `RenderCounts`
- [ ] T052 [TEST] In `crates/postio-render/tests/text_index.rs`, assert on the text of `html-transactional-receipt.eml`:
  - it contains its table as tab-separated cells and newline-separated rows;
  - a hidden preheader (`display:none`) is absent;
  - image `alt` text is present;
  - `hit()` at a known glyph's centre returns that glyph's offset;
  - `word_at` and `line_at` return the expected ranges;
  - `rects()` of a range lies inside the line's boxes;
  - `slice()` equals `text[range]`;
  - `find("TOTAL")` matches `Total` and `tötal` (case- and diacritic-folded)
- [ ] T053 [TEST] Write `crates/postio-render/tests/affordance_sweep.rs` (SC-007). It runs headlessly over **every** text-bearing corpus fixture, and asserts:
  - `slice(0..len)` equals `TextIndex.text`;
  - for a word taken from each fixture's own text, `find` returns at least one match, and every match's `rects()` lie inside a `MessageBox`;
  - `hit()` at the centre of every cluster returns an offset inside that cluster's range;
  - every `LinkBox` target is `External` (http, https or mailto), `Verb` or `Fragment`, and its rect lies inside its message's box.

  The widget tests in US4 then exercise one fixture each. This sweep is what makes "every text-bearing message in the corpus" true
- [ ] T054 Implement `crates/postio-render/src/text_index.rs`: a reading-order serializer over the laid-out tree (research R7) with cluster rects from parley geometry, plus `hit`, `word_at`, `line_at`, `rects`, `slice`, `find` and `char_at_top`
- [ ] T055 [P] [TEST] In `crates/postio-render/src/tile.rs` unit tests, assert that tiles rasterised over a document and stacked are byte-identical to one full raster of the same region
- [ ] T056 [P] Implement `rasterize_tile` in `crates/postio-render/src/tile.rs` from the recorded display list, callable from any thread
- [ ] T057 [TEST] Write `crates/postio-render/tests/egress.rs` to `contracts/renderer-graph-checks.md` § 3:
  - bind a loopback listener;
  - **control first**: connect from the test and assert the connection is counted, **seen red** before the listener's counter is wired;
  - rewrite every remote URL in every hostile fixture to the listener, and render each fixture in both the unconsented and the consented state;
  - assert zero accepted connections from rendering

### A minimal BodyView (every story's widget tests need it)

- [ ] T058 [TEST] Add `crates/postio-gtk/tests/gtk_suite/body_view.rs`, plus its row in `CASES`. Render a snapshot of `html-designed-three-column.eml` into a `BodyView`, then assert:
  - `vadjustment.upper == size.height`;
  - rendering the widget with `gtk::WidgetPaintable` at a known offset shows the fixture's hero-block colour at that point
- [ ] T059 Implement `crates/postio-gtk/src/body_view/mod.rs` and `tiles.rs` (research R8):
  - `gtk::Scrollable`, with a `Renderer` request on allocation;
  - 512 px tiles rasterised on a small pool;
  - an LRU tile cache capped at 64 MiB, with `gdk::MemoryTextureBuilder` and pooled buffers;
  - `snapshot()` appending only visible and prefetch tiles.

  Add `postio-render` to `crates/postio-gtk/Cargo.toml`
- [ ] T060 [TEST] In `gtk_suite/body_view.rs`, check frame continuity:
  - while a second request is outstanding, the widget's rendered pixels equal the previous frame, with no ground-only frame;
  - a tile evicted from the cache draws from `low_res` rather than the ground (FR-029)
- [ ] T061 Implement frame continuity in `crates/postio-gtk/src/body_view/mod.rs` and `tiles.rs`
- [ ] T062 Make the render deadline injectable:
  - export `DEFAULT_RENDER_DEADLINE: Duration = 400 ms` from `crates/postio-render/src/lib.rs`;
  - `BodyView::new` takes the deadline as a parameter, and production passes the default;
  - add one test helper, used by every `gtk_suite` and `app_suite` test that is not about the deadline itself, that builds readers with `postio_test_support::scaled(DEFAULT_RENDER_DEADLINE)`. The helper lives in `crates/postio-gtk/tests/gtk_suite/main.rs` and `crates/postio-app/tests/app_suite/main.rs`.

  This keeps a debug build on a busy CI runner from falling back by accident, while production keeps its 400 ms (research R6, `check-test-deadlines-scale.py`)
- [ ] T063 [TEST] In `gtk_suite/body_view.rs`, construct a `BodyView` with an **injected 1 ms deadline**, and hold its render with the delay hook until the test releases it. Assert that the widget shows the plain-text fallback with the notice text and a "View source" action. Then release the hook, and assert the late snapshot is not shown. No wall clock appears in the assertion (FR-023)
- [ ] T064 Implement the deadline timer, `abandon` and the fallback notice in `crates/postio-gtk/src/body_view/mod.rs`, using the injected length and `crates/postio-gtk/src/reader/notices.rs` for the notice
- [ ] T065 [TEST] In `gtk_suite/body_view.rs`, render `html-very-tall.eml` and scroll the adjustment to `upper - page_size` in 50 steps. Assert:
  - the unique last line's cluster rect is inside the viewport;
  - tile bytes held never exceed 64 MiB plus `low_res`, sampled at every step (FR-022)
- [ ] T066 Implement eviction and prefetch in `crates/postio-gtk/src/body_view/tiles.rs` until T065 is green
- [ ] T067 Add `RenderCounts` to the render counter in `crates/postio-ui/src/test_support/`, next to renders issued, surfaces created and bytes per document (001 T029/T030). `BodyView` reports each snapshot's counts to it

**Checkpoint**: Checks are green, risks are retired, the renderer produces
snapshots headlessly, and a `BodyView` paints them. The user stories can now
begin, in parallel where marked.

---

## Phase 5: User Story 1: every message is legible in dark mode (Priority: P1) 🎯 MVP

**Goal**: No text is ever dark on dark or light on light. Designed mail shows
as paper, with a per-message darken. Everything else adapts to the theme, with
a contrast floor asserted on painted pixels.

**Independent test**: `cargo nextest run -p postio-render --test contrast`
passes with zero clusters below the floor in the light, dark and
high-contrast themes (SC-001).

- [ ] T068 [TEST] [US1] In `crates/postio-gtk/tests/gtk_suite/body_view_theme.rs`, plus its row in `CASES`:
  - switching `adw::StyleManager` to dark issues exactly one new request with `theme.dark == true`, and one with `high_contrast` when that is toggled;
  - the text-index offset at the top of the viewport is the same before and after the switch;
  - no ground-only frame appears in between (FR-011, US1 scenario 4)
- [ ] T069 [US1] Implement the theme source in `crates/postio-gtk/src/body_view/mod.rs`: connect to `adw::StyleManager` `dark` and `high-contrast` with the handler ids disconnected on dispose, re-render on change, and restore the anchor
- [ ] T070 [TEST] [US1] Write a table test in `crates/postio-render/tests/presentation.rs` with the expected `Presentation` for each fixture:
  - `html-newsletter.eml` and `html-designed-three-column.eml` are `Paper` in dark;
  - `html-white-page-reply.eml` is `Adapted`;
  - `html-dark-text-no-background.eml` is `Adapted`;
  - `html-dark-aware.eml` is `SenderDark`;
  - `plain-text-simple.eml` is `Adapted`;
  - every fixture is `Styled` in light;
  - boundary cases: a canvas at relative luminance 0.91 is `Adapted`, and at 0.89 is `Paper` (FR-013)
- [ ] T071 [US1] Implement classification in `crates/postio-render/src/theme.rs`: a pure function over computed styles after the first style pass. The `SenderDark` detection reads `data-postio-color-scheme` or a scoped `prefers-color-scheme: dark` rule
- [ ] T072 [TEST] [US1] Write `crates/postio-render/tests/contrast.rs` (SC-001). Render **every** corpus fixture with an HTML or text body in the light, dark and high-contrast themes. For each text cluster, sample the rasterised pixels in the cluster rect's padding, excluding glyph pixels, and assert that `contrast(cluster.color, sampled) ≥ 4.5`, or `≥ 7` in high contrast, for text of **every** size. There is no large-text allowance (spec FR-012). **Observe red** on `html-white-page-reply.eml` and `html-dark-text-no-background.eml` in dark mode: that is today's bug
- [ ] T073 [US1] Implement the painted-ground walk and OKLCH L-only repair in `crates/postio-render/src/theme.rs`. Ancestors are composited over the canvas, and a `background-image` ground uses the decoded image's mean colour. Apply the repaired colours as highest-precedence `color` overrides, then restyle once, so `counts.style_passes == 2` exactly when `repaired_runs > 0`
- [ ] T074 [TEST] [US1] In `crates/postio-render/tests/contrast.rs`, `html-illegible-sender-dark.eml` in dark is `SenderDark`, and every cluster still meets the floor. The sender's dark styling is honoured, but not trusted
- [ ] T075 [TEST] [US1] In `crates/postio-render/tests/presentation.rs`, check paper in dark mode for `html-newsletter.eml`:
  - the container's box is painted with the sender canvas, or white if none, inset from the reader ground, with the reader radius;
  - the pixels outside the card are the reader ground;
  - the sender's text colours are unchanged, apart from floor repairs
- [ ] T076 [US1] Implement the paper card in `crates/postio-render/src/theme.rs`, and its style hook in `crates/postio-ui/src/reader/document.rs`: a container class set by presentation, never by the sender
- [ ] T077 [TEST] [US1] In `crates/postio-render/tests/presentation.rs`, test darken:
  - with `darkened` naming the newsletter's scope, every background's OKLab L is in [0.12, 0.30] with its hue kept within 2°;
  - every cluster meets the floor;
  - removing it from `darkened` restores a display list byte-identical to `Paper` (FR-013a)
- [ ] T078 [US1] Implement darken in `crates/postio-render/src/theme.rs`: `L → 0.12 + (1 − L) × 0.18` for backgrounds and borders, then repair
- [ ] T079 [TEST] [US1] In `crates/postio-render/tests/presentation.rs`, render `html-transparent-logo.eml` both `Darkened` and `Adapted`. Assert that the image's pixels equal its decoded pixels composited over the **sender's canvas colour**, never over the dark ground, and that no image pixel is recoloured (FR-015)
- [ ] T080 [US1] Implement the image canvas backing in `crates/postio-render/src/theme.rs`
- [ ] T081 [TEST] [US1] In `crates/postio-core/tests/core_suite/command_registry.rs`, `darken_message` exists with default `D` in message surfaces, is not destructive, and does not collide with any binding in an overlapping context
- [ ] T082 [US1] Add `DarkenMessage` in `crates/postio-core/src/command.rs` (the `command_ids!` entry, the variant and both conversion matches) and its `CommandSpec` in `crates/postio-core/src/registry.rs`, per `contracts/registry-commands.md`. Regenerate `docs/keybindings.md` with `POSTIO_UPDATE_DOCS=1`, and update the golden `linux-bindings.txt` by hand
- [ ] T083 [TEST] [US1] In `crates/postio-gtk/tests/gtk_suite/body_view_theme.rs`, in dark mode with `html-newsletter.eml` focused:
  - dispatching `darken_message` issues a request whose `darkened` contains its scope;
  - the command's title reads "Show as sent";
  - dispatching it again restores "Darken this message" and the `Paper` request
- [ ] T084 [US1] Handle `darken_message` in `crates/postio-gtk/src/reader/view.rs` and `crates/postio-gtk/src/body_view/mod.rs`, with a session-only set of darkened scopes and the dynamic title. Add the context-menu entry
- [ ] T085 [US1] Design the dark reader palette (#1588, FR-016) with `/ux-architect` and then `/gtk-design`, against `Design/Mail Client.dc.html`. Record the named roles in the design canvas's source, alongside the light ones
- [ ] T086 [TEST] [US1] In `crates/postio-gtk/tests/logic_suite/reader_tokens.rs`, assert that the dark reader roles equal the designed values recorded by T085, and that `--r-ground` no longer equals `neutral-900`
- [ ] T087 [US1] Replace `reader_dark_roles` in `crates/postio-ui/src/tokens.rs` with the designed roles, and regenerate `crates/postio-ui/data/reader-tokens.css`. `BodyView` builds `Theme.palette` from them in `crates/postio-gtk/src/body_view/mod.rs`

**Checkpoint**: US1 is independently proven. The contrast test is green over
the whole corpus in every theme, and darken round-trips.

---

## Phase 6: User Story 2: see the message the way its sender built it (Priority: P1)

**Goal**: Columns, colours, buttons and spacing appear where the sender put
them. Legacy markup is honoured, every message opens in its original layout,
and Reader view is opt-in.

**Independent test**: `cargo nextest run -p postio-render --test fidelity`.
At least 95% of the designed fixtures match their WebKit reference, and every
mismatch is listed as cosmetic (SC-002).

- [ ] T088 [TEST] [P] [US2] In `crates/postio-render/tests/layout.rs`, render `html-responsive-media.eml` at 500 px and at 800 px pane widths. Assert that the columns stack at 500 px (one `x` origin for the column boxes) and sit side by side at 800 px (FR-008)
- [ ] T089 [US2] Make T088 green: take the viewport width from the request's `width_px`, and verify the `@media` evaluation in `crates/postio-render/src/lib.rs`
- [ ] T090 [TEST] [P] [US2] In `crates/postio-render/tests/layout.rs`, a message containing `<head><title>Secret title</title><style>…</style></head>` has no "Secret title" in `TextIndex.text` and no painted box for `head` descendants (FR-010)
- [ ] T091 [US2] Add the UA rule `head, title, meta, link, style, script { display: none }` to the renderer's user stylesheet in `crates/postio-render/src/lib.rs`
- [ ] T092 [TEST] [US2] In `crates/postio-render/tests/layout.rs`, render `html-class-styled.eml`. The element targeted by the sender's `.cta` rule is painted in the rule's background colour, which proves the classes survive end to end (#1545)
- [ ] T093 [TEST] [US2] Write `crates/postio-render/tests/fidelity.rs` (SC-002). For every designed fixture that has a reference PNG, render it through the production pipeline (sanitize → `postio-ui` compose → render) at 800 px, light theme, zoom 100, and compare with `postio_test_support::fidelity`. Assert the pass rate is ≥ 95% and that `crates/postio-test-support/data/reference/MISMATCHES.md` lists every non-matching fixture as `cosmetic`
- [ ] T094 [US2] Run T093 for the first time and write `crates/postio-test-support/data/reference/MISMATCHES.md` with the cause of each mismatch. If the metric's constants are wrong for reality, change them **once**, with the evidence in research R13, before judging any fix (`contracts/fidelity-metric.md`)
- [ ] T095 [US2] Close the mismatch gaps until T093 is green. Sanitizer hints in `crates/postio-body/src/hints.rs` come first. A `[patch.crates-io]` `blitz-dom` fork pinned to a git revision is allowed only if a gap cannot be closed on Postio's side, and it is recorded in research R1 with the upstream PR it tracks
- [ ] T096 [P] [US2] Replace `::-webkit-details-marker` in `crates/postio-ui/data/thread.css` with standard `summary::marker` or `list-style`, and add a `postio-ui` unit test asserting that no `-webkit-` token remains in the reader stylesheets
- [ ] T097 [TEST] [US2] In `crates/postio-core/tests/core_suite/command_registry.rs`, `toggle_reader_view` exists with `mod+shift+o` in message surfaces and collides with nothing in an overlapping context. If the conflict test counts the composer's binding as overlapping, use the contract's fallback key `mod+alt+o`
- [ ] T098 [US2] Add `ToggleReaderView` in `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`, regenerate `docs/keybindings.md`, and update `linux-bindings.txt` by hand
- [ ] T099 [TEST] [US2] In `crates/postio-gtk/tests/gtk_suite/body_view.rs`, in a two-message conversation, `toggle_reader_view` reduces only the focused message: its request has that scope in `reader_view`, and the neighbour keeps its layout. `view_original` restores it
- [ ] T100 [US2] Handle `toggle_reader_view` in `crates/postio-gtk/src/reader/view.rs` with a session-only set of reduced scopes, and add the context-menu entry
- [ ] T101 [P] [US2] Fix the stale comments that say sender style or `class` are stripped: `crates/postio-body/src/sanitize.rs:9-13`, `crates/postio-ui/data/reader.css:24-26` and `118-121`, and `crates/postio-ui/src/reader/document.rs:609-611`

**Checkpoint**: US2 is independently proven. The fidelity test is green, and
bulk mail opens as sent.

---

## Phase 7: User Story 3: rendering cannot betray the reader (Priority: P1)

**Goal**: Hostile mail is contained. It makes no connection, runs no script,
causes no crash or hang, and nothing escapes a message's box.

**Independent test**: `cargo nextest run -p postio-render --test hostile` and
`--test egress` are green (SC-003, SC-004).

- [ ] T102 [TEST] [US3] Write `crates/postio-render/tests/hostile.rs`. For every hostile fixture from T005, plus `html-escaping-styles.eml` and `html-tracking-pixel-remote-images.eml`, assert that the render:
  - either returns `Rendered` with every painted box inside its `MessageBox` rect (001 FR-020, 001 FR-021), or returns `FellBack` with the expected reason;
  - finishes within `postio_test_support::scaled(DEFAULT_RENDER_DEADLINE)`: the production bound, reachable by the `POSTIO_TEST_PATIENCE` dial, as a containment assertion and not a performance gate;
  - produces no `LinkTarget` other than http, https, mailto, a verb or a fragment;
  - leaves `counts.resources_unresolved` counting every refused remote reference
- [ ] T103 [US3] Fix every T102 failure at the layer that owns it: the sanitizer (`crates/postio-body/src/sanitize.rs`), the resources (`crates/postio-render/src/resources.rs`) or containment CSS (`crates/postio-ui/data/reader.css`). Each fix is its own commit, naming the fixture
- [ ] T104 [TEST] [US3] Add `crates/postio-app/tests/app_suite/hostile_mail.rs`, plus its `CASES` row. Open each hostile fixture from the message list, and assert that the reading pane shows either a painted body or the fallback notice, the app keeps responding (the next keystroke is handled), and the process is alive
- [ ] T105 [US3] Wire the fallback notice's "View source" action to the existing original-source path (001 FR-027) in `crates/postio-gtk/src/reader/notices.rs`

**Checkpoint**: US3 is independently proven.

---

## Phase 8: User Story 4: read the way you read everywhere else (Priority: P2)

**Goal**: Selection and copy, find, links, folds, a screen reader, and the
script-free rail and scrolling all work in the new reader.

**Independent test**: `cargo nextest run -p postio-gtk --test gtk_suite body_view`
cases for select, find, links and a11y are green (SC-007).

- [ ] T106 [TEST] [US4] Add `crates/postio-gtk/tests/gtk_suite/body_view_select.rs`, plus its `CASES` row. On `html-transactional-receipt.eml`:
  - a synthetic drag from one cell to a cell in the next row selects that range, and the highlight rects are drawn;
  - the `Ctrl+C` clipboard text equals `TextIndex.slice`, with a tab between the cells and a newline between the rows;
  - the primary clipboard is set on selection;
  - a double click selects a word, and a triple click selects a line (FR-017)
- [ ] T107 [US4] Implement selection in `crates/postio-gtk/src/body_view/interact.rs`: `GtkGestureDrag` and `GtkGestureClick` over `TextIndex`, highlights as `append_color` overlays, the clipboard and primary clipboard, and autoscroll near the edges
- [ ] T108 [TEST] [US4] In `crates/postio-gtk/tests/gtk_suite/body_view_select.rs`, check links:
  - hovering a link sets the widget's tooltip to its real target;
  - `Tab` moves focus through the links in document order;
  - `Return` on an `https` link calls the launcher, captured by a test launcher;
  - a `javascript:` link from `html-script-forms.eml` is not a link at all;
  - verb links dispatch their `MessageVerb` (FR-019)
- [ ] T109 [US4] Implement links in `crates/postio-gtk/src/body_view/interact.rs`: hover target, keyboard focus ring drawn as an overlay, `gtk::UriLauncher` for `External` targets, and verb dispatch through the existing `connect_message_action`
- [ ] T110 [TEST] [US4] In `crates/postio-gtk/tests/gtk_suite/body_view_select.rs`, clicking a quote fold's summary rect issues a request with that fold in `open_folds`, and the fold body's text appears in the next snapshot's `TextIndex`
- [ ] T111 [US4] Implement fold clicks in `crates/postio-gtk/src/body_view/interact.rs`
- [ ] T112 [TEST] [US4] In `crates/postio-core/tests/core_suite/command_registry.rs`, check `find_in_message` (`mod+f`), `find_next` (`mod+g`, alternate `F3`) and `find_previous` (`mod+shift+g`, alternate `shift+F3`) against `contracts/registry-commands.md`, including the context-overlap rule
- [ ] T113 [US4] Add the three find commands in `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`, regenerate `docs/keybindings.md`, and update `linux-bindings.txt` by hand
- [ ] T114 [TEST] [US4] Add `crates/postio-gtk/tests/gtk_suite/body_view_find.rs`, plus its `CASES` row:
  - `mod+f` opens the find bar with focus in its entry;
  - typing "total" highlights exactly `TextIndex.find("total").len()` rects;
  - `mod+g` makes the next match current and scrolls it into view;
  - `mod+shift+g` goes back;
  - `Escape` closes the bar and clears the highlights;
  - the highlights survive a theme-change re-render (FR-018, FR-021f)
- [ ] T115 [US4] Implement `crates/postio-gtk/src/body_view/find.rs`: a `GtkSearchBar` in the reading pane, `FindState` over `TextIndex`, the overlay rects, and scroll-to-current
- [ ] T116 [TEST] [US4] Add `crates/postio-gtk/tests/gtk_suite/body_view_a11y.rs`, plus its `CASES` row, running under `GTK_A11Y=test`:
  - `AccessibleText::contents(0, -1)` equals `TextIndex.text`;
  - the caret and selection reflect `Selection`;
  - `extents(range)` equals the union of the cluster rects converted to widget coordinates;
  - the widget's role is `Document` (FR-020)
- [ ] T117 [US4] Implement `AccessibleTextImpl` in `crates/postio-gtk/src/body_view/a11y.rs` (gtk4 `v4_16` APIs, covered by the workspace's `v4_20`). Call `update_contents`, `update_caret_position` and `update_selection_bound` on every snapshot and selection change
- [ ] T118 [TEST] [US4] Port the `crates/postio-gtk/tests/gtk_suite/gtk_rail.rs` cases that assert the current message and scroll-to-message so they drive `BodyView`:
  - the current message is the one with the greatest visible area of `MessageBox`, computed by `RenderedDocument::current_message`;
  - activating a rail row scrolls that message's top to the viewport's top;
  - page down and page up move one real page (001 FR-034 to 001 FR-037)
- [ ] T119 [US4] Implement the script-free rail and scrolling in `crates/postio-gtk/src/body_view/mod.rs` and `crates/postio-gtk/src/conversation.rs` from `message_extents` and the vadjustment. The rail's observer script (`RAIL_HANDLER`), `SCROLL_REPORTER` and the `#pos-N` markers are no longer used on the `BodyView` path; they are deleted in T143

**Checkpoint**: US4 is independently proven.

---

## Phase 9: User Story 6: zoom in and out of a message (Priority: P2)

**Goal**: Browser-style zoom of the body only, in fixed steps, keeping the
reader's place and persisted in `[reader]`.

**Independent test**: `cargo nextest run -p postio-render --test zoom` and the
`body_view_zoom` gtk_suite case are green (SC-009).

- [ ] T120 [TEST] [P] [US6] In `crates/postio-config` tests, including the config-doc drift entry in `crates/postio-config/tests/config_suite/config_doc.rs`:
  - `[reader] zoom` defaults to 100;
  - `zoom = 112` loads as 110, and `zoom = 5` loads as 50;
  - a round trip preserves the value
- [ ] T121 [US6] Add `crates/postio-config/src/reader.rs`, with `ReaderConfig { zoom }`, a serde default and clamping, to `Config` in `crates/postio-config/src/lib.rs`. Add its `patch_reader`, the `ConfigChanged` field, the `docs/config.md` row, and the FFI mirror in `crates/postio-ffi/src/settings.rs`
- [ ] T122 [TEST] [P] [US6] In `crates/postio-core/tests/core_suite/command_registry.rs`, check `zoom_in` (`mod+plus`, alternates `mod+equal` and `mod+KP_Add`), `zoom_out` (`mod+minus`, `mod+KP_Subtract`) and `zoom_reset` (`mod+0`, `mod+KP_0`) against `contracts/registry-commands.md`
- [ ] T123 [US6] Add the three zoom commands in `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`, regenerate `docs/keybindings.md`, and update `linux-bindings.txt` by hand
- [ ] T124 [TEST] [US6] Write `crates/postio-render/tests/zoom.rs` (SC-009). For every text-bearing corpus fixture at every step from 50 to 300%:
  - no cluster lies outside the document width unless it is inside a scrolling box, meaning nothing is lost off the side;
  - every cluster's text appears in `TextIndex.text`, meaning nothing is clipped;
  - `html-responsive-media.eml` at 200% at an 800 px pane stacks its columns, because the effective width is 400 CSS px (FR-021a)
- [ ] T125 [US6] Carry `viewport.zoom` and `hidpi_scale` separately into Blitz's `Viewport` in `crates/postio-render/src/lib.rs`. They are never folded together (research R11)
- [ ] T126 [TEST] [US6] Add `crates/postio-gtk/tests/gtk_suite/body_view_zoom.rs`, plus its `CASES` row:
  - `mod+plus` twice goes 100 → 110 → 125;
  - the header's and message list's allocated sizes are unchanged;
  - `char_at_top` before and after each step is the same cluster;
  - Ctrl+scroll moves one step per notch, and the anchor is the cluster under the pointer;
  - at 300%, `zoom_in` changes nothing and raises no error;
  - the indicator is visible iff zoom ≠ 100, and its reset button dispatches `zoom_reset`;
  - a selection made before zooming is intact after it (FR-021 to FR-021f)
- [ ] T127 [US6] Implement `crates/postio-gtk/src/body_view/zoom.rs`: the steps, anchors (top for keys, pointer for scroll), the `GtkEventControllerScroll` with Control, the indicator overlay with reset, and the command handlers
- [ ] T128 [TEST] [US6] In `crates/postio-gtk/tests/gtk_suite/body_view_zoom.rs`, test pinch: emit `GtkGestureZoom` scale changes of 1.3 then end. During the gesture, existing tiles are drawn scaled and no request is issued. On end, exactly one request is issued at the nearest step to 130%, which is 125%
- [ ] T129 [US6] Implement pinch in `crates/postio-gtk/src/body_view/zoom.rs`: a transient visual scale while the gesture runs, then snap and re-render on end
- [ ] T130 [TEST] [US6] Add `crates/postio-app/tests/app_suite/zoom_persists.rs`, plus its `CASES` row. `zoom_in` writes `[reader] zoom = 110` through the config writer. A new reader created from the same config starts at 110. A live edit of the file to 150 applies to the open reader
- [ ] T131 [US6] Wire zoom persistence and the live config reload into `crates/postio-app` and `crates/postio-gtk/src/reader/view.rs`

**Checkpoint**: US6 is independently proven.

---

## Phase 10: User Story 5: show a trusted sender's remote images (Priority: P3)

**Goal**: The app, not the renderer, fetches allowed senders' remote images,
with no identifying headers, in memory only, and with no layout jump when they
arrive.

**Independent test**: `cargo nextest run -p postio-app --test app_suite remote_images_allowed`
is green, in both directions.

- [ ] T132 [TEST] [US5] In `crates/postio-runtime/src/remote_images.rs` unit tests, against a loopback HTTP server:
  - a PNG is fetched and cached in memory;
  - the request has no `Cookie`, `Referer`, `Origin` or `Authorization`, and a `User-Agent` without "postio";
  - a 4th redirect, a non-http(s) redirect, a response over 16 MiB, a response over the 10 s timeout, and an HTML body labelled `image/png` each yield their `Failed` reason;
  - at most 4 fetches are in flight at once for one message (`contracts/remote-image-fetch.md`)
- [ ] T133 [US5] Implement `RemoteImageFetcher` in `crates/postio-runtime/src/remote_images.rs` on `io-http` + `pimalaya-stream` over `postio-transport` TLS. The cache is in memory only, and the code carries a `POSTIO-CONSENT:` marker for `scripts/checks/check-no-silent-tracking.py`. Logs carry counts and outcomes only, never a URL
- [ ] T134 [TEST] [US5] In `crates/postio-render/tests/layout.rs`, render `html-tracking-pixel-remote-images.eml`, whose images declare their sizes, with the images missing and then present in the resource table. Every `MessageBox` and every non-image cluster rect is identical between the two renders (FR-026)
- [ ] T135 [US5] Size placeholders from their declared attributes and styles in `crates/postio-render/src/resources.rs` and `crates/postio-render/src/snapshot.rs`
- [ ] T136 [TEST] [US5] Add `crates/postio-app/tests/app_suite/remote_images_allowed.rs`, plus its `CASES` row, following the #1336 two-direction discipline:
  - an allowed sender's message renders first with placeholders; the image then arrives from the loopback listener and is painted, asserted on the rendered pixels;
  - "Show once" on an unallowed sender fetches for that message only;
  - with consent revoked, the listener's control connection is counted, then opening the message counts zero;
  - moving the list cursor across ten allowed senders' **unopened** messages counts zero;
  - offline, meaning the listener is closed, the message renders at once with placeholders;
  - an **allowed** sender's `html-every-url-vector.eml` fetches only its image URLs. The listener sees no request for a font, a stylesheet import, a cursor or any other non-image resource (FR-025)
- [ ] T137 [US5] Wire the fetcher to the reader's owner in `crates/postio-app` and `crates/postio-gtk/src/reader/view.rs`. Fetch on open, or on "Show once", for `Allowed` senders only. Add arrivals to that message's resource table, and coalesce re-renders to at most one per frame

**Checkpoint**: US5 is independently proven. All six stories are complete,
which is the merge condition in FR-030.

---

## Phase 11: The switch, governance and polish

**Purpose**: Make `BodyView` the only reading engine, delete the reader's
WebKit path, record the decisions, and land.

- [ ] T138 [TEST] Port the reading-path `app_suite` cases in `crates/postio-app/tests/app_suite/` (`click_preview`, `cursor_preview`, `reader_loads`, `reader_stability`, `one_document_conversation`, `body_arrives`, `conversation_body_arrives`, `render_dedup`, `reclaim_pages`) so they assert through `BodyView`'s snapshot and render counts instead of `test_document` and WebKit `loads`. Each is seen red against the WebKit reader it no longer drives
- [ ] T139 Make `BodyView` the body of `Reader` in `crates/postio-gtk/src/reader/view.rs`. `Reader` keeps its native chrome: header, actions, notices and attachments. In `crates/postio-gtk/src/conversation.rs`, replace `reader.view().connect_load_changed(… Finished)` with the snapshot-arrived signal and drop `use webkit6::prelude::WebViewExt`. In `crates/postio-gtk/src/window.rs`, `view().grab_focus()` becomes `widget().grab_focus()`
- [ ] T140 Port the behaviour cases of `crates/postio-gtk/tests/gtk_suite/` to `BodyView`: `gtk_reader_styles`, `gtk_reader_anchor`, `gtk_reader_fonts`, `gtk_reader_notices`, `gtk_reader_teardown`, `gtk_search_preview` and `gtk_window_teardown`. Search highlighting through `crate::search::mark_html` must still show. Each deleted WebKit-mechanics assertion is named in its commit together with its replacement (research R14):
  - web-process counting → T144;
  - CSP and JavaScript refusal → T032, T034 and T102;
  - `webkit_probe` computed styles → the `postio-render` style tests
- [ ] T141 Port the egress proofs in `crates/postio-gtk/tests/gtk_reader.rs`, including #1336's style-borne beacon, to the new reader. Keep their loopback listeners and their two-direction controls. Delete the sub-cases that exist only to test WebKit mechanics, naming their replacements
- [ ] T142 [P] Remove `crates/postio-gtk/tests/webkit_probe.rs` and its use in `crates/postio-gtk/tests/gtk_suite/main.rs` once nothing reads it
- [ ] T143 Delete the reader's WebKit path:
  - `crates/postio-gtk/src/reader/scheme.rs`;
  - in `crates/postio-gtk/src/reader/view.rs`: `build_view`, `hardened_settings`, `RAIL_HANDLER`, `SCROLL_HANDLER`/`SCROLL_REPORTER`/`Place`, `scroll_to_fragment`, `paint_ground`, `Reader::view()`, and the `page_for_test`/`document_for_test` hooks that return WebKit objects;
  - the reader's `web_process::watch` call.

  `crates/postio-gtk/src/web_process.rs` stays as long as the composer (`editor.rs`) uses it. Proof: `grep -rn webkit6 crates/postio-gtk/src/reader crates/postio-gtk/src/conversation.rs` returns nothing
- [ ] T144 [TEST] Add `crates/postio-app/tests/app_suite/reader_spawns_no_web_process.rs`, plus its `CASES` row. With the composer closed, opening ten conversations in turn spawns **no** WebKit web process, counted as the old `web_process` counters counted. Each navigation's `RenderCounts` show one render and at most two style passes. After the tenth navigation, the tile-cache bytes and the number of live `RenderedDocument`s equal those after the first: memory does not grow with messages viewed (SC-006)
- [ ] T145 [P] Write `docs/decisions/0042-the-reading-renderer-is-disconnected-and-memory-safe.md`, and add it to `docs/decisions/README.md`. It holds only the boundary rules: no network crate, no C and no fontconfig in `postio-render`'s graph; remote bytes only through the app's fetcher; `panic = "unwind"` kept. It cites the two checks (research R18)
- [ ] T146 [P] Amend `docs/decisions/0032-*.md`: its decision, one document per conversation, stands, and its mechanism, one `WebView`, is superseded by ADR 0042 and `postio-render`. Amend `docs/decisions/0003-rich-text-compose.md`'s reader statements to point at 0042
- [ ] T147 Amend `.specify/memory/constitution.md` Principle VI: "the reader's WebKit view has JavaScript and network off" becomes "the reader's renderer cannot run script or reach the network (ADR 0042)". This is a PATCH bump with its rationale in the Sync Impact Report. Make the same wording change in `CLAUDE.md` (*Privacy is a feature*) and `docs/PRODUCT.md` §21
- [ ] T148 [P] Add `docs/notes/2026-09-XX-the-reader-renders-without-a-display.md`, dated on the day it is written, and list it in `docs/engineering-notes.md`. The constraint it records: reader layout and pixels are asserted in `postio-render` tests, headlessly, and the old "zero `getBoundingClientRect`" wall no longer applies to the reader
- [ ] T149 [P] Add `crates/postio-bench/benches/reader_render.rs` (SC-005). It measures the warm render of each designed corpus fixture, meaning a second request on a live `Renderer`, and the first tile's rasterisation. `bench.yml` compiles it nightly. The numbers are reported and do not gate (Constitution V: timings report, counts gate). The gate for SC-005 remains T067's and T144's counts
- [ ] T150 Time the `postio-render` suites: `cargo nextest run -p postio-render`, warm. If `fidelity`, `contrast` or `zoom` push a landing past the four-minute budget, move that module to the nightly tier: a `//! POSTIO-MEASUREMENT:` marker, exclusion in `.config/nextest.toml`'s `profile.default` `default-filter`, and `check-measurement-tier.py` green. Never `#[ignore]`
- [ ] T151 Run every scenario in `specs/006-email-rendering/quickstart.md`, and record each outcome in the PR body
- [ ] T152 Land with `scripts/issue-land.sh --detach`. The PR body is reviewed against `spec.md`, carries `Closes:` lines for #1543, #1545, #1547, #1588 and #1501, and says the branch is spec-driven with no per-task issues
- [ ] T153 After the merge, the SC-008 soak: the maintainer uses the app in dark mode for a week. Each message reported unreadable becomes a corpus fixture through `/add-fixture` and a red case in `crates/postio-render/tests/contrast.rs`, fixed on a `fix/` branch. It is never answered with a workaround

---

## Dependencies & execution order

```text
Phase 1  Setup (T001–T010)              corpus, WebKit references, fidelity metric
        │
Phase 2  Engine-neutral (T011–T020)     sanitizer keeps/translates; original layout by default
        │
Phase 3  Engine evaluation (T021–T029)  both arms on the same input → scorecard → maintainer decides
        │
        ├── Blitz chosen ─────────────────────────────────────────────────────────────┐
        │                                                                             │
        │  Phase 4  Foundational, Blitz arm (T030–T067)                               │
        │           crate + proofs → risks → fonts/resources/thread                   │
        │           → snapshot/text/tiles → BodyView                                  │
        │     ├──► US1 (T068–T087)  P1  🎯 MVP                                        │
        │     ├──► US2 (T088–T101)  P1                                                │
        │     ├──► US3 (T102–T105)  P1                                                │
        │     ├──► US4 (T106–T119)  P2                                                │
        │     ├──► US6 (T120–T131)  P2                                                │
        │     └──► US5 (T132–T137)  P3                                                │
        │  Phase 11 Switch & polish (T138–T153)   needs all six stories (FR-030)      │
        │                                                                             │
        └── WebKit chosen: T029 amends the spec, and /speckit-plan and /speckit-tasks
            re-plan Phases 4–11 before any of them starts. T001–T029 stand.
```

- **Setup comes first.** The reference captures (T007, T008) happen while
  WebKit is the reader, and they are the fidelity bar for either arm.
- **Phase 2 comes before the evaluation.** Both arms must render the same
  improved input. Otherwise the evaluation compares an old sanitizer with a
  new one.
- **Phase 3 is a hard gate.** T021 (the protocol) is committed before T022
  and T023 produce any result. T023 needs T030 (the `postio-render`
  skeleton), and that is the only Phase 4 task allowed before T029.
- **Inside Phase 4:**
  - T031 → T032 → T033 land as one commit. The check is seen red against the
    spike's uncommitted graph, and only the green state is committed;
  - T035 comes before T050;
  - T036 to T041 retire the risks before T050 to T054 build on them;
  - T062 (the injectable deadline) comes before T063, and before every later
    widget or app test that builds a reader.
- **The stories are independent of each other** once Phase 4 is done. US2's
  fidelity test (T093) is judged in the light theme only, so it does not
  wait for US1.
- **Inside each story**, every `[TEST]` comes before its implementation task.
  The registry tasks (T081/T082, T097/T098, T112/T113, T122/T123) all edit
  `command.rs`, `registry.rs` and `docs/keybindings.md`, so they never run in
  parallel with each other.
- **Phase 11** needs every story complete (FR-030). T138 comes before T139,
  and T139 comes before T140 through T143.

## Parallel opportunities

- **Setup**: T002 to T006 go through `/add-fixture`, which edits the shared
  README and loader, so they run sequentially. T007 can be written alongside
  them.
- **Phase 2**: the hint tests and implementation (T017, T018,
  `postio-body/src/hints.rs`) run alongside the `sanitize.rs` tasks
  (T011–T016). T019 and T020 (`postio-ui`) run alongside both.
- **Phase 3**: the two arms' harnesses (T022 for WebKit, T023 for Blitz) are
  built in parallel. Their measurements then run one after the other on one
  machine, so they do not disturb each other.
- **Phase 4**: T055/T056 (tiles) run alongside T052 to T054 (text index).
- **After Phase 4**, the stories can run side by side where the files do not
  overlap:
  - US1 is mostly `postio-render/src/theme.rs` plus tokens;
  - US2 is `postio-render/tests/layout.rs` and `fidelity.rs`;
  - US3 is `postio-render/tests/hostile.rs`;
  - US6 is config, zoom and `body_view/zoom.rs`;
  - US5 is `postio-runtime` plus `app_suite`.
- **Examples**:
  - US2: T088 (responsive) and T090 (head) are `[P]`;
  - US6: T120 (config) and T122 (registry) are `[P]` with each other, but not
    with other stories' registry tasks;
  - Phase 11: T142, T145, T146, T148 and T149 are `[P]`.

## Implementation strategy

- **First: Setup, Phase 2 and the evaluation.**
  - **What is proven:** everything that holds whichever engine wins, plus the
    evidence to choose between the engines.
  - **What else:** Phase 2 alone improves today's reader. Classes survive,
    the canvas is kept, the legacy markup is translated, and newsletters open
    as sent.
- **Then, if Blitz: MVP = Phase 4 + US1.**
  - **What is proven:** the dark-on-dark bug is gone, as asserted on painted
    pixels over the whole corpus, and the renderer's disconnection and memory
    safety are proven by checks.
  - **What is not usable yet:** nothing ships, because the branch lands once
    and FR-030 makes all six stories the merge condition.
  - **Then**, in order: US2 and US3, which complete P1; then US4 and US6
    (P2); then US5 (P3); then the switch.
- **If WebKit:** the plan and tasks are regenerated for the engine-specific
  phases. The P1 → P2 → P3 story order stands.
- **Rebase** `feature/email-rendering` onto `main` as the branch goes. Other
  sessions change the reader, the registry and the corpus. A registry
  conflict is resolved by regenerating `docs/keybindings.md`, not by merging
  it by hand.
- **Commit every task.** A work-in-progress commit marked as such is better
  than loose files.
