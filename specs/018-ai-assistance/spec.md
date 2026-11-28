# Feature Specification: AI assistance with the model you run

**Feature Branch**: `feature/ai-assistance`

**Created**: 2026-11-22

**Status**: Roadmap draft. Not scheduled. Building it needs the constitution
amendment in Q1. No `tasks.md` until the maintainer schedules it and answers
the questions under "Clarifications"

**Input**: The maintainer, 2026-11-22: "I want to create roadmap specs/docs
for: … Continue the AI integration". This spec also takes in issue #7, "AI
subsystem: summarize, draft reply, semantic search".

## Context

"Continue" starts from what is built. Postio has one AI client, and it is
narrow on purpose:

| What exists on `main` | Where |
|---|---|
| **`postio-ai`**, a client for a model the person runs on this computer. It speaks the OpenAI-compatible chat completions that Ollama and llama.cpp's server serve, over `io-http` with no TLS, to a `ModelEndpoint` that can only be built from a loopback address or a local socket | ADR 0009 "What is built"; spec 007 R16 |
| **`[focus.model]`** in `config.toml`: endpoint, model, and one switch per feature. With no section, no client exists and nothing connects | `postio-config/src/model.rs`; spec 007 FR-165 to FR-169 |
| **Three features**, each with a fixed, flat answer schema checked on the client. The digest summary keeps a statement only if its quoted passage is found byte for byte in the message | FR-170 to FR-175 |
| A **non-AI path for every feature**: the built-in needs-action detector, digests opening on their plain list, and "Digest mail like this" absent | FR-105, FR-167, FR-175 |
| **The egress log** records every model call (`subsystem = 'model'`): ids, counts and outcomes only | ADR 0009 Q6 |
| **Rule-based natural-language search**: `postio_search::natural` lowers "the invoice Ada sent last month" into the one query language, as chips | spec 010 FR-062, R5 |
| **`postio-mcp`**, for external agents: reads and drafts, never sends | ADR 0010 |

### The rules every feature here obeys

The constitution's Scope, as amended to 1.3.0, says:

- Postio **embeds, bundles, downloads and starts no model and no inference
  engine**;
- a model the person runs on this machine is **optional, off until named,
  reached only on this machine, and never required**;
- **other AI features stay deferred to epic E12**.

This spec is E12's work. Building any of it needs Q1's amendment to that last
line. Nothing here needs more than a local model the person runs. Where a
capability would need more (a remote model, an embedded engine, Postio
starting a runtime), the spec does not assume it: Q2 and Q3 put it to the
maintainer.

`PRODUCT.md` §12 and ADR 0009 add two more constraints:

- **AI never silently modifies or sends mail.** Every externally visible
  action needs the person's own gesture in Postio's UI;
- **mail is attacker-controlled text.** Prompt injection is the dominant
  design constraint, and is answered structurally:
  - no tools in any request;
  - message text fenced as data;
  - output shown as text, never markup;
  - a proposal shown before it is anything.

### What this spec supersedes

- **Issue #7** is folded in. Its three capabilities (summarise, draft reply,
  semantic search) are US1, US2 and US3. Its acceptance criteria map as
  follows:
  - "provider choice and data-sharing controls are explicit and enforced"
    is FR-002 to FR-005;
  - "every externally visible action requires confirmation" is FR-006;
  - "works with a local model" is the premise of the whole spec.

  Its "cloud and local" provider trait is Q2, not assumed. The issue stays
  open until this spec lands, and then closes.
- **ADR 0009**'s design for summarise, draft and semantic search is kept and
  cited. Its **remote provider with consent** contradicts the constitution's
  "reached only on this machine", and the constitution wins. ADR 0009 is
  rewritten in place to match Q2's answer, in the slice that settles it.

## Decisions this spec takes

These are design calls an agent can make. Each is reviewable.

**D1. One model section, one switch per feature, and background work is off
until switched on.** `[focus.model]` stays the one home (one fact, one home).
Each new feature gets its own switch:

- **On-demand features** run only when the person invokes them. They default
  to on once the section exists: summarise a thread, suggest a reply, ask
  for a query.
- **Background features** read mail the person did not point at. They
  default to **off** even when the section exists: computing embeddings for
  semantic search.

An upgrade never starts a background feature the person has not turned on.

**D2. Every feature degrades to a path that already exists, and its command
is absent when it cannot run.**

| Feature | With the model | Without it, or when it is not running |
|---|---|---|
| Summarise thread (US1) | A summary with references | The command is absent. The thread reads as today, with the built-in detector's markers |
| Suggest a reply (US2) | A suggestion in the composer | The command is absent. Reply opens empty, as today |
| Semantic search (US3) | Full-text results re-ranked with meaning | Full-text results only, the same query, the same chips |
| Ask for a query (US4) | The model proposes a query in the one language, shown as chips | `postio_search::natural`'s rules, as today |
| Ask your mail (US5) | An answer whose every statement cites a message | The command is absent. Search answers the question |

"Absent" means absent from every surface (ADR 0009 Q2; constitution II: a
command that cannot run is not offered). A model that is configured but not
running, or too slow, is treated the same, and nothing waits on it
(FR-167).

**D3. The model's text is never mail until the person makes it so.**

- A summary is shown, never stored in a message.
- A suggested reply is drawn **beside** the draft as a suggestion block:
  "Suggestion from <model> · `Tab` use · `Esc` dismiss". It becomes draft text
  only on `Tab`. Before that it is not autosaved, not synced to the server's
  Drafts, and not sent.
- After `Tab` the text is the person's draft, and Send (`mod+Return`) is the
  only way it leaves (ADR 0009 Q4).

**D4. Every statement the model makes about mail must point into the
mail.** The digest summariser's rule (FR-173) applies to every feature that
writes about mail:

- each statement carries a reference to a message and a passage;
- a statement whose passage is not found byte for byte in that message's own
  text is dropped;
- "Asked of you" items are byte-exact spans, as needs-action markers are
  (FR-104).

A summary can be incomplete. It cannot invent a quote.

**D5. A proposed query is shown, never run blind.** The model's answer to
"ask for a query" is a string that `postio-search` parses. It appears as
chips in the command bar, exactly as the rule-based lowering does, and runs
as the person sees it (constitution III). An answer that does not parse is
discarded, and the rule-based lowering stands.

**D6. What is sent to the model is the least the feature needs, and the
person can see it.**

| Feature | Sent to the local model | Never sent |
|---|---|---|
| Summarise thread | Each message's own text: the newest part, with quoted history and signatures removed, up to a budget. Sender display names. Dates | Addresses, headers, attachments, remote content, other threads |
| Suggest a reply | The thread's own text as above. The person's draft so far. Their signature's name | Other threads, contacts, addresses |
| Semantic search | Each message's subject and own text, to compute its embedding. The query text, to embed it | Attachments, headers, addresses |
| Ask for a query | The person's words. The names of folders, labels and saved searches | Any message content |
| Ask your mail | The question, and the own text of the top results (as US1) | Everything else |

"Show what was sent" opens, for the last request of a feature, the exact text
that went to the model. It reads from memory, and is never logged or stored.

**D7. Logs and the egress log carry no content.** `POSTIO_LOG` and the
`egress_log` carry the feature, message ids, counts, byte totals, duration
and outcome. They never carry prompts, answers, subjects, names or
addresses. Embeddings are stored in the encrypted store, and are deleted when
their switch is turned off.

**D8. Keys.**

| Command | Key | Where |
|---|---|---|
| `summarise_thread` | `S` | List, thread, reader |
| `suggest_reply` | `mod+j` | Composer |
| `use_suggestion` | `Tab`, while a suggestion is shown | Composer |
| `dismiss_suggestion` | `Esc`, while a suggestion is shown | Composer |
| `show_model_input` ("Show what was sent") | in the command bar only | wherever a model answer is shown |
| `ask_mail` | the command bar: a sentence ending in `?` offers "Ask your mail" as its last row | Command bar |

There are no `ai:` prefixes in ids. These are built-in commands, each with
`requires` set to its switch (ADR 0009 Q5's provenance is kept in the
command's title and its footer line "Written by <model> on this computer").

## Clarifications

### Open questions for the maintainer (constitution amendments)

- **Q1. Lift E12's deferral for these features?** The constitution says
  "Other AI features stay deferred to epic E12." Nothing here can be built
  until that line changes.
  - **A: a MINOR amendment naming these features** (thread summary, reply
    suggestion, semantic search, query suggestion, ask your mail) under the
    same three conditions: optional and off until named, only on this
    machine, never required. Each feature keeps its own switch and its
    non-AI path. This is the spec's recommendation.
  - **B: a MINOR amendment stating the rule, not the list.** Any feature that
    uses the person's local model is allowed if it has its own switch, a
    non-AI path and no send path. Later features then need no amendment.
  - **C: keep them deferred.** This spec stays on the roadmap.
- **Q2. May a model run anywhere but this machine?** ADR 0009 designed a
  remote provider with per-account, per-feature consent. The constitution
  since forbids it.
  - **A: never.** Only this machine. ADR 0009 is rewritten to drop the remote
    provider. This is the spec's default, and nothing in the spec needs more.
  - **B: a machine the person names on their own network** (a GPU box at
    home), over TLS with a pinned certificate. This needs an amendment of
    "reached only on this machine", and a TLS stack in `postio-ai`, which
    today links none.
  - **C: a cloud provider, per account and per feature, with consent**, as
    ADR 0009 drew it, and an egress log the person reads. This needs an
    amendment to Scope, and weighs hardest on constitution VI: message text
    would leave the machine to a third party.
- **Q3. May Postio carry or start an inference engine for semantic search?**
  Embeddings need a model. Without one the person runs, semantic search does
  not exist.
  - **A: no.** Semantic search exists only when the person's runtime serves
    embeddings (OpenAI-compatible `/v1/embeddings`, which Ollama and
    llama.cpp serve). This is the spec's default.
  - **B: an embedded, in-process embedding model** (a small sentence model
    in pure Rust). This needs the amendment of "embeds no model", the
    boundary check "no app links an inference engine" relaxed for one crate,
    and weights shipped or downloaded.
  - **C: Postio starts the person's installed runtime** when a feature needs
    it. This needs the amendment of "starts no model". Postio would own a
    process it did not install.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Summarise a long thread (Priority: P1)

A person opens a thread of 23 messages and presses `S`. In a few seconds a
summary appears at the top of the thread:

- what was decided;
- what is still open;
- what is asked of them.

Each statement ends in a numbered reference that opens the message with the
passage highlighted. The footer says "Written by <model> on this computer".

**Why this priority**: It is `PRODUCT.md` §25's workflow ("`Ctrl+K →
Summarise`"). It reuses the digest summariser's proven machinery (FR-172 to
FR-175) on one thread, and it runs only on demand.

**Independent Test**: With the fake model (`postio-ai`'s `fake`), summarise
corpus threads. Assert that every shown statement's passage is in its
message, that a fabricated passage is dropped, that a reference opens the
passage, and that output looking like markup is drawn as characters. With no
model, assert that `S` is absent from the key map and the command bar.

**Acceptance Scenarios**:

1. **Given** a model is configured and running, **When** the person presses
   `S` on a thread, **Then** a summary appears with numbered references, and
   the thread is readable and actionable while it is written.
2. **Given** the model returns a statement quoting text not in the thread,
   **When** the summary is shown, **Then** that statement is absent.
3. **Given** the thread contains the corpus's `untrusted-instructions`
   fixture, **When** it is summarised, **Then** no command runs, no
   connection other than the one completion is made, and no live link
   appears.
4. **Given** no model is configured, **When** the person opens the command
   bar on a thread, **Then** "Summarise thread" is not offered.
5. **Given** the model is configured but not running, **When** the person
   presses `S`, **Then** a notice says "Your model at 127.0.0.1:11434 isn't
   answering", and nothing waits.

---

### User Story 2 - Get a suggested reply, and decide (Priority: P1)

A person presses `r` and then `mod+j`. A suggested reply appears beside the
empty draft. They press `Tab` to use it, edit it, and send it with
`mod+Return`. Or they press `Esc`, and it is gone.

**Why this priority**: It is issue #7's "draft reply" and §25's second half.
It is also the feature closest to sending, so its safety must be built
first and right.

**Independent Test**: Assert that before `Tab` the draft document is
unchanged, that nothing is autosaved, synced or queued, and that the
suggestion is plain paragraphs. After `Tab`, the text is the draft. After
`Esc`, nothing remains. Over the injection fixture, the suggestion cannot add
a recipient, an attachment or a link.

**Acceptance Scenarios**:

1. **Given** a reply composer, **When** the person presses `mod+j`, **Then**
   a suggestion block appears, and the draft and its recipients are
   unchanged.
2. **Given** a suggestion, **When** the person presses `Tab`, **Then** its
   text becomes paragraphs of the draft, and `mod+z` takes it back out.
3. **Given** a suggestion, **When** the person closes the composer, **Then**
   the saved draft does not contain it.
4. **Given** a suggestion containing `<a href="https://phish.example/">`,
   **When** it is used, **Then** the draft holds those characters as text,
   not a link.
5. **Given** no model, **When** the person presses `mod+j`, **Then** nothing
   happens beyond the absent-command hint, and replying works as today.

---

### User Story 3 - Find mail by meaning, not only by words (Priority: P2)

A person searches `lease renewal`. Postio also finds the message that says
"the apartment contract is up in March", ranked among the literal matches.
The same query typed tomorrow means the same thing, with or without
embeddings.

**Why this priority**: It is issue #7's "semantic search". It is a
background feature that reads the whole mailbox through the model, so it is
off until switched on, and it comes after the on-demand features.

**Independent Test**: With fake embeddings over the corpus, assert that the
hybrid results include every full-text hit, that semantic-only hits are
merged under the 100 ms budget (counted), that turning the switch off
deletes the embeddings, and that a message without an embedding is still
found by its words.

**Acceptance Scenarios**:

1. **Given** semantic search on, and embeddings computed, **When** the
   person searches, **Then** every full-text hit is in the results, and
   semantic hits are merged and ranked.
2. **Given** the switch off, **When** the person searches, **Then** the
   results are full-text only, and the query and chips are identical.
3. **Given** the switch turned off, **When** that happens, **Then** every
   stored embedding is deleted.
4. **Given** embeddings still being computed, **When** the person searches,
   **Then** results come from what exists, and Settings shows progress ("12,400
   of 81,700 messages").

---

### User Story 4 - Say what you want, get the query (Priority: P2)

A person types "that thing Ada sent before her trip about the venue" in the
command bar. The rule-based lowering finds `from:ada` and stops. With the
model on, the bar offers `venue from:ada before:2026-10-03` as chips. The
person sees them, adjusts one, and runs it.

**Why this priority**: Constitution III already says natural language lowers
into the one language. The model is a better lowerer, never a second
language.

**Independent Test**: With the fake model, assert that its proposal is
parsed, shown as chips and run only as shown, that an unparseable proposal
falls back to the rules, and that no message content is in the request
(D6).

**Acceptance Scenarios**:

1. **Given** the model proposes a query, **When** it parses, **Then** it
   appears as chips labelled "Suggested", and Return runs exactly those
   chips.
2. **Given** the model proposes text that does not parse, **When** that
   happens, **Then** the rule-based chips stand, and nothing says the model
   failed.
3. **Given** the request sent, **When** the person runs "Show what was
   sent", **Then** it holds their words and folder names only.

---

### User Story 5 - Ask your mail a question (Priority: P3)

A person types "when is the dentist appointment?" in the command bar and
chooses "Ask your mail". The answer is "Thursday 3 Dec at 15:00 [1]", and
[1] opens the confirmation email with the passage highlighted.

**Why this priority**: It is the most useful and the most exposed feature:
it reads mail the person did not open. It builds on US1's references and
US3's retrieval, so it comes last.

**Independent Test**: With the fake model and fake embeddings, assert that
every statement in an answer is referenced and verified (D4), that an answer
with no verified statement shows "I couldn't find that in your mail" and the
search results, and that the injection fixture changes nothing.

**Acceptance Scenarios**:

1. **Given** a question whose answer is in one message, **When** the person
   asks, **Then** the answer cites that message, and the reference opens it.
2. **Given** the model answers without a valid reference, **When** that
   happens, **Then** no statement is shown, and the search results are.
3. **Given** no model, **When** the person ends a sentence with `?`, **Then**
   "Ask your mail" is not offered, and the sentence is searched as today.

---

### Edge Cases

- **The model is slow** (a 7B model on a laptop CPU). Every feature shows
  "Writing on <model>…" in place and stays cancellable with `Esc`. The
  interaction budget is the UI's, not the model's.
- **A very long thread**: the summary uses the newest messages up to the
  budget, and says "From the last 18 of 40 messages".
- **A thread in several languages**: the model answers in the language the
  person's UI uses. Quoted passages stay in their own language, byte-exact.
- **Encrypted mail** (spec 016): its text is not decrypted, so it is never
  sent to a model.
- **A message from an account whose switch is off** (per-account switches,
  FR-005): it is left out of summaries, embeddings and answers, and the
  summary footer says what was left out.
- **The model's context is too small for the request**: the request is cut
  to fit at message boundaries, never mid-passage, and the footer says so.
- **The person changes models**: embeddings from the old model are deleted
  and recomputed. Cached summaries say which model wrote them.
- **The fake model in tests**: every test drives the fake. No test touches a
  real runtime unless it is `#[ignore]`.

## Requirements *(mandatory)*

### Functional Requirements

**Configuration and consent**

- **FR-001**: Every feature MUST use only a model the person runs on this
  machine, named in `[focus.model]` (constitution Scope), unless Q2 is
  answered otherwise.
- **FR-002**: Each feature MUST have its own switch in `[focus.model]`.
  On-demand features MUST default on when the section exists. Background
  features MUST default off (D1).
- **FR-003**: Postio MUST NOT look for, start, download or embed a model or
  runtime (constitution Scope; spec 007 FR-165, FR-166).
- **FR-004**: Settings MUST show each feature's switch, the endpoint, and
  whether the model answers. It MUST link to the egress log's model rows.
- **FR-005**: Each feature MUST be switchable per account, so that one
  account's mail is never sent to the model while another's is.

**Safety**

- **FR-006**: Model output MUST NOT change, move, send or delete mail, or
  change a recipient, a label or a setting. Every externally visible effect
  MUST follow the person's own gesture: `Tab`, Return, or Send.
- **FR-007**: A request MUST NOT carry a tool definition. Message text MUST be
  fenced as third-party data (ADR 0009 Q4).
- **FR-008**: Model output MUST be shown as plain text. Markup, links and
  images in it MUST be drawn as characters.
- **FR-009**: Every statement about mail MUST carry a reference whose
  passage is found byte for byte in the referenced message's own text, or
  MUST NOT be shown (D4).
- **FR-010**: `postio-ai` MUST keep no send path, no store engine, no
  toolkit and no inference engine in its graph, and no HTTP client but
  `io-http` (the boundary check).

**Features**

- **FR-020**: `summarise_thread` MUST summarise the thread under the cursor,
  on demand, as statements with references, including "Asked of you" items
  as byte-exact spans. It MUST NOT block reading or acting on the thread.
- **FR-021**: `suggest_reply` MUST show a suggestion beside the draft. The
  suggestion MUST NOT enter the draft, autosave, sync or the outbox until
  `use_suggestion`, and `use_suggestion` MUST be one undo step.
- **FR-022**: Semantic search MUST re-rank full-text candidates with
  embedding candidates. It MUST NOT change the query language or the meaning
  of a saved search, and MUST keep every full-text hit (ADR 0009 Q3).
- **FR-023**: Embeddings MUST be computed in the background, throttled like
  backfill, only for accounts whose switch is on. They MUST be deleted when
  the switch is turned off or the model changes.
- **FR-024**: A suggested query MUST be parsed by the one parser and shown as
  chips before it runs. An unparseable one MUST be discarded silently.
- **FR-025**: `ask_mail` MUST answer only with referenced, verified
  statements, and MUST fall back to search results when none verifies.

**Degradation**

- **FR-030**: When the model is not configured, not running, or past its
  timeout, every command that needs it MUST be absent or answer with the
  notice in US1, and the non-AI path of D2 MUST hold. The UI MUST NOT wait
  on the model.

**Privacy**

- **FR-040**: A request MUST contain only the inputs D6 lists for its
  feature.
- **FR-041**: `show_model_input` MUST show the last request's exact text
  from memory. Requests MUST NOT be stored or logged.
- **FR-042**: Logs and the egress log MUST carry only the feature, ids,
  counts, bytes, duration and outcome (D7).
- **FR-043**: Summaries MAY be cached in the store, encrypted at rest with
  the rest of it. A cached summary MUST record the model that wrote it, and
  caches MUST be deleted with their switch.

### Key Entities

- **Feature switch**: a feature, on or off, per account.
- **Proposal**: a model's answer before the person acts on it. It is one of:
  a summary (statements, references), a reply suggestion (paragraphs), a
  query (a string the parser accepts), or an answer (statements,
  references).
- **Reference**: a message id and a passage, verified against the message's
  own text.
- **Embedding**: a message id, the model that computed it, and a quantised
  vector, deleted with its switch.
- **Model request record**: in the egress log only: the feature, ids,
  counts, bytes, duration and outcome.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With no `[focus.model]` section, Postio makes zero model
  connections, and no AI command appears on any surface. Asserted on every
  frontend's command sweep.
- **SC-002**: Across the corpus and the fake model's fabricated answers,
  zero statements with an unverifiable passage are shown.
- **SC-003**: Over the injection fixtures, zero commands run, zero
  connections are made beyond the one model call, and zero live links are
  drawn, for every feature.
- **SC-004**: A suggested reply reaches the server, the outbox or the saved
  draft in zero cases without `use_suggestion`.
- **SC-005**: Semantic search keeps 100% of full-text hits, and stays within
  the 100 ms local search budget at 100,000 messages, gated as counts with
  the nightly bench reporting.
- **SC-006**: A person reaches a thread summary in one key from the thread,
  and the thread stays operable while it is written.
- **SC-007**: Every request's content matches D6's table for its feature, in
  100% of the request-shape tests.

## Assumptions

- The person's runtime serves the OpenAI-compatible chat completions and,
  for semantic search, `/v1/embeddings`. No runtime is named in code
  (FR-169).
- `postio-mcp` stays as it is. External agents are ADR 0010's question, not
  this spec's.
- Triage (filtering and holding) is already Focus's model layer (spec 007
  FR-110 onwards). This spec adds no new triage.
- Writing help on the person's own text (rewrite, shorten, change tone) is a
  natural later feature with a lower injection risk. It is left for a later
  spec, so this one stays about reading mail.
- Spec 016's encrypted mail and spec 017's events are not sent to a model by
  this spec.
