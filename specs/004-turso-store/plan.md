# Implementation Plan: The store, rebuilt on Turso

**Branch**: `feature/turso-store` | **Date**: 2026-09-12 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-turso-store/spec.md`

## Summary

Replace SQLCipher and rusqlite with Turso, natively: `postio-storage`
rewritten against an async engine rather than wrapped in a synchronous shim,
`postio-index` rewritten from FTS5 onto Turso's own full-text index, and the
counted-cost instrument replaced with something that engine can support.

The schema is not the work — a spike applied 96 of 102 head objects unchanged,
triggers included. The work is three things the engine does differently: it is
async where the storage layer is not, its full-text search indexes columns
rather than virtual tables, and it folds no diacritics.

## Technical Context

**Language/Version**: Rust 1.98.0, edition 2024 (pinned by `rust-toolchain.toml`)

**Primary Dependencies**: `turso 0.8.0-pre.11` — prerelease, pre-1.0, with
encryption at rest and full-text search both marked experimental and neither
third-party audited. Replaces `rusqlite` + `libsqlite3-sys` +
`openssl-src`/`openssl-sys`, which leave the graph entirely. `zstd` leaves the
body path (see Complexity Tracking).

**Storage**: One encrypted Turso database per installation, AES-256-GCM under a
256-bit key from the OS keyring. Attachments and raw messages stay in the
existing file-backed `BlobStore`, which is untouched by this work.

**Testing**: `cargo nextest` per crate; the GTK and app suites under the
headless compositor; no test reaches the network.

**Target Platform**: Linux, Wayland, GTK4/libadwaita.

**Project Type**: Desktop application, Cargo workspace of 21 crates.

**Performance Goals**: Unchanged and non-negotiable — 500 ms to a usable
window, 16 ms per interaction, 100 ms for local search (`PRODUCT.md` §18).

**Constraints**: Never load a whole mailbox into memory. The UI never awaits
the network. No message content in logs.

**Scale/Scope**: `postio-storage` 9,495 lines / 265 call sites; ~190 more
across `-runtime`, `-session`, `-sync`, `-index`, `-app`. Reference mailbox
~81,000 messages, 223 MB.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Local-first, UI never awaits network | Writes land locally before the network hears; no read blocks on a socket | **Pass** — unchanged by engine. Watch: making the storage layer async must not let a UI path `await` a store read on the main context. |
| II. The keyboard is a system | No surface changes | **Pass** — out of scope. |
| III. Search is navigation, one query language | Same string means the same thing everywhere; answers locally within budget | **At risk** — the query *language* is untouched (`postio-search` parses; only execution moves), but the engine folds no diacritics. FR-008 and SC-002 make equivalence with today the acceptance. See Phase 0. |
| IV. Test-first | Every task's test observed red first | **Pass** — the task list is written that way. |
| V. Performance is a functional requirement | Budgets defended by counted work, not wall-clock | **At risk** — the instrument reads SQLite's trace hook and has no counterpart. A replacement is a task, not an afterthought; without it the budgets are documentation again (#1434). |
| VI. Privacy is a feature | Encrypted at rest, no content in logs | **Pass**, with a caveat recorded: the encryption is experimental and unaudited upstream. The spec makes "no plaintext in the file" and "another key is refused" acceptance criteria rather than assumptions for exactly that reason. |
| VII. Boundaries are enforced | `postio-gtk` no SQL; `postio-model` no engine; `postio-config` no engine | **Action required** — `check-crate-boundaries.py` bans `rusqlite` by name. It must ban `turso` in the same places or the boundary silently stops being checked. |

**No unjustified violations.** The two "at risk" rows are the two hardest
tasks, and both have acceptance criteria in the spec rather than promises here.

## Project Structure

### Documentation (this feature)

```text
specs/004-turso-store/
├── spec.md              # Written
├── plan.md              # This file
├── research.md          # Phase 0
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/           # Phase 1
├── checklists/
│   └── requirements.md  # Written, passing
└── tasks.md             # /speckit-tasks
```

### Source Code

```text
crates/
├── postio-storage/          # rewritten: async repositories over turso
│   ├── src/store.rs         #   replaces db.rs — open, key, schema at head
│   ├── src/schema.rs        #   the head schema, one place, no migrations
│   ├── src/repository/      #   every repository, async
│   └── src/test_support/    #   counting replaced (see research)
├── postio-index/            # rewritten: fts index method, not fts5
│   ├── src/index.rs         #   CREATE INDEX ... USING fts, and the fold
│   └── src/executor.rs      #   fts_match/fts_score, ranking preserved
├── postio-search/           # untouched — the parser does not know the engine
├── postio-runtime/          # SqliteStore becomes a thin async adapter
├── postio-session/          # open_store, ensure_search_index
├── postio-sync/             # writes become awaits
├── postio-app/              # composition root follows
└── postio-model/            # untouched — takes no engine, and must not start
```

### What is deliberately not touched

`postio-search` (the query language), `postio-gtk` (no SQL by boundary),
`postio-model`, `postio-body`'s MIME work, and the `BlobStore`. If a change
reaches them, the design is wrong.

## Complexity Tracking

| Cost accepted | Why | What was rejected |
|---|---|---|
| A pre-1.0 dependency with two experimental features | The maintainer asked for a build to test. Acceptance criteria, not assumptions, cover the two features. | Waiting for 1.0 — which is a decision, not a plan. |
| Bodies stored as text, losing zstd (1.39x measured upper bound, 2.19x recorded on a real account) | A column-indexing full-text engine cannot tokenise compressed bytes. | Keeping compression and losing body search. |
| A second, folded copy of body text — **unless Phase 0 finds otherwise** | The engine folds no diacritics, and folding the stored copy would fold what is displayed. | Folding only the query, which cannot work; expanding a query to every accented variant, which is not tractable. |
| Every repository becomes async | The engine is async-native; a synchronous facade would carry SQLite's shape into a database that is not SQLite. It is also what the maintainer rejected. | The `postio-turso` shim, built and measured on `spike/turso-port`. |
| Losing the trace-hook instrument | It does not exist here. | Leaving the budgets undefended, which is the state #1434 was filed about. |

**The body-storage total is the number to watch.** Losing compression is
~2.19x; a folded second copy would be ~2x on top of that. Phase 0's first
question is whether the second can be avoided, because together they are the
difference between a 223 MB store and something near a gigabyte.
