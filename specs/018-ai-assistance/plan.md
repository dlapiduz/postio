# Implementation Plan: AI assistance

**Branch**: `feature/ai-assistance` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: `specs/018-ai-assistance/spec.md`; issue #7; ADR 0009; spec 007
FR-165 to FR-175 and R16; `origin/main` at `17fff8da`.

This is a **roadmap plan**. It records the research, the slices and the crate
placement. The Phase 1 artifacts and `tasks.md` are written when the
maintainer schedules it. **It cannot be scheduled until Q1's constitution
amendment lands.**

## Summary

Five features on the model the person already runs, each extending what
`postio-ai` does today:

1. **thread summary** on `S`, which is the digest summariser applied to one
   thread;
2. **reply suggestion** on `mod+j`, a proposal beside the draft that becomes
   text only on `Tab`;
3. **semantic search**, which re-ranks full-text results with embeddings the
   person's runtime computes. It is a background feature, so it is off until
   switched on;
4. **query suggestion**, the model as a better lowerer into the one query
   language;
5. **ask your mail**, referenced answers built on 1 and 3.

Each feature has its own switch per account and a non-AI path, and none
needs more than a local model. The guarantees are the ones already built:

- no tools;
- fenced input;
- text-only output;
- statements kept only if their quotes are in the mail;
- no send path in the crate's graph;
- egress rows with no content.

**The first slice (A) is the thread summary.** It reuses the digest
summariser's verified-reference machinery, runs only on demand, and needs no
composer work, no new storage beyond a cache, and no background job.

## Technical Context

**Language/Version**: Rust (1.100, edition 2024); Swift for the Mac.

**Primary Dependencies**:

- `postio-ai` as built: `Client`, `Schema`, `Summariser`, `resolve`, `fake`,
  `LocalTransport` over `io-http`;
- no new crate for the model;
- the OpenAI-compatible `/v1/chat/completions` with `response_format:
  json_schema`, and `/v1/embeddings` for slice D. Ollama and llama.cpp's
  server (`--embeddings`) serve both. No runtime is named in code.

**Storage**:

- `thread_summaries`, a cache keyed by thread, newest message and model;
- `message_embeddings` (ADR 0009 Q3): message id, model id, and a
  `vector8` column;
- `egress_log` rows, with `subsystem = 'model'` and the feature named.

**Testing**:

- the `fake` model for every case;
- the corpus's `prompt-injection` category (`untrusted-instructions`);
- request-shape tests for D6;
- counting assertions on hybrid search;
- `focus_suite` and the composer suites for frontends;
- storyboards.

**Performance Goals**:

- the UI budget is unaffected: every model call is off the UI path, and
  cancellable;
- local search stays under 100 ms with the semantic merge, gated as counts.

**Constraints**: the constitution's Scope (local only, optional, never
required, no engine), FR-165 to FR-169, ADR 0009 Q1 and Q4, and logs with no
content.

## Constitution Check

| Principle | Status |
|---|---|
| **Scope: "Other AI features stay deferred to epic E12"** | **Blocked.** Q1 must amend this line (A or B) before any slice is scheduled. This plan is written so the amendment can cite it |
| Scope: no embedded engine, local only, optional | Met as designed. Q2 and Q3 ask about going further, and the plan assumes their A answers |
| I. Local-first | No mutation comes from the model. Accepting a suggestion is a composer edit, which is local. Embedding work is background and throttled |
| II. Keyboard is a system | `summarise_thread`, `suggest_reply`, `use_suggestion`, `dismiss_suggestion`, `show_model_input` and `ask_mail` are registry commands, each `requires` its switch, so each is absent when it cannot run |
| III. One query language | Semantic search is a retrieval strategy, not an operator. A query suggestion is parsed by the one parser and shown as chips |
| IV. Test-first | Every slice starts from red cases on the fake model and the injection fixture |
| V. Performance | Model calls are off the UI path. Semantic search is gated by counts. The embedding scan runs in the engine (R3), not in Postio's memory |
| VI. Privacy | D6's input table, D7's logging, "Show what was sent", per-account switches, and deletion with the switch |
| VII. Boundaries | `postio-ai`'s boundary entry is unchanged: no send path, store, toolkit or engine, and no HTTP client but `io-http` |

**Gate result**: passes on every principle, and **blocked on the Scope line**
until Q1 is answered.

## Phase 0: Research

### R1. What is built, and what each slice reuses

| Built | Reused by |
|---|---|
| `Client` and `Question`; the 30 s `TIMEOUT`; the 60 s `RETRY_AFTER` back-off when the runtime fails | every slice |
| `Schema`: flat answers checked on the client | every slice |
| `Summariser` and `resolve`: statements with references, kept only if the passage is found byte for byte (`TextIndex::find`) | A (thread summary), E (ask your mail) |
| `NeedsActionModel`: byte-exact spans | A ("Asked of you") |
| `fake`: a scripted model for tests | every slice |
| `[focus.model]` with per-feature switches | every slice gains a switch; FR-005 adds per-account overrides |
| `postio_body`'s quote folding and signature detection | the "own text" D6 sends |
| `postio_search::natural` | C falls back to it |

### R2. Thread summary (slice A)

- **Input** (D6): for each message in the thread, oldest first, its own text
  with quotes and signature removed, its sender's display name and its date.
  The budget is `SOURCE_CHARS` per message, newest messages first, until the
  context budget is reached. The footer says how many were used.
- **Schema**: rows of `{kind: decided|open|asked_of_you|context, statement,
  message, passage}`. `asked_of_you` must also pass the needs-action rule:
  addressed to the person, and a byte-exact span of the newest message's own
  text.
- **Where it shows**: at the top of the open message's column, as the digest
  summary is drawn (screen 22's language). References open the passage
  (FR-173's behaviour), and `Esc` returns to the summary.
- **Cache**: `thread_summaries` (thread id, newest message id, model id,
  statements). A new message makes it stale, and `S` writes a new one. It is
  deleted with the switch.

### R3. Semantic search (slice D)

- **Embeddings** come from the person's runtime (`/v1/embeddings`), from the
  subject plus own text, truncated to the model's input size. The vector's
  dimension is whatever the model returns. It is stored as Turso's `vector8`
  (an int8 vector type, present in the engine's `core/vector`), with the
  model id.
- **The scan** is brute force first (ADR 0009 Q3). `ORDER BY
  vector_distance_cos(embedding, ?) LIMIT k` runs inside the engine, which
  streams pages, so Postio never holds a mailbox's vectors (constitution V).
  The engine's only vector index method is a toy sparse IVF, so an ANN index
  is not assumed. If the nightly bench misses budget at 100,000 messages, an
  index becomes its own issue with the number attached.
- **The merge** happens in `postio-index`: full-text candidates ∪ vector
  top-K, re-ranked by a fixed rule (reciprocal rank fusion). Every full-text
  hit is kept (FR-022). The query's predicates apply to both sides, so
  `from:ada lease` stays a `from:ada` search.
- **Backfill** runs as a host job behind body backfill, throttled like it
  (paused on metered and active use). It runs only for accounts whose switch
  is on, and shows progress in Settings.
- **Spike S1**: the scan's statements and wall time at 100,000 and 250,000
  messages with 768-dimension `vector8`. It also checks whether
  `vector_distance_cos` over `vector8` is exact enough for ranking.

### R4. Reply suggestion (slice B)

- **Input** (D6): the thread's own text, the draft so far, and the
  identity's display name. Nothing else.
- **Output**: paragraphs of plain text. They become `Block::Paragraph` of
  `Inline::Text` on `Tab` (ADR 0004). No link, image, quote or recipient can
  be expressed.
- **Surface**: a suggestion block in the composer, outside the document,
  which the composers draw and do not save. That depends on the composer
  surface. `specs/015-native-composer` (ADR 0039) is the planned home, and
  this slice is placed after it. If 015 is late, the WebKit composer shows
  the block as a separate widget above the editor, and the block is still
  never in the document.
- **Undo**: `Tab` inserts in one undo step.

### R5. Query suggestion (slice C)

- **Input**: the person's words; the operator list with one example each;
  the folder, label and saved-search names. No message content (D6).
- **Output**: `{query: string}`. It is parsed by `postio_search::parse`, and
  must round-trip with no free-text-only fallback for operators it names.
  Then it is shown as chips labelled "Suggested".
- **When**: only after the rule-based lowering leaves free words it could not
  place, debounced. It never blocks the bar, and results from the rule-based
  chips show at once.

### R6. Ask your mail (slice E)

- **Retrieval**: the hybrid search of R3 for the question, top-K threads.
  Without embeddings it uses full-text only, and says so in the footer.
- **Answer**: the R2 schema without `asked_of_you`, verified per statement.
  With nothing verified, the search results stand (FR-025).
- **Injection**: a retrieved message is third-party text, as in every
  feature. D4 means an answer can only repeat what a message says, attributed
  to that message. "Click here to verify" becomes a statement *quoting a
  sender*, not an instruction from Postio.

### R7. Pimalaya first

The model interface is OpenAI-compatible HTTP, already spoken through
Pimalaya's `io-http`. Pimalaya has no inference or embedding crates. Its
`assistant` repository is a development runner, and not a library for this.
Nothing else to survey.

### R8. The amendment text (for Q1)

Q1 A would replace the Scope's last AI line with: "Focus may also use that
model to summarise a thread, suggest a reply, rank search results by
meaning, suggest a query, and answer a question with references to the mail
(specs/018-ai-assistance), each with its own switch and a path that works
without it." That is a MINOR bump: it expands a section and narrows nothing.
Q1 B would state the rule instead: "any feature that uses that model has its
own switch, a path that works without it, no send path, and shows nothing
about mail it cannot point to in the mail."

## Phasing

| Slice | What | Stories, FRs | Depends on |
|---|---|---|---|
| **0. Amendment** | The constitution PR for Q1's answer, with its Sync Impact Report; ADR 0009 rewritten for Q2's answer | — | the maintainer |
| **A. Thread summary** (first slice) | `summarise_thread`; the R2 schema; the cache; the surface on GTK, terminal and Mac; "Show what was sent"; the per-account switch (FR-005); egress rows naming the feature | US1; FR-001 to FR-010, FR-020, FR-030, FR-040 to FR-043 | 0 |
| **B. Reply suggestion** | `suggest_reply`, `use_suggestion`, `dismiss_suggestion`; the block | US2; FR-021 | A, the composer surface (spec 015) |
| **C. Query suggestion** | R5 in the command bar | US4; FR-024 | A |
| **S1 (spike)** | Embedding scan budget (R3) | — | — |
| **D. Semantic search** | `message_embeddings`; backfill job; hybrid merge; Settings progress; deletion with the switch | US3; FR-022, FR-023 | A, S1 |
| **E. Ask your mail** | `ask_mail`, retrieval plus referenced answer | US5; FR-025 | A, D |
| *If Q2 or Q3 is not A* | A remote or LAN provider behind `Locality`, or an embedded embedder | — | the amendment it needs |

## Crate placement

| Crate | What it gains | Boundary it keeps |
|---|---|---|
| `postio-ai` | `ThreadSummariser`, `ReplySuggester`, `QuerySuggester`, `Answerer`, `Embedder`; their schemas; request builders that take only D6's inputs | no send path, store, toolkit or inference engine, and `io-http` only (unchanged) |
| `postio-config` | Switches `thread_summary`, `reply_suggestion` and `query_suggestion` (default on), and `semantic_search` (default off); per-account overrides | no rusqlite, gtk4 |
| `postio-storage` | `thread_summaries`, `message_embeddings` (`vector8`) | — |
| `postio-index` | The hybrid merge (R3), behind the existing executor | — |
| `postio-search` | No change; that is the point (ADR 0009) | pure leaf |
| `postio-host` / `postio-session` | Jobs (summary on request, embedding backfill, ask). A proposal becomes a command only on the person's gesture. "Show what was sent" held in memory | — |
| `postio-core` | The commands, each with `requires` set to its switch | no GTK |
| `postio-ui` | View models and strings: summary rows, the suggestion block, chips labelled "Suggested", the footers | no GTK, no SQL |
| `postio-widgets`, `postio-gtk`, `postio-tui`, `postio-ffi`, `macos/` | Drawing | as today |
| `scripts/checks/check-no-silent-tracking.py` | No change: the model endpoint already carries its `POSTIO-CONSENT:` line | — |

## Testing (ADR 0049)

- **`postio-ai`**: every schema's acceptance and rejection; reference
  verification; D6's request shape per feature (the request body asserted
  field by field); markup in an answer stays characters.
- **Injection**: `untrusted-instructions` through every feature. Assert zero
  commands, zero extra connections, zero live links, and an unchanged draft
  and recipients.
- **Counting**: the hybrid merge's statements; the summary read; the
  embedding job's statements per message.
- **Frontends**: one case per path: `S` and a reference and `Esc`; `mod+j`
  then `Tab` then `mod+z`; `mod+j` then `Esc`; a suggested chip; "Ask your
  mail". Plus the every-command sweep with no model (SC-001). Storyboards for
  `S` and `mod+j`.
- **No real runtime** in the default suite. A live-runtime smoke test is
  `#[ignore]`.

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| The amendment is not made | Nothing can be built | Slice 0 is first, and R8 drafts the text |
| A local model is slow or weak | Features feel bad, and people blame Postio | On demand, cancellable, and never blocking; the footer names the model; D4's verification drops what is wrong rather than showing it |
| Social engineering through a summary | The model repeats a phishing line | D4: statements quote and attribute the mail; no links; spec 016's warning line still shows on the message |
| Embedding a whole mailbox is slow on a CPU | Semantic search arrives late | Off by default; progress shown; partial coverage still helps, and full text is always there |
| The vector scan misses budget | Search slows | S1 measures before slice D. ANN becomes its own issue, with a number |
| The suggestion block depends on the composer rewrite | Slice B waits | Its fallback is a separate widget above the editor, never inside the document |
| Constitution and ADR 0009 drift | Confusion about remote models | Slice 0 rewrites ADR 0009 to Q2's answer |

## Project Structure

### Documentation (this feature)

```text
specs/018-ai-assistance/
├── spec.md
├── plan.md
├── checklists/requirements.md
└── (when scheduled) data-model.md, contracts/, quickstart.md, tasks.md
```

### Source Code (repository root)

```text
crates/postio-ai/src/        # thread.rs, reply.rs, query.rs, answer.rs, embed.rs
crates/postio-config/src/model.rs
crates/postio-storage/src/schema.rs
crates/postio-index/src/     # hybrid merge
crates/postio-host/src/ai/
crates/postio-core/src/registry.rs
crates/postio-ui/src/ai/
crates/postio-widgets/src/, crates/postio-gtk/src/, crates/postio-tui/src/, crates/postio-ffi/src/
.specify/memory/constitution.md   # slice 0
docs/decisions/0009-ai-subsystem.md   # slice 0
storyboards/ai/
```

**Structure Decision**: no new crate. Every feature is a question in
`postio-ai`, a job in the host, and a view model in `postio-ui`.

## Complexity Tracking

No new crate, no new boundary and no new network path, so there is nothing
to justify. The one addition that needs justifying is the constitution
amendment, and Q1 states it.
