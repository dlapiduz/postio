# Feature Specification: The store, rebuilt on Turso

**Feature Branch**: `feature/turso-store`

**Created**: 2026-09-12

**Status**: Draft

**Input**: Rebuild Postio's storage layer natively on Turso, replacing
SQLCipher and rusqlite entirely — no translation layer. Encryption from the OS
keyring, search rewritten onto Turso's own full-text index, bodies stored as
text rather than compressed blobs, and no migration: a store in the old format
is rebuilt by resyncing.

## Why this exists

Postio's store is SQLCipher, and SQLCipher's crypto is OpenSSL. That costs a
28-second uncacheable build step per cold tree, an `unsafe` call to stop
libcrypto tearing itself down under a writing thread (#794, #699), and a page
path that is two passes — cipher, then MAC — where a modern AEAD is one.

Three spikes measured the alternatives. A Rust crypto provider for SQLCipher
works and buys only the build time. SQLite3 Multiple Ciphers is 1.84x faster on
the page path but is a single-maintainer C dependency. Turso is the Rust
rewrite: **96 of 102 of Postio's head-schema objects already apply to it
unchanged, every trigger included**, its encryption is AES-256-GCM in Rust, and
its store proved encrypted and key-refusing under test.

What it does not have is FTS5. That is the whole of the work below.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A mailbox that opens, encrypted (Priority: P1)

Someone starts Postio with no store. It creates one, opens it under a key from
the OS keyring, and brings up a window. Nothing about the mail on screen tells
them the engine changed.

**Why this priority**: Everything else needs a store to exist. It is also
where the one non-negotiable lives: a mail store that is not encrypted at rest
is a defect, not a slower version of the feature.

**Independent Test**: Create a store through the application's own opening
path, then read the file: it must not begin `SQLite format 3`, must contain no
message text or table name in the clear, and must refuse to open under a
different key.

**Acceptance Scenarios**:

1. **Given** no store on disk, **When** the application starts, **Then** a
   store is created, the schema is at head, and the window opens.
2. **Given** a store created under one key, **When** it is opened under
   another, **Then** it is refused and the user is told, rather than opening
   empty.
3. **Given** a store holding mail, **When** its bytes are scanned, **Then** no
   subject, body, address or table name appears in the clear.

---

### User Story 2 - Mail arrives and can be read (Priority: P2)

Someone adds a real IMAP account. Sync writes messages into the store, the
list fills, and opening a message shows its body.

**Why this priority**: This is the slice that makes the rebuild real rather
than theoretical — a store nothing writes to proves nothing. It is also the
bulk of the surface: every repository, every write path.

**Independent Test**: Against a loopback IMAP server with a known mailbox,
drive a first sync and assert the rows land, the list model fills, and a body
opens — the same assertions the existing suites make, over the new engine.

**Acceptance Scenarios**:

1. **Given** an account and a server with mail, **When** a first sync runs,
   **Then** the messages, mailboxes, threads and their counts are in the store.
2. **Given** mail in the store, **When** the window opens, **Then** the list
   fills from it without a network round trip.
3. **Given** a message in the list, **When** it is opened, **Then** its body
   is shown.
4. **Given** a message archived or flagged, **When** the action is taken,
   **Then** the store records it locally first and the queue carries it.

---

### User Story 3 - Search that finds what it finds today (Priority: P3)

Someone searches their mail. The same query that works today returns the same
messages, in an order that is at least as useful.

**Why this priority**: Search is one of the three things the constitution says
Postio must do better than the alternatives, and it is the only part of the
store that cannot be ported — FTS5 has no counterpart here, so it is rewritten
rather than moved.

**Independent Test**: One corpus, two engines, the same query strings: every
message today's search returns must be returned by the new one. Differences
are failures, not deltas.

**Acceptance Scenarios**:

1. **Given** a mailbox indexed under the new engine, **When** a term is
   searched, **Then** the messages containing it are returned.
2. **Given** a message whose subject holds an accented word, **When** the
   unaccented spelling is searched, **Then** the message is found — today's
   behaviour, which the engine does not provide and the application must.
3. **Given** a query with an operator and free text, **When** it is run,
   **Then** it means what the same string means in the sidebar and in
   `config.toml`.
4. **Given** a message deleted, **When** its term is searched, **Then** it is
   not returned.

---

### User Story 4 - It still feels instant (Priority: P4)

Someone with a large mailbox opens Postio, scrolls, and searches. Nothing is
slower than it was.

**Why this priority**: The budgets are functional requirements, and the
instrument that currently defends them reads SQLite's trace hook — which does
not exist here. Without a replacement the budgets become documentation again,
which is the state #1434 was filed about.

**Independent Test**: A gate that fails when a read becomes proportional to
the mailbox, over two stores an order of magnitude apart, with the same
numbers on any machine.

**Acceptance Scenarios**:

1. **Given** two stores an order of magnitude apart, **When** a window is
   opened on each, **Then** the work done is the same and bounded.
2. **Given** a mailbox of 100,000 messages, **When** a page is read, **Then**
   it costs what a page of a thousand costs.

---

### Edge Cases

- **A store in the old format.** It is not read and not migrated. The
  application says so plainly and offers the one thing that recovers it —
  starting again from the server — rather than opening it empty or partly.
- **A key the keyring will not give up.** Unchanged from today: the mail does
  not open, and the screen says why (ADR 0014 Q3).
- **An in-memory store.** The engine refuses to key one. Anything that opened
  an unencrypted in-memory database must either stop doing so or be explicit
  that it holds no mail.
- **A query with no bound.** The engine's rows arrive as a stream; a read that
  does not limit itself can hold a whole mailbox in memory, which §18 forbids.
  The design must make an unbounded read visible rather than possible.
- **An engine upgrade that changes the file format.** The dependency is
  pre-1.0. A store that a newer build cannot read must be detected and
  reported, not opened.
- **A body that is not text.** Bodies are stored as text so they can be
  indexed; anything that is not must still round-trip byte for byte.

## Requirements *(mandatory)*

### Functional Requirements

**The store**

- **FR-001**: The application MUST create and open its store through Turso,
  with no rusqlite-shaped translation layer between the storage layer and the
  engine.
- **FR-002**: The store MUST be encrypted at rest with a key held in the OS
  keyring, and MUST NOT open under any other key.
- **FR-003**: The store MUST refuse to open a database written by a different
  engine or format rather than opening it partly.
- **FR-004**: The schema MUST be created at head. No migration path from an
  existing store is provided.
- **FR-005**: Every repository operation the application performs today MUST
  be available: accounts, mailboxes, messages, threads, drafts, contacts,
  labels, the operation queue, settings, and the egress log.
- **FR-006**: A write MUST be local-first: recorded in the store before the
  network is told, and never awaited by the UI.

**Search**

- **FR-007**: Search MUST match messages by subject, sender, recipients,
  filenames, list id and body.
- **FR-008**: Search MUST match a term irrespective of diacritics in either
  the query or the text, preserving today's behaviour. The engine's tokenizer
  does not fold them, so the application MUST normalise text identically on
  the way into the index and on the way into a query.
- **FR-009**: Search MUST support the query language unchanged — the same
  string means the same thing in the box, the sidebar and `config.toml`.
- **FR-010**: Search results MUST be ranked, and the ranking MUST continue to
  combine textual relevance with recency and sender affinity.
- **FR-011**: A message removed from the store MUST stop matching.
- **FR-012**: Search MUST answer without a network round trip.

**Bodies**

- **FR-013**: Message bodies MUST be stored in a form the full-text index can
  read, and MUST round-trip byte for byte for display and reply.
- **FR-014**: The store MUST NOT hold a second copy of body text beyond what
  the index requires.

**Keeping it honest**

- **FR-015**: There MUST be a gate that fails when opening a window, reading a
  page, or running a search becomes proportional to the size of the mailbox.
- **FR-016**: That gate MUST produce the same verdict on any machine — it MUST
  NOT be a wall-clock threshold.
- **FR-017**: An unbounded read MUST be prevented or detectable, not merely
  discouraged by convention.
- **FR-018**: No test in the default suite may reach the network.
- **FR-019**: The crate boundaries MUST hold: the view layer speaks no
  database, and the model crate depends on no engine.
- **FR-020**: Logs MUST carry ids, counts and outcomes, never message content.

### Key Entities

- **Store**: One encrypted database per installation, holding every account's
  mail, its metadata and its search index.
- **Message**: Its headers, flags, thread membership, and its body as
  indexable text.
- **Search index**: Derived entirely from the store's own rows, rebuildable
  from them, and never the only copy of anything.
- **Store key**: 256 bits from the OS keyring. Never in config, never in a
  log, never derived from a passphrase on the opening path.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A person can add a real account, receive their mail, read it and
  search it, with nothing on screen revealing that the engine changed.
- **SC-002**: Every message that today's search returns for a given query is
  returned by the new search over the same corpus — including queries whose
  terms differ from the text only by diacritics.
- **SC-003**: No subject, body, address or table name from any message can be
  found in the store's bytes.
- **SC-004**: A store opened with the wrong key yields no mail and an
  explanation.
- **SC-005**: Opening a window costs the same measured work over a mailbox of
  a thousand messages as over one of ten thousand.
- **SC-006**: A page of a hundred-thousand-message folder costs what a page of
  a thousand costs.
- **SC-007**: Startup to a usable window stays within the 500 ms budget on a
  mailbox of the reference size, and search within 100 ms.
- **SC-008**: The whole test suite passes with no test reaching the network.

## Assumptions

- **No migration, by instruction.** The maintainer will delete the existing
  mailbox and resync. The store is a cache of the server, which is what makes
  this cheap.
- **Compression is given up.** A column-indexing full-text engine cannot
  tokenise compressed bytes, so bodies are stored as text. Measured cost: 1.39x
  on a corpus (an upper bound, since repetition flatters a dictionary) against
  a recorded 2.19x on a real 1.43 GB text axis. Accepted when this work was
  asked for.
- **Turso replaces SQLCipher rather than joining it.** There is no dual-engine
  abstraction; "rebuild the app with Turso" is read as replacement.
- **The dependency is pre-1.0 and both features used are experimental and
  unaudited.** This is accepted for a build the maintainer tests, and is the
  reason SC-003 and SC-004 are acceptance criteria rather than assumptions.
- **First use is against a fresh resync, not the live store.** Nothing here
  reads an existing store, so the live one cannot be damaged by it.
- **Diacritic folding moves into the application.** The engine will not do it
  and the application owns both the write and the query path, so it can.
- **The four indexes on `WITHOUT ROWID` tables** are resolved by dropping
  `WITHOUT ROWID` from those tables rather than by losing the lookups.

## Out of Scope

- Migrating an existing SQLCipher store.
- Keeping SQLCipher working alongside Turso.
- Changing the query language, the keyboard, or any surface a user sees.
- Vector or semantic search, which the engine offers and Postio defers to E12.
