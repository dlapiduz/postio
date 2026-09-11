# Implementation Plan: The Conversation Reading Pane

**Branch**: `001-conversation-reading-pane` | **Date**: 2026-09-08 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-conversation-reading-pane/spec.md`

## Summary

Replace the single-message reading pane with a conversation pane: a pinned
two-row header, every message's body visible in one stack, per-message actions,
sender-intended layout, and a rail that tracks where you are reading. The
approach is **one document per conversation in one reusable rendering surface**
(ADR 0032), with sender CSS admitted and contained at sanitize time rather than
deleted, and with the cost of moving between messages asserted as counts.

Research is in [research.md](./research.md). Its one open decision — whether
the reader may run Postio's own script so the rail can follow the scroll — was
taken on 2026-09-08 and is recorded in **R3**; sender script stays refused.
Every phase is now sequenceable.

## Technical Context

**Language/Version**: Rust, pinned by `rust-toolchain.toml`

**Primary Dependencies**: GTK4 4.22, libadwaita 1.9, WebKitGTK 2.52.5
(`webkit6` 0.6, features `v2_50`), `ammonia`/`html5ever` in `postio-body`,
`rusqlite` in `postio-storage`

**Storage**: SQLite (SQLCipher) plus the content-addressed blob directory.
This feature adds one stored per-message value (message length) and no tables.

**Testing**: `cargo test --lib` for the pure crates, `cargo nextest run` for
integration suites, `crates/postio-app/tests/app_suite/` for wiring, headless
by default under the cargo runner

**Target Platform**: Linux, Wayland first. The macOS frontend consumes the same
`postio-ui` rules (#1259, #1285) and is not built here.

**Project Type**: Desktop application, Rust workspace of 20 crates

**Performance Goals**: interaction < 16 ms, conversation open independent of
thread length, one render per gesture, zero additional rendering surfaces per
message

**Constraints**: the reading pane never awaits the network; no remote resource
without per-sender consent; message content cannot affect the application or
another message; resources bounded by what is displayed, not by what has been
visited

**Scale/Scope**: 6 user stories, 71 functional requirements, 16 success
criteria. Touches `postio-body`, `postio-ui`, `postio-gtk`, `postio-core`
(registry), `postio-index` (message length), `postio-app` (wiring).

## Constitution Check

*GATE: must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| I. Local-first, UI never awaits the network | **Pass** | FR-029/FR-030/FR-031. The pane draws header, actions and banners from local state before any body arrives. |
| II. The keyboard is a system | **Pass, with work** | FR-054 requires three new registry commands (R7). Keys come from the registry; `docs/keybindings.md` regenerates. The design brief's table is not adopted. |
| III. Search is navigation | **N/A** | This feature does not touch the query language. |
| IV. Test-First | **Pass** | Every phase below is sequenced red-first, and the pure-crate layers (`postio-body`, `postio-ui`) are where the rules are proven in milliseconds. |
| V. Performance as counts | **Pass, with work** | FR-065 needs a rendering counter that does not exist yet (R4). Without it the cost-of-moving requirements are unenforceable prose. |
| VI. Privacy is a feature | **Passes, with an amendment owed** | R3 decided 2026-09-08. See below. |
| VII. Boundaries are enforced | **Pass** | Containment and scoping land in `postio-body` (a pure leaf); presentation rules in `postio-ui`; only drawing in `postio-gtk`. No SQL or protocol crosses into the frontend. |

### The one principle that needed a decision

**JavaScript is enabled in the reader for Postio's own injected observer**, and
script arriving in a message stays refused by `enable_javascript_markup(false)`.
Decided by the maintainer, 2026-09-08. It is the only mechanism found that
satisfies FR-034 and FR-035 once the conversation is one document.

Principle VI is satisfied because the guarantee it protects is untouched: a
message cannot run code, cannot reach the network, and cannot observe the
reader. The spec already stated the invariant in the form that survives —
FR-024 forbids executing *"any script contained in a message"*, which remains
literally true.

**Three amendments are owed, in the phase that lands the change**: ADR 0003,
`PRODUCT.md` §21, and `CLAUDE.md` each say the reader's view has JavaScript off
without qualification. Amending ADR 0003 is a deliverable of Phase 5, not a
footnote — the reasoning has to outlive this session. R3 carries the table.

**The decision is gated on a spike, not reopened by it.** Phase 5 begins by
proving two things: that `enable_javascript_markup(false)` genuinely refuses
inline `<script>`, event-handler attributes and `javascript:` URLs with
JavaScript enabled; and that an injected observer runs in an isolated world
exempt from the document's own `script-src 'none'`. If either fails, the
fallback is marking by navigation with FR-034/FR-035 amended — never a weaker
sanitizer.

### Where the plan stands, 2026-09-09

The one-document pane is **merged into this feature branch** and lives behind
`POSTIO_ONE_DOCUMENT`, off by default. R1's mechanism question is therefore
settled in practice as well as on paper, and ADR 0032's remaining open question
— whether one large document beats N processes — was measured (#1348,
`docs/notes/2026-09-08-what-a-thread-costs-in-two-panes.md`):

| | 2 messages | 10 | 50 |
|---|---|---|---|
| stacked, web Pss | 146 MiB | 378 MiB | 1559 MiB |
| one document, web Pss | 101 MiB | 101 MiB | 104 MiB |
| stacked, handover | 83 ms | 298 ms | 1.34 s |
| one document, handover | 59 ms | 47 ms | 102 ms |

One document is flat; the stacked pane grows about 31 MiB of Pss per message.
The rebuttal ADR 0032 could not answer loses.

**What that changed about the plan.** Phase 2 is no longer this initiative's
work to do — it arrived. What replaced it is *finishing* it, because the
experiment deliberately left per-message chrome behind when it moved that
chrome into the document:

| Gap | Spec | State |
|---|---|---|
| No action bar at all | FR-006 | Fixed (#1349), then moved to the header (#1351) and given icons (#1356) |
| Allowed senders' images still blocked | Principle VI | Fixed (#1353) |
| Blocked-images notice cannot be acted on | FR-005 | **Open** — needs a verb through `decide_policy` |
| No recipients line | FR-001 | **Conflicts with screen 28**; put to the maintainer on #1349 |

**And a testing constraint the plan did not anticipate.** The suite's display
produces no layout: every `getBoundingClientRect` is zero, so an assertion
about rendered geometry is either vacuous or measures the machine. #1307 records
it from the other end. Four attempts at one width assertion went that way before
it was rewritten as a DOM fact. Anything in Phase 5's rail that wants to assert
on layout will meet the same wall, and should plan to assert on the *rule* in
`postio-ui` instead — which is where FR-035's "greatest visible area" already
lives.

### Post-design re-check (after Phase 1)

Re-evaluated against `data-model.md`, `contracts/` and `quickstart.md`:

- **Principle V moved from "with work" to satisfiable.** `contracts/render-counting.md`
  names the five assertions and puts the counter on the `postio-ui` seam, so
  both frontends are held to the same numbers and the assertions run without a
  display.
- **Principle VII strengthened by the design.** Every rule this feature adds —
  containment and the refused-property set, the current-message rule, the
  header — is provable in a pure crate. `quickstart.md` sequences each phase at
  that layer, with the integration suites confirming rather than deriving.
- **Principle IV holds**, because the design put the rules where they can be
  seen red in milliseconds. The one exception is the surface-count assertion,
  which needs `gtk_reader`.
- **Principle VI is settled** by the R3 decision of 2026-09-08, with the ADR
  and two document amendments owed in Phase 5, and the spike as a proof
  obligation at the start of it.

No new violations. The Complexity Tracking table below is unchanged.

## Project Structure

### Documentation (this feature)

```text
specs/001-conversation-reading-pane/
├── plan.md              # This file
├── research.md          # Phase 0 output — R1..R8
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── conversation-document.md
│   ├── registry-commands.md
│   └── render-counting.md
├── checklists/
│   └── requirements.md
└── tasks.md             # /speckit-tasks output — not created here
```

### Source code

```text
crates/postio-body/src/
├── sanitize.rs          # admit sender CSS; scope it; refuse the escape set
└── reader_view.rs       # stops dropping style/bgcolor/width/class

crates/postio-ui/src/
├── conversation.rs      # participants (exists); collapsed_runs() DELETED (R6)
├── reader/
│   ├── document.rs      # one-document assembly; contain_body/Sheet exist
│   ├── header.rs        # the shared header rules (#1285 lands first)
│   ├── rail.rs          # NEW — rail model, current-message rule
│   └── parts.rs
└── test_support/        # NEW — render counting (R4)

crates/postio-gtk/src/reader/
├── view.rs              # one surface per pane, reused; settings (R3)
├── message_header.rs    # delegates to postio_ui::reader::header (#1285)
├── actions.rs           # conversation bar + per-message actions
└── rail.rs              # NEW — draws the rail, degrades by width

crates/postio-core/src/registry  # three new commands (R7)
crates/postio-index/             # message length at index time (FR-039)
crates/postio-app/tests/app_suite/  # wiring, keystroke, click_preview cases
```

**Structure Decision**: no new crate. The work follows the existing boundary —
rules that can be proven without a display go in `postio-body` and `postio-ui`,
drawing stays in `postio-gtk` — which is what lets the containment rules (R2)
and the current-message rule (FR-035) be tested in milliseconds rather than
through a compositor.

## Phasing

Ordered so that each phase is landable and green on its own, and so that the
riskiest work — the rail, which opens with the R3 spike — comes after the
document shape it depends on is proven.

| # | Phase | Delivers | Depends on |
|---|---|---|---|
| 0 | **#1285: share the header** | One definition of the header rules; #1285's red test green | — |
| 1 | **Sender CSS, contained** | FR-019, FR-019a, FR-019b, FR-020..FR-023; the escape-set tests | R2 spike (`@scope`) |
| 2 | **One document per conversation** | FR-013, FR-052, FR-057, FR-063; per-message `cid` tokens; chrome in HTML | R1, #1316's numbers |
| 3 | **Header and actions** | FR-001..FR-011, FR-002a, FR-008a, FR-009a | Phase 0 |
| 4 | **The cost of moving** | FR-024, FR-058..FR-065; the render counter | R4, Phase 2 |
| 5 | **The rail** | FR-033..FR-047; ADR 0003 amended; `PRODUCT.md` §21 and `CLAUDE.md` corrected | R3 spike, Phase 2 |
| 6 | **Retire the collapsed conversation** | FR-014, FR-015, FR-018; ADR 0015 amended; `collapsed_runs()` deleted | Phase 2 |

Phase 6 is last deliberately: deleting the collapse machinery while the
one-document stack is unproven would leave no working pane in between.

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| Enabling JavaScript in the reader for Postio's own script (R3, **approved 2026-09-08**) | FR-034/FR-035 require the marked message to follow the scroll, and one document (R1) puts message positions where only the engine can see them | Marking by navigation fails the requirement the spec wrote specifically to forbid it; a toolkit-owned stack fails FR-057 and reinstates ADR 0032's process-per-message cost. Sender script stays refused, so the guarantee Principle VI protects is unchanged |
| Sender CSS scoped at sanitize time rather than deleted (R2) | FR-019 admits it; ADR 0032's containment-for-free premise dies with it | Trusting CSS `contain` is a hint, not a boundary; per-message iframes cannot size to content without script |
| A second counting facility, for rendering (R4) | Principle V gates on counts, and the storage counter cannot see a duplicate document load — the exact defect #749 found | Timing assertions cannot be defended on a shared runner |
