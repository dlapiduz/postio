# Implementation Plan: The Compose Editor

**Branch**: `002-compose-editor` | **Date**: 2026-09-10 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-compose-editor/spec.md`

## Summary

79 requirements over a composer that mostly exists. The work divides three ways,
and the division is the plan:

1. **One governance item, first and blocking.** FR-044 — a reply quotes the
   sender's sanitised HTML rather than a rebuilt reduction — reverses the
   property ADR 0003 and ADR 0004 rest on. An ADR supersedes them or the change
   does not start.
2. **Three builds.** Markdown input, the editor's appearance, and the quote
   change itself. Each is independently shippable.
3. **A conformance pass.** Most requirements describe behaviour that already
   works and nothing asserts. Those become tests, not code — and the two worth
   having regardless are FR-021 (a Bcc recipient is never disclosed) and FR-032
   (a draft never carries two signatures), because both fail silently and both
   are visible to a recipient.

The technical approach follows one existing precedent: `postio-ui::reader::document`
already owns the reader's document assembly — stylesheet, ground colour, CSP,
wrapping — as toolkit-free code both frontends inherit. The editor has no
equivalent, which is why it renders in WebKit's defaults. Giving it one is the
spine of this plan, and it is where the quote construction belongs too.

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml` (1.98.0)

**Primary Dependencies**: gtk4 / libadwaita / webkit6 (`postio-gtk`); `ammonia`,
`html5ever`, `cssparser` (`postio-body`); no new dependency is required by this
feature

**Storage**: SQLite via `postio-storage` (drafts, attachments as blobs); the
blob store holds attachment and inline-image bytes

**Testing**: `cargo test --lib` per crate for pure logic; `gtk_suite` and
`app_suite` under `cargo nextest` for anything needing a display; the `.eml`
corpus in `crates/postio-model/tests/corpus/` for HTML quoting

**Target Platform**: Linux, GTK4/libadwaita, Wayland first (Constitution,
Additional Constraints). `postio-ui` and `postio-body` changes are inherited by
the macOS frontend and must stay toolkit-free.

**Project Type**: Desktop application, Rust workspace of ~20 crates

**Performance Goals**: interaction < 16 ms (typing must not be perceptibly
late); transitions ≤ 100 ms or absent; draft autosave must not block a keystroke

**Constraints**: the UI never awaits the network; the editor's WebView runs
under a hardened profile with a restrictive CSP; nothing leaves the machine that
the user did not ask for

**Scale/Scope**: one draft in the reading pane plus N detached windows
(FR-010/FR-011); a quote may carry an arbitrarily large sender document, bounded
by the same size total as attachments (FR-055)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| **I. Local-first** | Sending writes, enqueues, emits, repaints; the composer never awaits the network (FR-065, FR-059) | **PASS** — already true; the conformance pass asserts it |
| **II. Keyboard is a system** | Every command in `postio-core::registry` with binding, palette entry and accessible control; `docs/keybindings.md` regenerated | **PASS with work** — markdown input adds no command; the editor's appearance adds none; any new verb must enter the registry or it does not exist |
| **III. Search / one query language** | — | **N/A** — this feature adds no query surface |
| **IV. Test-first (NON-NEGOTIABLE)** | Failing test observed before the code that satisfies it; assertions on what a person would see | **PASS with work** — the conformance pass is *entirely* this, and the three builds each start red |
| **V. Performance is functional** | Budgets; counts not timings where a read path changes | **PASS with care** — see Complexity Tracking: the composer is not a SQLite read path, so `counting` applies only to draft save/load |
| **VI. Privacy is a feature** | Nothing leaves the machine unasked; no remote content while editing | **PASS with a new risk** — FR-042 makes a reply re-emit sender markup. FR-047 is the containment and must be tested as the security property it is |
| **VII. Boundaries enforced** | `postio-body`/`postio-ui` stay toolkit-free; `postio-gtk` holds no protocol or SQL | **PASS with work** — editor document assembly belongs in `postio-ui`, not `postio-gtk`, matching the reader |

**One governance gate, and it blocks Phase 2:**

FR-044 supersedes the reply-construction property of ADR 0003 and ADR 0004. The
constitution's "One fact, one home" puts decisions in ADRs, and its compliance
review requires complexity a principle does not obviously permit to be justified
in the ADR that introduces it. **The ADR is task one.** A spec records a
decision; it cannot amend an accepted one.

## How This Lands

**One feature branch, no issues.** Decided by the maintainer 2026-09-10 and
recorded in the constitution (1.1.0, Development Workflow) rather than only
here, because CLAUDE.md may not contradict it and the previous rule was "claim
an issue, work it, land it, take the next one".

```bash
git worktree add ~/src/postio-worktrees/compose-editor \
    -b feature/compose-editor origin/main
cd ~/src/postio-worktrees/compose-editor
# work tasks.md top to bottom, one commit per task
scripts/issue-land.sh --detach       # one PR, reviewed against spec.md
```

- `tasks.md` is the queue and `spec.md` is the acceptance. Both are in the
  repository and reviewable, which is what an issue was providing.
- Commits end `Refs: specs/002-compose-editor` and the task id, not
  `Refs: #<issue>`.
- `issue-land.sh` accepts `feature/<slug>` as an issueless branch as of this
  change; its self-test covers both directions.

**What does not relax.** Test-first is unchanged — every task's test is
observed red before the code that satisfies it, and the gates are the same
gates. Work *discovered* that is not in the spec is still filed through
`scripts/issue-file.sh`: the exemption covers the planned work, not everything
the branch touches.

**The one exception to "no issues" is the ADR.** The governance gate above is
not a task on this branch — ADR 0033 supersedes part of two accepted ADRs, and
that is a decision the repository records separately from the feature that
needed it.

## Project Structure

### Documentation (this feature)

```text
specs/002-compose-editor/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── commands.md
│   ├── editor-document.md
│   └── quote-construction.md
└── tasks.md             # Phase 2 (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
crates/
├── postio-model/
│   ├── src/draft.rs            # Draft: to/cc/bcc, in_reply_to, identity, signature
│   ├── src/reply.rs            # recipients, subject prefix, in_reply_to
│   └── src/outgoing.rs         # In-Reply-To / References; undisclosed-recipients (FR-022)
├── postio-body/                # toolkit-free; owns the document and the sanitiser
│   ├── src/document.rs         # the typed Document (authoring form)
│   ├── src/sanitize.rs         # what may be rendered — and now, re-emitted
│   ├── src/styles.rs           # sender CSS scoping, already built for ADR 0032
│   ├── src/replying.rs         # quoted_reply: the file FR-042 changes
│   └── src/outgoing.rs         # multipart assembly, cid: parts
├── postio-ui/                  # toolkit-free; shared by both frontends
│   ├── src/reader/document.rs  # EXISTING precedent: sheet_for, reader_ground, wrap_document
│   └── src/editor/document.rs  # NEW: the same treatment for the editing surface
├── postio-gtk/
│   ├── src/editor.rs           # seed(): the bare contenteditable shell to be replaced
│   ├── src/composer.rs         # fields, attachments, identity picker
│   ├── data/editor.js          # where markdown input lives
│   └── tests/gtk_suite/        # display tests
└── postio-app/
    ├── src/reading.rs          # reader factory / warmer wiring
    └── src/compose.rs          # draft resume, queued-send cancellation
```

**Structure Decision**: no new crate. The feature is placed by Principle VII
rather than by convenience — document assembly and quoting are toolkit-free and
go in `postio-ui` and `postio-body`, so the macOS frontend inherits both and
they are testable with no display; `postio-gtk` keeps only the WebKit glue it
already has. The one new module is `postio-ui/src/editor/document.rs`, mirroring
`reader/document.rs` deliberately so the two cannot drift.

## Constitution Re-check (post-design)

Re-run after Phase 1. Nothing in the design moved a gate, and two got firmer:

- **VII. Boundaries** — strengthened rather than merely satisfied. Putting the
  editor's document in `postio-ui` and the quote in `postio-body` means the
  macOS frontend inherits both and neither needs a display to test. The design
  removes an existing boundary problem (document assembly living in
  `postio-gtk`) rather than adding one.
- **VI. Privacy** — the risk is now named and contained in a contract rather
  than a paragraph: [quote-construction.md](./contracts/quote-construction.md)
  makes FR-047 a corpus-wide security assertion with a number to hit, not a
  rendering nicety.
- **II. Keyboard** — confirmed by design: markdown input and the editor's
  appearance add no command, so the registry does not grow and
  `docs/keybindings.md` does not move. Stated in
  [commands.md](./contracts/commands.md) so a task cannot quietly add a
  mouse-only button.
- **IV. Test-first** — every artefact names how it is verified, and
  [quickstart.md](./quickstart.md) records the one thing tests cannot see: all
  WebKit tests here run on the software path, so the appearance work needs an
  eye on the accelerated one (#1307).

No new violations. The Complexity Tracking entries below are unchanged.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| A reply re-emits sender markup (FR-042) | The maintainer's decision of 2026-09-10: fidelity of the quote beats the closed-type guarantee | Rebuilding from the typed `Document` is simpler and safer, and is what exists — rejected by the maintainer because the quote does not then look like the message being answered. The risk is re-narrowed by FR-045 to "only what the reader would render", which is the same sanitiser, not a new one |
| Performance gated by tests, not counts (SC-003) | Principle V gates budgets as counts off SQLite's trace hook; typing latency in a WebView has no such counter | Counting statements would measure nothing here — the composer is not a read path. Draft save/load *is*, and does carry `counting` assertions. Wall-clock typing latency stays a bench that reports and does not gate, per Principle V's own rule |
