# Architecture inventory: Postio, Flectar and Letter

Last reviewed: 2026-10-05. This is an investigation inventory, not an ADR,
implementation plan, or list of approved migrations. Keep it updated as
source evidence and experiments change the ranking.

## Baseline and evidence rules

- Postio: `e92515973809b64c7ed451c37ca5f8b47bde42d9`, fetched from
  `origin/main`. The shared checkout was older; this review used a separate
  worktree at the fetched revision.
- Flectar: `4c12945ed250e48600baa71ffb4b742d495ba6e9` in the adjacent
  checkout. References below pin that revision.
- Letter: `01b4c581dfb988def767ccb9272e96317455cece`. Its earlier review
  covered the GTK/Camel/EDS shell and mail/session integration; this pass
  concentrated on Flectar and Postio.
- Source inspection only: neither reference client was built or benchmarked.
  Benefits below are hypotheses unless a test or existing measurement is
  explicitly identified. Historical timings in Postio ADRs are evidence of
  the original problem, not measurements of this revision.
- Issue status is supplementary. Read the implementation: #1609 and #1612
  remain open while several mechanisms described in their original reports
  have already changed. Closed #1607, #1610 and #1614 are prior art, not
  evidence that their original defects remain.

## Subsystem inventory

| Subsystem | Postio today | Reference evidence | Assessment |
|---|---|---|---|
| Composition and frontends | `postio-host` owns store operations in the active app; `postio-client` carries typed requests/events; GTK, TUI and Swift/FFI share core/presentation logic | Flectar embeds `Core` behind a Slint shell; Letter delegates substantial mail/account work to Camel/EDS | Keep Postio's boundaries. A smaller crate count is not evidence of less runtime cost or better reuse. |
| Message identity and location | A `messages` row has one required `mailbox_id`; UID identity and `remote_id` lookup are mailbox-scoped | Flectar Gmail/JMAP resolve account-scoped remote identity, then write `message_folders` memberships | Strong candidate for a larger domain/schema redesign: A below. |
| Thread assignment | Stored thread links, pure assignment policy, merge handling and per-account ownership | Flectar uses provider thread IDs where available, otherwise reference/subject matching and recomputed summaries | No evidence to replace Postio's threading algorithm. Identity/location changes must preserve its merge semantics. |
| List reads | Paged store, bounded resident window, shared `Paging`; folder representative queries, count witnesses and sparse seek marks | Flectar maintains thread summaries and uses keyset pages, but still has correlated folder filters/count queries | Candidate B is further materialization, not adopting Flectar's SQL wholesale. |
| Search and indexing | Embedded encrypted Turso FTS; separate metadata/body tables; sync defers metadata trigger work and indexes each changed document once | Flectar explicitly updates a contentless SQLite FTS table | Repeated folder occurrences amplify work; address A first. Search execution deserves a bounded experiment, G. |
| Sync | `MailBackend`, durable operations, QRESYNC/IDLE, prioritized connection/write admission, selective body sections and batched section prefetch | Flectar separates metadata, priority bodies, history, bulk bodies and send workers; shared background byte/planning permits | Postio already has most scheduling concepts. Cross-account admission/lifetimes are candidate F. Flectar's IMAP wrapper explicitly lacks CONDSTORE/QRESYNC. |
| Event delivery | Nonblocking unbounded hub per subscriber, depth watermark; clients forward events into another unbounded queue; shared scope-aware paging policy | Flectar uses bounded broadcast with lag recovery, bounded UI queues and coalesced pending state | Candidate D must distinguish state invalidation from command outcomes. Simply dropping old events is incorrect. |
| UI reads | Many operations are async, but `blocking::now` adapts synchronous callbacks to host/store futures | Flectar's navigation reads use latest-only workers with bounded result delivery | Candidate C removes the synchronous presentation dependency rather than tuning individual queries. |
| Reader layout and painting | Disconnected, memory-safe Blitz worker produces an immutable whole-document display list, text/geometry indexes and low-resolution raster; separate tile pool | Flectar retains Blitz document state, supports paint-only resource updates and viewport tiles | Candidate E changes the document lifetime/invalidation model. Same Blitz beta.2 and Parley 0.11.1 make regression cases directly relevant. |
| Composer | `Document` is authoritative, WebKit is the current editing projection; history keeps before/after document snapshots | Flectar owns rich text/style runs, fragment transactions and retained cosmic-text layout independently of its IME bridge | Complete accepted ADR 0039. Flectar's delta history is useful design evidence; its custom visible editor is not a reason to replace GTK text editing. |
| Lifecycle/resources | Lazy surfaces, per-view tile budget, count-limited prepared-body cache and application-side remote fetching | Flectar has per-document/global image budgets, explicit suspension and cancellation-aware permit ownership | Smaller findings fit into the larger resource ownership work in F. |
| Measurement | Deterministic SQL/count/plan gates, renderer counts, nightly measurement tier and existing first-sync memory probe | Flectar has isolated decoder allocation probes and matched repeated RSS/PSS reports | Extend existing Postio tooling; do not replace deterministic merge gates with shared-runner timings. |

Primary Postio anchors: [architecture](../ARCHITECTURE.md),
[schema](../../crates/postio-storage/src/schema.rs),
[thread queries](../../crates/postio-storage/src/repository/threads.rs),
[local list store](../../crates/postio-runtime/src/store/local.rs),
[paging policy](../../crates/postio-ui/src/paging.rs),
[host](../../crates/postio-host/src/lib.rs),
[sync runtime](../../crates/postio-runtime/src/engine.rs),
[body backfill](../../crates/postio-sync/src/backfill.rs).

## Larger change candidates

### A. Separate canonical message content from mailbox occurrences

**Status: strongest new structural candidate; design and experiment required.**

Postio's `messages.mailbox_id` is mandatory. `find_by_remote_id` looks up
`(mailbox_id, remote_id)`, and IMAP uniqueness is
`(mailbox_id, uid_validity, uid)`. Body columns and the flattened search
document belong to that occurrence. The duplicate-search problem is already
recorded in [#1526](https://github.com/dlapiduz/postio/issues/1526).

Flectar's [Gmail sync][fgmail] finds the existing message by account and
provider message ID, then replaces its [folder memberships][fmembership].
Its [JMAP sync][fjmap] uses the same membership table. This is a working
example of content identity being different from folder location, although
the helper's Gmail-specific name and provider branches are not a suitable
Postio abstraction.

Investigate an account-scoped content record, separate occurrence/membership
records, and body/search data attached once to the content record. REST/JMAP
stable IDs can establish identity directly. IMAP occurrences retain their
mailbox, UIDVALIDITY and UID and may remain distinct when content identity
cannot be established safely. Folder flags, pending actions and saga
coordinates must retain their proper scope. RFC Message-ID alone does not
establish content equality; never merge unrelated mail merely because that
header matches. Cross-account copies retain independent account/action
ownership even if immutable payload storage is shared.

**Potential payoff:** remove repeated body fetch/storage/indexing for
recognized copies, make folder changes membership writes, and reduce the
need to repair duplicate results in search/list presentation.

**Proof before a migration:** one synthetic logical message in several
folders produces one body/index document and several addressable
occurrences. Folder queries/facets still agree; moves, copy, expunge,
UIDVALIDITY reset, undo and uncertain sends retain correct targets. Measure
both write amplification and saved storage/index work. Inherits the backend
seam in [ADR 0018](../decisions/0018-jmap-and-gmail-backends.md); requires a
new cross-crate contract if adopted.

### B. Make mailbox/thread list rows a maintained read model

**Status: existing proposal, partly mitigated; not a newly discovered bug.**

The folder query still chooses representative messages with `NOT EXISTS`
and computes folder-slice aggregates. Counts have witnesses and local
adjustments; random access has sparse cursor boundaries. Those fixes landed
and matter. There is still no `mailbox_threads` table in the reviewed schema.

[ADR 0040 section 3](../decisions/0040-the-store-keeps-few-connections-maintains-its-counts-and-budgets-its-index.md)
already proposes materialized `(mailbox, thread)` rows with a sort key and
folder-slice aggregates. Flectar's [thread summaries][fthreads] illustrate
the write/read trade-off, but its correlated folder filters and scalar counts
do not solve all of Postio's query costs.

**Potential payoff:** replace repeated representative/aggregate derivation
with indexed reads and cheap exact counts; reduce reliance on cache witness
repair and boundary rebuilds. Coordinate with A so the projection is based
on memberships rather than cementing an occurrence-as-content schema.

**Proof:** compare query plans/work at shallow and deep positions and after
large sync/move bursts; count incremental maintenance at writes, including
long threads and thread merges. A trigger that rescans a growing thread per
insert can move the cost into a quadratic first sync. Preserve time-dependent
snooze handling and measure the complete write path.

### C. Remove synchronous host/store waits from presentation callbacks

**Status: source-supported structural opportunity; contention impact unmeasured.**

[blocking::now](../../crates/postio-session/src/blocking.rs) blocks its
calling thread until an async operation completes and supports nested
calls through a thread-local runtime. The module documents why this bridge
exists. [CID resolution](../../crates/postio-session/src/reading.rs) is one
concrete host read reached from a synchronous resource callback.

An indexed query bounds database work, not waiting for reader admission,
blob decryption, runtime scheduling or nested callbacks. Flectar's
[navigation worker][fwork] separates async acquisition from UI application.

Investigate asynchronously prepared presentation snapshots/resources, with
synchronous callbacks answering only from already-owned data. This fits
`postio-host`/`postio-client`: no new store access in GTK, and no network
wait added to the interaction path. Native compose under ADR 0039 removes
some browser callbacks but does not by itself audit every blocking caller.

**Potential payoff:** remove a class of UI stalls, nested-runtime failure
modes and thread ownership complexity, not just save a query per click.

**Proof:** hold an artificial host read pending and verify that input and
UI state transitions continue; stale reads never repaint a new selection.
Add a boundary check only after defining legitimate non-UI blocking callers.
For resource lookup, test message-scoped CID ownership and bounded prepared
bytes. Do not assume every present `blocking::now` call is on the UI thread.

### D. Revisioned query sessions and bounded state subscriptions

**Status: new architectural hypothesis; must respect ADR 0013's event contract.**

Postio already shares paging/generation policy and refetches affected
resident pages for ordinary message changes. Some membership changes still
invalidate a whole window. The hub and each client's forwarded event queue
are unbounded, with diagnostics rather than an admission bound.

Flectar's [pending updates][fstartup] merge changed thread IDs into a bounded
set, promote overflow to a full invalidation, and recover from broadcast lag.
Its [UI dispatcher][fdispatch] bounds delivery and coalesces wakes. Those
mechanisms are state-refresh evidence, not a replacement for total delivery
of Postio command outcomes and tracked invocation envelopes.

Investigate host-owned query sessions: subscribe before taking a revisioned
snapshot; return bounded pages plus scoped changes; resnapshot on a detected
gap. Keep durable command acceptance/outcomes separate from replaceable
state notifications. Start with one list scope, not every frontend surface.

**Potential payoff:** one subscription/read/invalidation lifecycle across
frontends, bounded slow-consumer memory, and fewer full reloads while keeping
row identity and scroll anchors stable.

**Proof:** a mutation during snapshot acquisition is neither missed nor
applied twice; a suspended consumer converges after overflow; two subscribers
both see the required outcomes; page boundaries remain correct when sorting
or membership changes. Carry forward [ADR 0013](../decisions/0013-event-fanout.md)
and [shared paging](../../crates/postio-ui/src/paging.rs), rather than
reimplementing their fixes.

### E. Retain and incrementally update the conversation render document

**Status: substantial renderer experiment; some scaling work is already specified.**

[render](../../crates/postio-render/src/render.rs) creates a Blitz document,
may lay it out again for theme repair, and records the entire display list
and geometry/text indexes. Raster tiles are windowed; layout and document
assembly are not made incremental by that fact. The conversation code
explicitly says its hundred-message preparation bound (FR-051) is not yet
implemented. [Host readings](../../crates/postio-host/src/reading.rs) also
return a whole vector before the app's per-message preparation pipeline runs.

Flectar [retains document state][frenderer], prioritizes visible resources,
and tests that a fixed-size image arrival changes only affected tiles without
new layout in its [renderer regressions][fregression].

Investigate a worker-owned document session with distinct content, geometry,
paint and viewport revisions. Hand immutable snapshots/patches to GTK, never
the mutable Blitz document. Introduce bounded conversation acquisition and
preparation around the visible/focused region while preserving the product's
continuous conversation and cross-message selection/find semantics. A fold
or intrinsic-size image can still require wider reflow; local invalidation
must be proven, not assumed.

**Potential payoff:** one arrival or presentation change stops rebuilding
every message, and long threads stop loading/preparing all bodies before a
useful first view. This is more than increasing the tile cache.

**Proof:** fixed-size image delivery performs no layout and leaves other
tiles identical; intrinsic-size changes reflow correctly; preparation counts
stay bounded for a long thread; resize/theme/fold/find/copy/accessibility and
scroll anchoring remain correct. Preserve the no-network/no-C graph from
[ADR 0042](../decisions/0042-the-reading-renderer-is-disconnected-and-memory-safe.md).
Flectar's bounded synchronous resource-event patch cannot be copied blindly:
Postio's same-worker resource delivery needs a deadlock-free drain strategy.

### F. Give the host process-wide admission and ownership of expensive work

**Status: generalize existing mechanisms; shared-runtime topology is conditional.**

Postio already bounds sync pass admission, gives interactive writes priority,
reserves protocol capacity, bounds backfill planning, streams whole-message
payloads to disk, and caps a selective prefetch batch. Flectar already shares
[body planning/encoded-byte permits][fsync] across accounts and
[image decode admission][fscheduler] across documents. Its comments correctly
distinguish encoded estimates from actual decoded memory/RSS.

Investigate explicit host-level resource classes for background fetch,
parse/decode, render, tile and index work. Define byte budgets as well as job
counts and foreground reserves; a permit belongs to the actual worker until
that worker finishes, including blocking work after async cancellation.
Bound queues without allowing blocked producers to retain unlimited payloads
outside the queue. Use fair account admission to prevent one archive from
monopolizing the process.

The current engine owns an OS thread and a one-worker Tokio runtime per
account. A shared runtime with per-account actors could remove runtime and
shutdown coordination, as Flectar's actor topology demonstrates. Treat that
as a second experiment: Postio's blocking paths and large async stacks must
be understood first. Reserved stack address space is not resident memory.

**Potential payoff:** predictable multi-account peak resources, coherent
foreground priority, and testable cancellation/shutdown across subsystems.

**Proof:** instrument admitted bytes/jobs/live workers while several accounts
sync and the reader/compose remain active. Delay cancellation of a blocking
worker and assert its permit stays held. Compare thread/scheduler costs before
changing runtime topology; retain durable operations and backend isolation.

### G. Treat a search as one bounded, revisioned execution session

**Status: investigate after A; existing #1612 is partly implemented.**

Current search completion reads maintained mailbox `bodies_owed`.
[facets](../../crates/postio-index/src/executor.rs) already combines current
scope count/refinements in one walk. Other scopes perform separate capped
counts: their counts describe switching scope, not narrowing the current one.
Do not repeat #1612's original claim of five separate facet walks as current
fact.

Consider reusing safe intermediate work between count, ranked candidates,
hydration and facets under one query revision, with cancellation when a query
is superseded. One globally capped hit set is not an equivalent shortcut:
one scope could fill the cap and hide matches from another. Preserve each
scope's cap/meaning and complete result identity when evaluating reuse.

**Proof:** search list, totals and facets agree for duplicates and broad
queries spanning uneven scopes; quantify postings/rows examined as well as
statement counts. Use [#1526](https://github.com/dlapiduz/postio/issues/1526)
and [#1612](https://github.com/dlapiduz/postio/issues/1612) as existing work
records, not new duplicates.

## Accepted large work that the references reinforce

**Finish native compose (ADR 0039), rather than commission a toolkit rewrite.**
This removes a browser engine, script-message bridge and HTML round-trip
from editing. It is already accepted and remains unfinished in the reviewed
tree. Flectar's [rich document][fcompose] stores fragment transactions,
selection and a byte-accounted history independently of the visible editor.
Postio's [history](../../crates/postio-body/src/edit.rs) retains up to 200
before/after document snapshots. Investigate document-native edit intents
and compact transactions together with the native projection. Keep
Postio's block structure, paste normalization, draft semantics and distinct
mail/text undo. A long draft test should constrain retained history bytes
and undo/redo behavior, including multibyte text and formatting.

## Smaller findings kept in the inventory

These remain useful; they are not the extent of the architectural review.

- Decode image limits/downsampling, per-document totals, global admission,
  foreground/viewport priorities and cancellation-aware worker permits.
- Latest-only pending reads/renders; bounded tile jobs that stop retaining
  obsolete document snapshots; coalesced UI wakeups.
- Shared immutable remote bytes and cache budgets by bytes, not only entries;
  explicit transient-failure retry and progressive delivery after consent.
- Release disposable raster/layout caches on view suspension or pressure.
- Reproduce Flectar's floats/table/font-fallback regressions against the shared
  renderer versions before adopting selected patches.
- Extend Postio's existing memory probes with isolated decoder allocation
  counts and matched repeated process measurements. Flectar's advertised
  minimum memory figure is not a comparative benchmark.

## Alternatives not justified by this review

- **Move Postio to Slint or merge its crates.** No measured benefit establishes
  that migration's cost; Flectar also has large shell/core modules and custom
  editing/rendering responsibilities. Retain native GTK accessibility and
  Postio's checked boundaries.
- **Replace Turso/embedded FTS with Flectar's SQLite or a separate plaintext
  index.** Flectar's database is not evidence that Postio can discard its
  encryption model. ADR 0040 explicitly favors budgeting the embedded index
  over moving it out. Decoupled indexing would require replay, deletion,
  rebuild and search-freshness contracts; current metadata batching/body
  separation already remove some amplification. Measure remaining costs first.
- **Move all HTML rendering to a sandbox process now.** Flectar's bounded PDF
  worker and subprocess hang regression tests are availability evidence for
  different workloads, not a production HTML architecture to copy. ADR 0042
  explicitly rejected a sandboxed reader. Postio's abandoned render threads
  do warrant live-worker accounting and a recovery bound. Revisit hard
  termination only if an availability experiment falsifies the current policy.
- **Replace sync with Flectar's protocol code.** Postio has the protocol trait,
  durable queue, selective batching and more capable IMAP incremental sync.
  Keep Pimalaya-first implementation policy and provider-neutral capabilities.
- **Adopt Flectar's AI, calendar/files suite, favicons or speculative fetches.**
  This review supplies no product direction to expand scope or change consent.
- **Move Postio's mail ownership into Camel/EDS because Letter does.** That
  would replace storage/sync contracts and encryption ownership; Letter's
  desktop integration is a separate optional integration question.

## Next evidence to collect, in order

1. Map A through backend identity, occurrence flags, local actions/undo and
   cross-account saga targets. Count duplicate bodies/index documents using
   synthetic multi-folder fixtures. Compare a small membership-model spike.
2. Compare B's incremental projection maintenance against representative reads,
   sparse cursor rebuilding and exact counts at two mailbox/thread sizes.
3. Trace production UI callback uses of `blocking::now`; test delayed host
   responses before deciding how much of C should move into query sessions D.
4. Measure E's full assembly/layout/index work on long conversations and image
   arrival bursts; compare retained-worker and bounded preparation strategies.
5. Attribute multi-account peaks to live body/decode/render/index jobs and
   retained caches before choosing F's budgets or runtime migration.

Update this note with the revision, inspected paths, what changed, observed
counts, and any falsifying result. Confirmed implementation work belongs in
the existing issue/spec where possible; unresolved hypotheses here are not
automatically `ready` tasks. Architectural adoption requires its own decision
or spec, not an assertion that a reference client is faster.

[fgmail]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/crates/flectar-mail-core/src/sync/gmail.rs#L1630
[fmembership]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/crates/flectar-mail-core/src/db/repo/gmail.rs#L107
[fjmap]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/crates/flectar-mail-core/src/jmap/sync.rs#L877
[fthreads]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/crates/flectar-mail-core/src/db/repo/threads.rs#L164
[fwork]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/mail_work.rs#L89
[fstartup]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/startup.rs#L437
[fdispatch]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/ui_dispatch.rs
[frenderer]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/renderer.rs#L582
[fregression]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/renderer/regression.rs#L370
[fsync]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/crates/flectar-mail-core/src/sync/engine.rs#L2559
[fscheduler]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/remote/scheduler.rs
[fcompose]: https://github.com/flectar/mail/blob/4c12945ed250e48600baa71ffb4b742d495ba6e9/src/rich_compose.rs
