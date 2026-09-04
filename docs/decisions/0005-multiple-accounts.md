# ADR 0005 — Multiple accounts and the unified inbox

- **Status:** Accepted — **GO** (2026-08-24), **substantially revised 2026-08-24**,
  **Q5 answered from measurement 2026-08-26**
  ([#435](https://github.com/dlapiduz/postio/issues/435)), **Q5b decided
  2026-08-26** ([#186](https://github.com/dlapiduz/postio/issues/186)),
  **Q6a decided 2026-08-27** ([#464](https://github.com/dlapiduz/postio/issues/464)),
  **Q6b decided 2026-08-28** ([#470](https://github.com/dlapiduz/postio/issues/470)),
  **Q6c decided 2026-08-28** ([#471](https://github.com/dlapiduz/postio/issues/471))
- **Date:** 2026-08-24
- **Issue:** [#1 Multiple accounts & unified inbox](https://github.com/dlapiduz/postio/issues/1)
- **Unblocks:** [#64](https://github.com/dlapiduz/postio/issues/64) (add-account
  flow), and is the reason [#2](https://github.com/dlapiduz/postio/issues/2)
  (OAuth) has somewhere to put a second provider
- **Decision:** **an engine per account, one database, and a unified inbox that
  is a scope rather than a mailbox.** A thread never spans accounts; the
  unified list groups threads at read time. `AppState.account: Option<AccountId>`
  becomes `AppState.scope: Scope`. **A cross-account move is a three-step saga
  that never deletes before the copy is confirmed.** **Every aggregated view
  reports whose data is missing** rather than silently omitting it.

> **Revised, and what changed.** The first version answered storage and
> concurrency and stopped there. Reviewing it against the tree found four
> things it got wrong or ducked, and they are the ones that decide whether
> multi-account is trustworthy rather than merely present:
>
> 1. **Cross-account move was not mentioned at all** — the hardest problem in
>    the feature, and the one that can lose mail. Now §Q9.
> 2. **Partial failure was not designed.** "Two accounts sync concurrently"
>    says nothing about what the unified list shows when one of them is down,
>    and a view that silently omits an account is one the user acts on wrongly.
>    Now §Q10.
> 3. **The event vocabulary is account-thin** and the first version did not
>    notice. `MessagesChanged { messages }` carries no account at all. Now §Q11.
> 4. **`ConnectionState` cannot say *why*** an account is failing, so it cannot
>    drive the one action that matters — reauthorise *this* account. Now §Q10.
>
> Nothing here contradicts the original storage or concurrency decisions; they
> were right and are unchanged. The revision is what sits on top of them.

---

## What is already built

More than the issue implies, which is why this is wiring rather than a
redesign. Measured at `0e0ec08`.

| Piece | State |
|---|---|
| `accounts` and `identities` tables | Built, no cardinality assumption |
| `account_id` on `mailboxes`, `threads`, `messages`, `sync_state`, `operation_queue`, `contacts` | Built, indexed |
| `contacts.account_id NULL` meaning "shared across accounts" | Built, with the partial unique indexes to match |
| `[accounts.<id>]` as a map, with `default = true` | **Gone** — #470 retired the section; see the note under Unified, below |
| `AppState.connections: BTreeMap<AccountId, ConnectionState>` | Built — status is already per account |
| `Engine` keyed to one account, owning one connection on one thread | Built (`runtime/src/engine.rs`) |
| SQLite in WAL with a connection pool and a 5s busy timeout | Built (`storage/src/db.rs:73`) |
| **`first_account()`** | The cut. `app/src/lib.rs:389` and `:449` |

The single-account assumption lives in exactly two call sites in the
composition root and in `AppState.account`. Everything below it is already
account-aware.

---

## Q1 — One database or one per account?

**One database.** It is what is built, and the alternative is worse in the
place that matters.

A database per account would make the unified inbox a cross-connection join,
which SQLite can do with `ATTACH` but which then makes every list query's plan
depend on how many accounts are configured. It would also fork the FTS5 index,
so a search across accounts becomes N searches merged in Rust with no way to
rank them against each other — and search is Postio's primary way to move
around (`PRODUCT.md` §7).

The cost of one database is writer contention, and Q3 answers it.

---

## Q2 — Does a thread span accounts?

**No.** `threads.account_id` stays `NOT NULL`, and
`idx_threads_account_last_at` stays the list's index.

The temptation is real: the user is cc'd on one conversation at two of their
addresses, and it is one conversation to them. But a thread in Postio is not
only a display grouping — it is **sync state**. JWZ threading runs over an
account's message set; `threads.last_at` drives a per-account index; removing
an account has to remove its threads. A thread whose `account_id` were nullable
would need re-keying on every account add and split on every account removal,
and it would lose the index that makes the message list windowed rather than
loaded (`ARCHITECTURE.md` §4).

**Instead, the unified list groups threads at read time.** A `ThreadGroup` is
one or more threads from different accounts whose JWZ root carries the same
`RfcMessageId`, or — where a root is missing, which is common — the same
normalised subject within the coalescing window `postio-model::subject` already
defines. Grouping is computed by the same paged query that builds the list, over
`idx_messages_rfc_message_id`, which already exists.

Two consequences worth stating rather than discovering:

- **A message the user received at two addresses appears once.** Deduplication
  is by `RfcMessageId` within a group. The copies are distinct rows in distinct
  accounts and both stay; the *list* shows one.
- **An action on a group hits every copy.** Archiving a unified-inbox row
  archives it in both accounts, as two operations in two per-account queues.
  This is the only answer that matches what the user believes they did. It is
  also why `Selection` staying a predicate (`ARCHITECTURE.md` §4) matters here:
  the expansion from group to messages happens in the store, not in a `Vec` the
  frontend built.

---

## Q3 — Concurrency: how do N engines share one database?

**One engine per account, each on its own thread with its own connection,
exactly as `runtime/src/engine.rs` already builds one.** Nothing about that
design assumed there was only one.

The pieces that make N of them safe are already in place:

- **WAL** (`db.rs:73`) — readers never block the writer, so the UI's list
  queries are unaffected by a sync pass on another account.
- **`busy_timeout = 5000`** (`db.rs:79`) — SQLite allows one writer at a time;
  a second engine's write retries rather than failing. Sync writes are short
  batched transactions, so five seconds is many orders of magnitude of headroom.
- **Per-account operation queues** — `idx_operation_queue_drain` is
  `(account_id, state, next_attempt_at, id)`. Two drainers on two accounts never
  see each other's rows.

**The one thing that must be added: a bound on concurrent engines.** Each
engine holds an IMAP connection, a TLS session and a connection from the pool.
`Database::open_with` takes `max_connections`; the composition root must size
the pool from the account count rather than from a constant, and must refuse to
start more engines than the pool can serve — otherwise the tenth account
deadlocks waiting for a connection that a sync pass is holding.

**What must *not* be added: a global sync lock.** Serialising accounts would
make one slow or unreachable server stall every other account's mail, which is
the failure mode multi-account exists to avoid.

**The test that proves the criterion.** Issue #1's first acceptance criterion —
"two accounts sync concurrently without interference" — is a `postio-runtime`
test over two `MailBackend` mocks with different latencies, asserting that the
fast account's messages land before the slow account's pass finishes, and that
each account's `sync_state` row reflects only its own pass. No network, per the
repository rule.

---

## Q4 — What the frontend selects: `Scope`, not an optional account

```rust
pub enum Scope {
    /// One account. Its mailboxes are real folders; `a` archives into one.
    Account(AccountId),
    /// Every enabled account at once. A view, never a destination.
    Unified,
}
```

`AppState.account: Option<AccountId>` becomes `AppState.scope: Scope`, and
`Option` stops carrying two meanings at once ("no account configured" and "no
account chosen").

**The unified inbox is a scope, not a mailbox, and the distinction is the one
`ARCHITECTURE.md` §6 already draws.** A real mailbox has a `UIDVALIDITY`, a
message set that physically lives there and a `MailboxRole`; mail *moves into*
it. Unified is a view over the `Inbox`-role mailbox of every enabled account.
Concretely:

- **Move has no meaning in Unified.** The move command is unavailable in that
  scope — not silently a no-op, *unavailable*, so it is absent from the palette
  and the cheat sheet, which is what `registry::reachable` for a context is for.
- **Archive does.** `a` archives each copy into *its own account's* Archive
  mailbox. There is no ambiguity: every message knows its account.
- **Compose from Unified uses the default identity** of the account marked
  `default = true` in `[accounts]`, and replying uses the identity of the
  account the message arrived at. A reply that went out from the wrong address
  is a bug the user notices after it is sent, so the identity picker shows the
  resolved identity rather than assuming it (`composer.rs` already has
  `identity_row` and `identity_only` for exactly this).

> **`[accounts]` no longer exists, and the default account needs a new home
> (2026-09-04).** [#470](https://github.com/dlapiduz/postio/issues/470) found
> `[accounts.<id>]` wired to nothing — editing it saved, re-parsed and changed
> nothing about the running account — and retired the whole section, correctly
> on the evidence it had. What went with it was the only home this ADR ever
> gave the default marker, and it was dead at that moment only because the
> thing that reads it, compose from Unified, is not built yet.
>
> Nothing in the decision above changes: there is a default account, and
> compose from Unified resolves through it. Where it is stored does.
> [#960](https://github.com/dlapiduz/postio/issues/960) carries the design — on
> the `accounts` row rather than in `config.toml`, mirroring
> `Identity.is_default` one level up, with unset a normal state that falls back
> to today's behaviour — and is the issue to read before implementing this
> paragraph.

**Per-account visual identification.** One accent hue per account, drawn as the
3px left border the PLATE design already gives the selected row, plus the
account's short name on the row in Unified scope only. Hues come from a fixed
ordered palette in `tokens.rs`, assigned by account position — generated, never
typed (`ARCHITECTURE.md` §10). Colour is never the only signal: the short name
carries it for anyone who cannot distinguish the hues.

**Sidebar shape.** Unified at the top as its own root, then one collapsible
section per account. `Context::Sidebar` already exists and the folder commands
are already reachable from it, so account switching is `registry` work rather
than new interaction.

---

## Q5 — Search across accounts

`postio-index` executes a parsed `postio-search` query against FTS5. Scope
becomes a *filter on the executor*, not a change to the query language: the
same query string means the same thing in either scope, which is
`ARCHITECTURE.md` §6's rule.

Two small additions:

- The executor takes `Scope`, and `Scope::Account` adds the `account_id`
  predicate it already has an index for.
  **Amended by Q5b (#186):** that sentence read two ways once it reached the
  SQL, and the one it turned out to mean is *two* scopes rather than one.
- The query language gains `account:` as a `Field`, so a saved search can pin
  itself to one account regardless of the scope it is run from. This is one row
  in `postio-search::Field` and one arm in the parser, and it keeps
  `[filters]` expressive enough to survive multi-account without a second
  syntax.

### Q5a — Unified search needs an index of its own (#435)

**Decided from measurement, 2026-08-26.** The two bullets above are still
right, and they quietly assumed a third thing that is not: that
`Scope::Unified` is `Scope::Account` with the predicate *removed*, and that
removing a predicate cannot make a query slower. It can, and here it does.

`idx_messages_account_list` is `(account_id, received_at DESC, id DESC)`. A
composite index can only supply `ORDER BY received_at DESC` when its leading
column is pinned to a single value. `Scope::Account` pins it; `Scope::Unified`
does not, and there is no `(received_at DESC, id DESC)` index without
`account_id` in front of it. The executor's recency path
(`Plan::fetch_form` → `Form::Probed`) exists precisely because it can walk
that index in the requested order and stop at `LIMIT`. Unscoped, it cannot,
and falls back to a full scan into a temp b-tree.

**Decision: add `idx_messages_recency ON messages (received_at DESC, id DESC)`
and let the planner take it when no account is named.** Unified search stays
one query with a predicate removed, exactly as Q5 says.

#### What was measured

A four-account corpus, 30,000 messages each, interleaved in time so a unified
recency order genuinely has to merge them. Best of five, warm.

Only the **broad** case is affected, and that follows from the executor's own
shape: `rank_by_relevance = has_match && total_hits <= RANK_BY_RELEVANCE_LIMIT`
(2,000). Under that threshold a query is ranked by `bm25` and takes
`Form::Driven`, whose `ORDER BY` is a score and never touches
`idx_messages_account_list` at all. Over it, the query is recency-ordered and
takes `Form::Probed`, which is the path that needs the index.

Broad match — every message contains the term, so `Form::Probed`:

| plan | time | query plan |
|---|---|---|
| account-scoped (today) | **7.8 ms** | `SEARCH m USING COVERING INDEX idx_messages_account_list` |
| unified, no new index | **20.3 s** | `SCAN m …` + `USE TEMP B-TREE FOR ORDER BY` |
| unified + `idx_messages_recency` | **18.8 ms** | `SCAN m USING COVERING INDEX idx_messages_recency` |
| per-account `UNION ALL`, merged | **32.4 ms** | 4 × `SEARCH …` + `USE TEMP B-TREE FOR ORDER BY` |

Narrow match — `Form::Driven`, ranked by relevance:

| plan | time |
|---|---|
| account-scoped | 2.0 ms |
| unified | 2.3 ms |

So the regression is real, it is a factor of ~2,600, and it is confined to one
of the two plan forms. The index removes it.

#### Why not per-account `UNION ALL`

It was the more conservative-looking option and it measured worse on every
axis that matters:

- **Slower**, 32.4 ms against 18.8 ms, and it *still* sorts in a temp b-tree —
  the merge needs one. It does not avoid the cost the index was supposed to
  buy off; it pays it in a different place.
- **The cost is per account**, because each arm has to walk far enough to fill
  its own `LIMIT` before the merge can discard most of it. Four accounts pay
  four times for a page of fifty.
- **It is a third scoring surface.** The two ranked plans already sum scores
  across two indexes (#379); merging ranked pages across accounts adds another
  thing that has to stay coherent with `rank_score`.
- **It forces "unified" to become a loop**, which is exactly the shape #186
  must not be pushed into by a storage decision — see below.

#### The objection to the index does not survive measurement

The case against a unified index was write throughput and disk on the largest
table in the store. Measured:

- **Disk: 2.2 MiB for 120,000 messages** — about 19 bytes per message, on rows
  that already carry a subject, recipients and an FTS posting list.
- **Writes: below noise.** 20,000 inserts timed with the index, without it,
  and with it again gave 2.73 s / 4.91 s / 2.76 s — the slow run is the middle
  one, in both orderings, which is the page churn from the delete between
  runs and not the index. There is no measurable insert penalty at this size.

That is a real cost and a small one, paid by every account to make a feature
work for the accounts that use it — the same trade `idx_messages_account_list`
itself already makes.

#### What this does and does not settle for #186

#186 asks what a search scope *is*. This answers it from the storage end:
**scopes stay predicates and compose as predicates.** A role scope keeps
building `role = 'inbox'`, and account scope is one more `AND` that may or may
not be present. `scope_condition` today builds `account_id = ? AND role =
'inbox'`, so "Inbox" means *this account's* inbox; with the index in place a
cross-account Inbox is the same query with the first conjunct dropped, not a
different query.

Had the answer been `UNION ALL`, "unified" would have become a loop over
accounts and every scope would have had to compose *inside* an iteration —
a different and worse enum.

**This paragraph originally said "settles", and that was too strong** — noted
when #186 was picked up the next day. Both candidate shapes there produce
queries with no account predicate, so the index is necessary for either and
selects neither; what it removes is the *loop*, which was the only structural
argument tying this decision to the enum's shape. The shape itself is Q5b.

#### Two things this does not settle

- **Facet counts run the query once per scope** (`executor.rs:201`), so the
  tri-tab multiplies whatever a unified search costs. At 18.8 ms a three-scope
  tab is ~56 ms: inside the `<100 ms` budget, but with much less headroom than
  the 7.8 ms single-account case implies. Worth its own measurement before the
  tri-tab grows a fourth scope.
- **`search_budget.rs` cannot see any of this.** It is 120,000 messages across
  *one* account, where unified and account-scoped are the same query, so it
  would stay green straight through this regression. A multi-account corpus is
  a prerequisite for #186's "search budget bench unchanged" criterion meaning
  anything, and is tracked separately. **Done with Q5b** — the bench now
  carries a four-account corpus and a `search_unified_common_word` shape
  beside its account-scoped control (#452).

### Q5b — Role scope and account scope are two fields, not one enum (#186)

**Decided 2026-08-26**, on the maintainer's delegation, when #186 stalled on
Q5's wording reading two ways: "the executor takes `Scope`" could mean the
role scope gains an `Account` variant, or that a second scope joins it.

**It is the second.** `SearchRequest` carries `scope: facets::Scope` — the
tri-tab's `AllMail`/`Inbox`/`Lists` — *and* `account: AccountScope`. They
compose; neither constrains the other.

The deciding argument is expressiveness, not tidiness. One enum holds one
value, so folding account into `facets::Scope` makes **"Inbox, this account
only" inexpressible** — a query a person with two accounts obviously wants,
given up to save a field. It would also leave an enum whose own `Scope::ALL`
deliberately omits a variant, which is a trap for the search bar that iterates
it positionally to draw the tri-tab.

#### Where the type lives, and why not a second one

The honest cost of two fields was a second, parallel enum: `postio-index`
cannot depend on `postio-core`, where #182 put
`state::Scope { Account(AccountId), Unified }`.

It does not need to. `postio-core` and `postio-search` both already depend on
`postio-model`, and `postio-index` depends on both — so the type lives in
**`postio-model` as `AccountScope`**, and `postio_core::state::Scope`
re-exports it. Nothing #182 wrote had to change, and there is one type rather
than two that must agree about what "unified" means. That agreement is not
hypothetical: `AppState.scope` and `SearchRequest.account` are supposed to be
the same answer to the same question, so two enums drifting apart is the list
and the search bar disagreeing about which accounts they are showing — the
class of lie Q10 forbids of aggregate views.

#### `account:` resolves in SQL, not in Rust

`postio-search` gains `Field::Account` and `Filter::Account(String)` and stops
there: the value stays the text the user typed, because resolving it needs the
store and a saved search in `[filters]` has to keep meaning the same thing
after an account is removed and re-added under a new id.

The executor resolves it the way `in:` already resolves a mailbox — as a
subquery, `m.account_id IN (SELECT id FROM accounts WHERE lower(display_name)
= lower(?) OR lower(address) = lower(?))`. A name that matches nothing yields
an empty `IN` set and so matches **nothing**, never everything: silently
dropping an unresolvable predicate would turn `account:typo` into an unscoped
search, which is Q10's lie again in a smaller costume.

#### One bug this shook out

`Plan::hydrate` recovered the account id as `self.params[0]`, on a comment
promising it was always bound first. Under `AccountScope::Unified` nothing is
bound there at all, and it panicked. The scope is now carried on `Plan`
explicitly rather than recovered by position — and sender affinity, which is
what that parameter fed, now counts sightings across every account when the
search is unified, which is the right answer rather than a convenient one:
"how often do I hear from this person" is about the person, not about which
inbox they landed in.

---

## Q6 — Adding, disabling and removing an account

- **`accounts.enabled`** already exists. A disabled account keeps its mail,
  stops its engine, and drops out of `Scope::Unified`. This is the reversible
  operation, and it is what the settings panel offers by default.
- **Removal** is `ON DELETE CASCADE` across mailboxes, messages, threads,
  sync state and the operation queue — and is therefore genuinely destructive
  and genuinely local. It must be `destructive: true` in the registry, and
  because `Recovery::None` on a destructive command is rejected at
  registration (`ARCHITECTURE.md` §2), it needs a real recovery: the account is
  soft-deleted and its rows are reaped on next start, so the toast's *Undo*
  has something to undo.
- **The keyring entry outlives the account row on purpose.** Removing an
  account removes Postio's copy of the mail, not the user's credential;
  deleting the keyring entry is a separate, confirmed step. A remove that
  silently destroyed a keyring item shared with another tool would be
  unrecoverable in a way nothing else here is.

### Q6a — Editing an existing account, and updating its credential (#464)

**Decided 2026-08-27.** Q6 settled disable and removal; it did not say how
any of the three reach a surface, or how a stored credential is updated once
onboarding has already written one. Two questions, one answer each.

**Ordinary fields (host, port, security, display name) are out of scope
here, and `config.toml` is not where they would go if they were not.**
The issue describes `postio-config`'s `[accounts.<id>]` schema
(`config/src/accounts.rs`) as a real, if unguided, editing path for these
today. It is not: `postio-app` reads and writes an
account's connection settings exclusively through
`postio_model::Account`/`AccountRepository` in SQLite, written once by
`onboarding::persist` and never again. `ConfigChanged::accounts` is
computed by `postio-config::change` but nothing downstream reads it — the
schema exists and round-trips in that crate's own tests, but the settings
panel's raw-text edits to `[accounts.<id>]` currently reach nothing a
running account looks at. Editing that section today looks like it works
(it saves, it re-parses, no error) and silently does not.

That gap predates #464 and is bigger than it: making `config.toml` a real
edit path for ordinary fields means either a reconciler that writes
`AccountConfig` changes into the SQLite row (a second writer to reconcile
against sync, and a place a stale keyring reference or an id mismatch has
to be handled) or retiring the TOML schema outright — a decision this ADR
does not make. **#464 ships none of that.** Its three acceptance criteria
never asked for host/port/security editing, and this amendment does not
expand them to it. Filed separately as
[#470](https://github.com/dlapiduz/postio/issues/470).

**What the raw editor cannot do gets exactly three small, targeted
affordances in the settings panel, next to each account's row — not a new
screen, and not `CommandId` registry entries.** The registry's commands are
keyboard-first verbs meaningful from a bare keystroke, in a `Context` the
keymap resolver can name — and there is no `Context::Settings` at all today,
because the settings panel has never had anything context-worthy to reach:
it is a raw `GtkTextView`, not a widget tree with rows. All three of these
need a specific account as their payload with no sensible keystroke-derived
default (unlike `Archive`'s "the current selection"), so forcing them
through the registry would mean inventing a settings context and an
account-picker for no discoverability most users will ever use one account
to need.

**The precedent already in this codebase is `SavedSearchAction`** (#292):
sidebar context-menu commands — rename, reorder, delete a saved search —
live in a `gio::SimpleActionGroup` scoped to the widget, addressed by name
(`savedsearch.rename`, `savedsearch.delete`), never registered as
`CommandId`s. That gap was named rather than silently accepted: #455 tracks
giving saved searches a keyboard path. The same shape applies here —
`account.toggle-enabled(id)`, `account.remove(id)`,
`account.update-credential(id)` as a `gio::SimpleActionGroup` on the
settings panel, mouse- and Tab-reachable within it but not in the palette
or cheat sheet — and the same follow-up discipline: filed as
[#471](https://github.com/dlapiduz/postio/issues/471) rather than expanding
this issue to design a settings keyboard context it does not need yet.

- **Enable/disable** — the toggle writes `accounts.enabled`, per Q6.
  `postio-session::engine` only reads it at startup
  (`engine.rs:197`); there is no live engine attach/detach today (that is
  #64/ADR 0012 Q2's problem, not this one's), so disabling takes effect on
  the next launch. The panel says so.
- **Remove** — per Q6's semantics and its undo, implemented as
  `AccountRepository::mark_pending_deletion`/`restore`/
  `reap_pending_deletions`. Marking is instant and reversible from the
  undo toast; the actual `DELETE ... CASCADE` only runs once, at the next
  startup, before any engine is created — the same restart boundary
  disabling already has to live with, for the same reason (no live
  account teardown yet), and also what makes a crash before the toast
  expires harmless rather than a race against a live cascade. The keyring
  entry is untouched either way, exactly as Q6 says: deleting it is a
  separate, still-unbuilt, confirmed step of its own, not something this
  removal does on the account's behalf.
- **Update credential** — the one truly new piece. **Reuses
  `Status::Reauthenticate`, which already exists and is already tested**
  (`postio-app/src/onboarding.rs`, `crates/postio-app/tests/app_suite/
  startup_repair.rs`) — today reachable only automatically, when
  `startup_route` finds a row the keyring will not give up a password for.
  This adds a manual entry point that opens the same `Onboarding` widget
  the way [ADR 0012](0012-add-account-and-orientation.md) Q1 already
  decided any second onboarding surface should be hosted — **an
  `AdwDialog` over the shell, not a window-content replacement** — seeded
  with `Status::Reauthenticate(configured(account))` instead of the empty
  first-run state. The form, its probe timing and its seam stay exactly as
  built; only what starts it and where it appears are new.

**Why this does not collide with #64.** #64's hard problem is a *second
engine* joining a running application without a restart — genuinely new
composition-root work. Reauthenticating an account already running needs no
new engine: same account, same connection, only the credential changes. The
dialog host these two share is ADR 0012 Q1's decision, already made and
already the right shape for both; building it once for the narrower case
here does not decide anything #64 still has to.

### Q6b — `config.toml` stops describing accounts (#470)

**Decided 2026-08-28.** Q6a named the gap and explicitly declined to close it:
either a reconciler that writes `[accounts.<id>]` changes into the SQLite row,
or retiring the TOML schema outright. **Retire it.**

Three things in the tree decide this, and none of them is visible from the
schema alone.

**The panel already has two account UIs stacked on one screen.** #464 put a
real `accounts_list` of widget rows — enable/disable, update credential,
remove — directly above the nav-and-text body in `settings.rs`, and #151 added
an egress list beside it. Below them, the `[accounts]` nav item scrolls the
raw TOML to a section that parses, validates, round-trips and reaches nothing.
This is not a missing feature. It is a duplicate of a working one, and the
duplicate is the one that silently does nothing.

**The schema is stale, not merely unwired.** `postio-config`'s `AuthMethod` is
`Plain | Login`. The model's `AuthMethod` — the one every real code path uses
— is `Password | AppPassword | OAuth2 | XOAuth2`. `MailSecurity` is a second
spelling of `TransportSecurity`. The config crate's copies are used by nothing
outside their own tests, and they cannot express an OAuth account at all, which
[ADR 0006](0006-oauth-and-provider-presets.md) put in scope. A user editing
`[accounts.personal].auth` today is choosing between two mechanisms, neither of
which is what their account uses. Wiring that up would mean first rebuilding
it to say what the model says — at which point it is a second spelling of the
domain vocabulary, kept in sync by hand, which is the thing the one-registry
rule exists to prevent.

**An account is state, not preference.** Every other section of `config.toml`
solely owns what it describes: `[ui]`, `[keys]`, `[sync]`, `[filters]`,
`[mailboxes]`, `[logging]`, `[storage]`, `[compose]`. An account owns
mailboxes, messages, threads, sync state and an operation queue by
`ON DELETE CASCADE`; it is created by a flow that probes servers and writes a
keyring entry; its identity is a database id, not a TOML key. A reconciler
would be a second writer to that row, racing sync, and would have to answer —
in a raw text editor whose only validation is that the file parses — what
renaming a key means, what deleting a section means, and whether typing a new
one creates an account with no probe and no credential. Every one of those has
a wrong answer that loses mail or manufactures a broken account.

What retiring means, concretely:

- `Config::accounts`, `AccountConfig`, `ImapConfig`, `SmtpConfig`,
  `MailSecurity`, `AuthMethod` and `ConfigChanged::accounts` come out of
  `postio-config`.
- An existing file's `[accounts.*]` table lands in `Config::extra` and
  round-trips verbatim — nothing is rewritten under the user, which is the
  rule for a file they own. **But it must not look live.** The footer's
  validity line is where the panel already reports things it parsed and did
  not honour (the keymap problems line), so it says so there:
  *"[accounts] is ignored — accounts are managed in the list above"*.
  A preserved section that says nothing is the bug this issue reports, wearing
  a different hat.
- The `[accounts]` nav item **stays** and moves focus to the account list
  instead of scrolling the text. The nav is the panel's table of contents; a
  person who clicks it wants accounts, and should land where accounts are.
  Canvas 3f's five sections survive; only what the fifth one points at changes.

**This does not make accounts less editable — it makes the editable thing the
only one.** Host, port and security remain unreachable after onboarding, which
is exactly where Q6a left them and is a separate gap; the difference is that
it now *looks* unreachable instead of looking editable and lying. Whoever
closes that gap should do it in the account row, beside the three affordances
Q6a already put there, not in a text file.

### Q6c — The account list is a keyboard context, and it is not the settings panel (#471)

**Decided 2026-08-28.** Q6a put three affordances on each account row as a
`gio::SimpleActionGroup` and filed #471 for the keyboard path it deliberately
did not build, naming the open question as *"does the settings panel become a
real `Context`?"*. **A context, yes — but `Context::Accounts`, not
`Context::Settings`, and the difference is the decision.**

**The precedent is #455, and it is exact.** Saved searches had the same shape:
widget-scoped context-menu verbs (`savedsearch.rename` et al.), no keyboard
path, no registry entries, and the same worry about whether registering them
needed a new navigation model. b05192f resolved it by making the rows
reachable from the keyboard and the verbs real `CommandId`s scoped to a
context that already existed. Nothing here needs a different answer; the only
reason a context has to be *added* is that no existing one covers this widget.

**Why the widget and not the panel.** The settings panel is a nav, an account
list, an egress list and a `GtkTextView` holding the literal `config.toml`. A
`Context::Settings` spanning all of that would put bare-letter bindings live
while the user is typing TOML — `d` removing an account instead of inserting a
`d`. That is not a bug to be careful about; it is a bug the name invites,
because the next person to add a binding will scope it to the context whose
name says "the settings panel". `Context::Accounts` covers exactly the
`accounts_list` widget, the way `Sidebar` and `Parts` are named for theirs, and
the mechanism follows from the name: an `EventControllerFocus` on the list
flips the context on enter and restores the previous on leave, which is
verbatim what `window.rs` already does for `Context::Sidebar`. The `TextView`
never enters it, so the trap is closed by construction rather than by
remembering.

**There is no account-picker problem, and that is Q6a's second open question
dissolved rather than answered.** #471 asked what these commands do in the
palette with no natural target, pointing at `Move` and `AddLabel`, which open a
picker when invoked with `None`. They need one because they are invoked from a
list of mail, where the target is not an account. These are only ever offered
while `Context::Accounts` is active, which means an account row has focus,
which means the target is that row — exactly as `Archive`'s target is the
current selection. Building a picker so that `Ctrl+K → Remove account` works
from the message list would be a second way to do a thing whose first way is
"open settings, land on the account, press the key", and it would put an
account-scale deletion one fuzzy match away from somebody who was not looking
at accounts.

The three commands:

| Command | Binding | Destructive | Recovery |
|---|---|---|---|
| `ToggleAccountEnabled` | `Return` | no | `None` — pressing it again is the reversal |
| `RemoveAccount` | `d` | **yes** | `Undo` |
| `UpdateCredential` | `c` | no | `None` |

`d` matches `DeleteSavedSearch`'s spelling in `Context::Sidebar`, which is the
neighbouring list and the same verb.

> **Corrected while implementing (#471).** This originally gave
> `UpdateCredential` no default binding, reasoning that "ten commands already
> have none". None do: it would have been the only entry in the registry
> without one, and `PRODUCT.md` §8 makes a shortcut a structural requirement
> that `every_command_has_an_id_a_title_and_a_default_binding` enforces. The
> reasoning behind the exemption — that it opens a dialog, happens once every
> few months, and wants discoverability rather than muscle memory — argues for
> the palette entry it has either way, not against a binding. So it takes `c`,
> for credential. Nothing is shadowed: `Compose`'s `c` is scoped to the
> message surfaces, and this context layers over `Global` alone. Getting into the list needs no new key either: the account rows
sit above the text in the focus chain, so `Tab` already reaches them.

**`Recovery::Undo` on `RemoveAccount` has to become true.** Q6a built the
removal as soft-delete plus a narrower toast wired straight to
`AccountRepository::restore`, explicitly *not* through the global undo stack,
and said so because `Remove` was not a command then. Registering it makes the
recovery a declaration the registry enforces, and a declaration nothing backs
from the keyboard is the thing this whole ADR keeps refusing to ship. So while
that toast is up, `u` in `Context::Accounts` invokes it. Context-local state,
context-local binding, no change to the global stack — which is what a context
is for. After the next start the reap has run and there is nothing to undo,
which is exactly why Q6a put the reap at a restart boundary.

`Context::ALL` gains `Accounts` at the end, so the `?` sheet grows a section
rather than reordering the ones people have learned.

---

## Q7 — Which Pimalaya libraries do the work

Worth stating explicitly, because "use the Pimalaya stack" is not a decision
until it names crates, and because a survey of this found one thing already
right and one opportunity not taken.

**In the graph today, and staying:**

| Crate | Role | Notes |
|---|---|---|
| `io-imap` `=0.6.0` | IMAP, behind `MailBackend` | Pinned (ADR 0001). Covers RFC 3501, plus 2177 (IDLE), 4315 (**UIDPLUS**), 5161 (ENABLE), 5256 (SORT/THREAD), 6851 (MOVE), 7628/7677 (OAUTHBEARER, SCRAM) |
| `io-smtp` `=0.3.0` | Submission | |
| `io-sasl` `=0.1.0` | Auth mechanisms | Where `XOAUTH2`/`OAUTHBEARER` land for [ADR 0006](0006-oauth-and-provider-presets.md) |
| `io-pim-discovery` `0.7` | Autoconfig + SRV | Features `autoconfig`, `rfc6186` |
| `pimalaya-stream` `0.3` | TLS transport | `rustls-ring` |

**A correction worth recording**, because it is the sort of thing a reviewer
assumes and gets wrong: Postio does **not** reimplement autoconfig.
`postio-imap/src/discovery/` uses `io-pim-discovery` for steps 2–5 of its
chain and adds only what that crate deliberately leaves to a caller — probe
*order*, per-step and whole-probe budgets, cancellation, the built-in preset
table, and the mapping to something an onboarding screen can render. That
division is correct and should not be "simplified" by pushing policy down into
the library.

**Not adopted, and why:**

- **`io-proxy`** — a proxy client. Postio makes no outbound connection the user
  did not ask for (`ARCHITECTURE.md` §11), and a proxy is configuration for a
  network Postio does not manage. When somebody actually needs it, it is a
  transport-level addition and nothing above changes.
- **`io-http`** — already present transitively under `io-pim-discovery`. Postio
  should not acquire it directly: a direct HTTP client in the graph is an
  invitation to make a request that is not a mail protocol, which is exactly
  what §11 exists to prevent. OAuth's token exchange (ADR 0006) is the one
  legitimate case, and it belongs behind the same discovery/transport seam.

**The opportunity not yet taken, and it bears directly on OAuth:**
`io-pim-discovery` also ships `rfc8414` (OAuth 2.0 authorization-server
metadata) and `rfc9728` (protected-resource metadata), plus `rfc8620` (the
JMAP session resource) and `rfc6764` (CalDAV/CardDAV).

[ADR 0006](0006-oauth-and-provider-presets.md) assumed a provider's
`authorize`/`token` endpoints and scopes come from the preset row. For any
provider publishing RFC 8414 metadata they can be **discovered instead**, which
turns three hand-maintained fields per provider into zero. The preset table
stays — it is the offline and no-metadata path, and it still carries the
client id — but it stops being the only source. That is a change to ADR 0006's
Q4 worth making before its preset schema is written, and it is why this section
lives in an ADR about accounts: it is the *account type* that decides which
discovery mechanism applies.

> **Settled:** ADR 0006 Q4 was amended as asked (2026-08-24, #152) — metadata
> discovery is now a first-class endpoint source, and the hand-carried fields
> are the offline path.

`rfc8620` is likewise the reason `MailBackend` should stay protocol-agnostic:
JMAP discovery is already available in a crate Postio depends on, so the day
JMAP is wanted the missing piece is a backend, not a discovery story.

---

## Q8 — Onboarding, once there is more than one kind of account

[ADR 0012](0012-add-account-and-orientation.md) settled *where* the add-account
form lives — one form, two hosts, with `attach_account` joining a running app.
What it did not settle is that accounts are not all the same shape, and the
one-screen flow only stays one screen if the differences are **discovered
rather than asked about**.

**The screen never asks "what kind of account is this?"** That is the
provider's question to answer, and the probe already answers it: the chain in
Q7 returns `AccountSettings` carrying `requires_app_password`, a `note` and a
`password_help_url`. So three of the four types are the same gesture — type an
address, wait, confirm:

| Type | What changes on screen | Where the answer comes from |
|---|---|---|
| Password | Nothing | Probe result |
| App-specific password | The field's label, a sentence, and a link to generate one | `requires_app_password` + `note` — **already built** |
| OAuth 2 | The password field is **replaced** by a *Sign in* button opening the system browser | Preset row's auth list, or RFC 8414 metadata (Q7) |
| Manual | The form expands to host/port/security | `DiscoveryOutcome::ManualEntry` |

**The OAuth branch changes the screen's shape, and it is worth being explicit
that it removes a field rather than adding a step**: there is no password to
type, so the screen gets simpler, not longer. The waiting moves from the probe
to a browser round-trip, which is why [ADR 0006](0006-oauth-and-provider-presets.md)
makes cancellation first-class — closing the dialogue must not leave a loopback
listener bound.

**What multi-account adds:**

- **The second account is onboarded by the same code against a running app.**
  That is ADR 0012's `attach_account`, and it is why that issue depends on this
  one rather than the reverse.
- **A duplicate address is refused with a sentence at the form.** The schema's
  uniqueness is per-address-per-*account* and will not catch it, so nothing
  below the form will.
- **The first account is not special.** Nothing may key off "account 1".
  `first_account()` disappearing is what makes the second account work, and any
  code that treats the first differently fails exactly once — in the field, for
  a user who deleted their original account.
- **A failed probe never blocks adding an account.** Manual entry stays
  reachable, because a provider Postio has never heard of is precisely the case
  the preset table exists to not be required for.

---

## Q9 — Moving a message between accounts

**The problem the first version of this ADR skipped.** It is the only operation
in Postio that can lose mail.

IMAP `MOVE` (RFC 6851) works within one server. Between accounts there is no
server-side operation at all: the message must be uploaded to the target and
removed from the source, over **two connections, in two per-account queues**
(`operation_queue.account_id` is `NOT NULL` and `idx_operation_queue_drain` is
keyed on it), with no transaction spanning them.

Two orderings, both wrong:

- **Delete then append** — the source is gone and the append fails. **Mail
  lost.** Unacceptable at any probability.
- **Append then delete** — the append succeeds, the delete fails, the user has
  two copies. Recoverable, visible, annoying.

**Decision: a three-phase saga, ordered so the only failure mode is a
duplicate, and structured so even that is usually avoided.**

```
  1. COPY    fetch the raw message from A's blob store (or from A's server
             if the body is not local yet) and APPEND it to B.
             Record the returned UidMapping.            ── B's queue
  2. CONFIRM the message is present in B: the APPENDUID from RFC 4315, or,
             where the server has no UIDPLUS, a targeted search for the
             Message-ID in the target mailbox.
  3. REMOVE  only now, store \Deleted + EXPUNGE on A.   ── A's queue
```

**What makes this tractable is that `MailBackend::append` already returns
`Option<UidMapping>`** — `Some` exactly when the server speaks UIDPLUS, which
`io-imap` implements (RFC 4315). That is a *proof of arrival*, not an
assumption, and it is what phase 3 waits for.

**Where the saga lives.** Not in either account's queue, because it belongs to
neither. A `cross_account_moves` table holds the saga's own state — source
message, target mailbox, phase, the confirmed target UID — and the two queue
operations reference it. The drainer for A refuses to run the REMOVE until the
row says CONFIRMED. That is what replaces the transaction the two queues cannot
share.

**Design constraints that fall out, each of which is a test:**

- **A saga is resumable, because a restart mid-move is normal.** The phase is
  on disk before either side is touched.
- **Phase 1 is idempotent by Message-ID.** Re-running an APPEND after a crash
  must not create a second copy: confirm-before-append, using the same lookup
  phase 2 uses.
- **No UIDPLUS is not a blocker, it is a slower path.** Fall back to searching
  the target for the `Message-ID`. If even that cannot confirm, the move
  **stops at phase 2 and asks** — it does not guess and it does not delete.
- **The user sees it as one action.** Local-first still applies: the message
  appears in B and disappears from A immediately, and the saga reconciles.
  Undo is the inverse saga, which is why the confirmed target UID is recorded.
- **A failed move never leaves the message only in flight.** If the saga
  aborts, the source copy is still there, because nothing deleted it.

**The cheap case is still cheap.** A move *within* one account stays a single
`MOVE` on one queue. Nothing above applies to it, and the UI must not make the
common case pay for the rare one.

**Drag-and-drop makes this reachable by accident**, which is the strongest
argument for the ordering above: dropping an inbox row onto another account's
folder is one gesture, and the user will not have thought about atomicity.

---

## Q10 — When one account is broken and the others are fine

**The state the first version did not design.** It is also the normal state:
one expired password, one flaky server, one laptop that woke on a captive
portal.

### The connection model has to say *why*

`ConnectionState` today is `Offline | Connecting | Online | Failing`
(`core/src/event.rs:23`). `Failing` cannot distinguish a rejected password from
a DNS failure, so no UI built on it can offer the one action that resolves the
most common case — **reauthorise this account**.

**Decision: `Failing` carries a reason**, and the reasons are the ones that
imply different user actions rather than different error text:

| Reason | What the user can do | Retry? |
|---|---|---|
| `Auth` | Re-enter the password, or re-run the OAuth flow | **No.** Retrying a rejected credential is how an account gets locked |
| `Network` | Nothing; it will recover | Yes, with the existing backoff |
| `Server` (5xx, `TRYCREATE`, over quota) | Usually nothing; sometimes free space | Yes, slower |
| `Config` (host wrong, TLS refused) | Fix the settings | No |

The distinction that matters most is **`Auth` must not be retried on a timer**.
`postio-sync`'s `Attention` already exists as the "stop and ask a human" state;
this is what routes into it correctly.

### An aggregate view must never lie by omission

This is the rule, and it applies to the unified inbox, unified search, and any
future aggregate:

> **A view that cannot include an account says so, names the account, and
> stays usable.**

Concretely:

- The unified list shows what it has, with a persistent, non-modal line
  naming the account. Not a toast, which disappears; not a modal, which
  blocks; not silence, which is the current design's default and the one that
  is actually dangerous.

  **Amended by Q10a (#187):** this bullet originally gave the line as
  *"Personal is offline — showing 1 of 2 accounts."* That sentence is false
  in this design. Postio is local-first, so an offline account's synced mail
  is still in the unified list: the account is not *excluded*, it is
  *unrefreshed*, and "showing 1 of 2" claims an omission that has not
  happened whenever the absent account has mail on disk — which is almost
  always. The rule above is unchanged and the account is still named; what
  the line says about it is that its mail is what was already synced, in the
  vocabulary canvas 3d settled on ("Offline — reading local mail"). A count
  of accounts would be the right sentence only for a view that genuinely
  dropped one, and this view never does.
- **Counts are marked partial.** An unread count that silently excludes an
  account is worse than no count, because it looks authoritative.
- **Search results carry the same marker.** A user who searches for an invoice,
  finds nothing, and concludes it does not exist has been misled by a view that
  looked complete. This is the single most important instance of the rule.
- **`Selection::Everything { except }` is scoped to what is loaded.** "Select
  all" in a degraded unified view must not silently act on an account whose
  state Postio cannot currently see.

### One account's failure must not spend another's budget

- **No global sync lock** (unchanged from the first version) — one unreachable
  server must not stall the others.
- **Backoff is per account.** A failing account's retries must not consume the
  connection pool slots a healthy account needs, which is the practical form of
  the pool sizing rule in Q3.
- **A disabled account is not a failing one.** `accounts.enabled = 0` stops the
  engine and drops out of Unified silently and correctly, because the user
  asked for that.

---

## Q11 — The event vocabulary needs an account

Found while revising, and it is a contract-level gap rather than a UI one.

`Event` (`core/src/event.rs`) carries an account on `MailboxesChanged` and on
the connection events, and **not** on the ones that matter most for an
aggregated view:

```rust
MessagesChanged { messages: Vec<MessageId> },      // no account
MessagesRemoved { mailbox: MailboxId, messages },  // account only via a lookup
MessageListChanged { mailbox: MailboxId },         // same
```

With one account this is fine — everything belongs to the only account there
is. With several, a frontend maintaining a unified list has to resolve every id
to an account before it can decide whether a repaint concerns the view on
screen, and a `MessageId` alone cannot be resolved without touching the store,
which is the thing the event exists to avoid.

**Decision: every `Event` variant that names data names its account.** It is a
mechanical change, it is caught by the compiler at every construction site, and
it should land *before* the frontend work rather than as a fix afterwards —
adding it later means auditing every subscriber for the assumption that there
was only ever one account.

---

## Q12 — Filters, folders, and what "tied to search" actually implies

`ARCHITECTURE.md` §6 says there is one matching language and that a filter is a
saved query. Multi-account is where that promise is tested, because the two
things in the sidebar look alike and behave differently — and Q4 already drew
that line for mailboxes. Filters land on the other side of it.

**A filter is a query, so a filter is account-agnostic by construction.** It
runs in whatever scope the user is in: `is:unread from:team` pinned in the
sidebar means "across everything" in Unified and "in this account" in an
account scope. That falls out of Q5's decision to make scope a filter on the
executor rather than part of the query language, and it is the behaviour a user
expects from something that lives beside folders but is not one.

**When a filter must be pinned to one account, the query says so**, using the
`account:` field from Q5 — `account:work is:unread`. One language, one parser,
and a saved search that means the same thing wherever it is evaluated.

**Folders remain per-account and real.** A folder has a `UIDVALIDITY`, mail
physically lives in it, and `a` archives *into* one. There is no unified
"Archive" folder, because archiving in Unified scope archives each message into
**its own account's** Archive (Q4). The sidebar must not draw those two kinds
of thing so alike that a user expects to drag mail into a filter.

**Rules ([ADR 0008](0008-filters-and-rules.md)) are per-account in execution
and global in definition.** `[[rules]]` is one ordered list; each rule is
evaluated against each account's arriving mail, in the same order, by that
account's sync pass. Two consequences worth writing down:

- A rule whose action names a mailbox (`move:Receipts`) resolves that name
  **within the account the message arrived at**. If an account has no such
  folder, the rule fails for that account only, raises `Attention`, and leaves
  the message where it is (ADR 0008 Q6) — it must not move mail into a
  different account's folder because the name matched there.
- A rule can be scoped with `account:` in its query like any saved search, so
  "only for work" needs no new configuration concept.

---

## Q13 — The rest of it

Smaller, and each is a real decision rather than a note.

- **Which identity replies.** A reply uses the identity of the account the
  message arrived at, never the default. Getting this wrong sends from the
  wrong address and is only discovered after it is sent, which is why the
  composer shows the resolved identity rather than assuming it.
- **Notifications name the account** when more than one is configured, and are
  configurable per account — `spec`-level behaviour that becomes wrong the
  moment a second account exists.
- **Duplicate suppression is display-only.** A message received at two
  addresses is two rows in two accounts (Q2). The unified list shows one; every
  count, every action, and every export still knows there are two.
- **Removal with work in flight.** Deleting an account with pending queue rows
  must drain or discard them deliberately — the `ON DELETE CASCADE` in the
  schema will remove the queue rows silently, which is right for local state
  and wrong for a half-finished cross-account move (Q9). A move saga naming a
  removed account aborts and leaves the source intact.
- **Undo spans accounts.** A bulk archive in Unified touched several accounts;
  `u` must reverse all of them, which the existing `UndoStack` handles because
  an entry carries `Command`s rather than server state. A partial undo, where
  one account is offline, must undo what it can and say what it could not.
- **Per-account quota and rate limits.** Providers differ, and a shared drainer
  that treats all accounts identically will trip the strictest one. Backoff is
  already per account; the connection budget in Q3 has to be too.

---

## Alternatives

**A database per account.** Rejected in Q1: forks the FTS5 index and makes
cross-account search unrankable.

**Per-account `UNION ALL` for unified search.** Rejected in Q5a on
measurement: slower than the unified index it was meant to avoid (32.4 ms
against 18.8 ms), still needs a temp b-tree for the merge, costs a walk per
account to fill one page, and adds a third scoring surface to keep coherent
with `rank_score`. It would also have forced unified search to be a loop, and
so forced #186's scope enum into a worse shape.

**Threads that span accounts.** Rejected in Q2: it makes thread identity depend
on account membership, costs the list its index, and requires splitting threads
on account removal — to buy something read-time grouping already gives.

**Unified as a synthetic mailbox row.** Attractive because the sidebar and the
list would need no new concept, and wrong for the reason `ARCHITECTURE.md` §6
gives: `move` and `UIDVALIDITY` would then have to mean something for a folder
mail cannot live in.

**One sync engine multiplexing all accounts.** Fewer threads, one connection
pool, and one slow server stalling everyone. The engine is already a thread
that owns a connection because `rusqlite::Connection` is `!Sync`; N of them is
the cheap path, not the expensive one.

**Cross-account move as a plain copy, leaving the source alone.** Honest, never
loses mail, and not what the user asked for — they said *move*. A client that
quietly turns a move into a copy trains people not to trust it. The saga in Q9
costs one table and buys the operation actually requested.

**Cross-account move as delete-then-append**, which is what a naive "remove
from A, add to B" writes. Rejected in Q9: it loses mail on any failure between
the two steps, and no probability of that is low enough to accept.

**Letting an aggregate view fail silently when one account is down.** The path
of least code, and the one that turns an empty search result into a lie.
Rejected in Q10 — it is the difference between a client that is degraded and
one that is wrong.

---

## Consequences

- `first_account()` disappears. `app/src/lib.rs` starts an engine per enabled
  account and sizes the pool from the count.
- `AppState.scope` is a breaking change inside `postio-core`, caught by the
  compiler everywhere it matters.
- `postio-search` gains one `Field`; `docs/keybindings.md` and the config
  reference regenerate themselves (`ARCHITECTURE.md` §2).
- **A migration adding `idx_messages_recency ON messages (received_at DESC,
  id DESC)`** (Q5a) — landed as migration 0012 with Q5b. 2.2 MiB per 120,000 messages, no measurable insert cost,
  and without it unified recency-ordered search is 20 seconds rather than 19
  milliseconds. It belongs in the same change that teaches the executor to
  drop the `account_id` predicate, or an earlier one — never a later one, and
  there is no reason to land it on its own ahead of #186, where it would cost
  writes and disk to speed up a query nothing can yet ask.
- **`search_budget.rs` needs a multi-account corpus** (Q5a). Today it cannot
  distinguish a unified search from an account-scoped one, so it would stay
  green through the regression the index exists to prevent.
- The `Move` command becomes context-conditional on scope, which is the first
  time a command's availability depends on state rather than on `Context`.
  That is a registry question, not a frontend one — resolve it as a predicate
  the registry can evaluate, or the palette will offer a command that cannot run.
- **`Event` variants gain an account** (Q11). Mechanical and compiler-checked,
  and far cheaper before the frontend work than after.
- **`ConnectionState::Failing` gains a reason** (Q10), and `Auth` stops being
  retried on a timer.
- **A `cross_account_moves` table and a saga drainer** (Q9). The only genuinely
  new subsystem here, and the only place mail can be lost — so it is where the
  tests concentrate: a crash between every pair of phases, the no-UIDPLUS
  fallback, a target folder that has gone, an account removed mid-saga.
- **Aggregate views need a "partial" state to render** (Q10): the list, search
  results, and every count.
- [ADR 0006](0006-oauth-and-provider-presets.md) Q4 is worth revisiting before
  its preset schema is written — RFC 8414 metadata can supply the OAuth
  endpoints the preset row was going to hand-carry (Q7). *Done: Q4 was amended
  2026-08-24 (#152).*
