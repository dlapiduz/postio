# ADR 0016 — Full-mailbox backfill by default, folders optionally excluded

- **Status:** Accepted (2026-08-25). Built: backfill continues to the whole
  mailbox, and a folder can be excluded
- **Date:** 2026-08-25
- **Decision by:** the maintainer, directly, in response to #318
- **Issue:** [#318 Backfill is seeded once, 200 per folder, and never
  again](https://github.com/dlapiduz/postio/issues/318)
- **Related:** [#316](https://github.com/dlapiduz/postio/issues/316) (the
  status line's honesty), [#74](https://github.com/dlapiduz/postio/issues/74)
  (backfill visibility), `PRODUCT.md` §6 (what is stored locally), §7
  (search), §14/§15 (sync, offline), §18 (never load a whole mailbox into
  memory)
- **Decision:** **every selectable folder backfills to completion by
  default — every message's body, eventually, in the background, throttled
  by the backfill policy.** A folder can be excluded from backfill
  explicitly; nothing is excluded unless the user says so. There is no
  horizon: it is *the whole mailbox*.

---

## The question this settles

How far back Postio pulls bodies unprompted (#318). **There is no horizon.**
Every message in a folder Postio backfills gets its body, not the newest N;
the queue is topped up in batches until the folder is done. "Eventually" is
doing real work in that sentence — this is a background-lane, throttled,
resumable process, not a promise about how fast a 40,000-message account
catches up.

## Why the whole mailbox, not a cap

Two of the product's own promises assume it:

- **§15: "Fully usable offline after the first sync."** A message whose
  body was never pulled is not usable offline — it is a placeholder that
  turns into a network request the moment someone opens it, exactly #318's
  symptom ("every older message pays a round trip when it is opened").
- **§7: "Search is a defining feature and a primary way to navigate."**
  The body index — a `USING fts` index written by
  `postio_session::spawn_body_indexer` off the sync lane — indexes what is
  locally parsed. A body that never arrived is a body that can never match a search
  term. A 200-per-folder cap does not mean
  "search is slightly less complete" — it means search silently stops
  covering a mailbox's own history past whatever arrived in the first
  20-odd minutes of first sync, which is the opposite of "a primary way to
  navigate."

A client that only ever backfills its own most recent few hundred messages
per folder is a client whose search and whose offline promise both quietly
degrade the moment an account is more than a few months old. That is not
the product this repository is building.

## What "download everything" does not mean

**It does not mean loading a mailbox into memory.** `PRODUCT.md` §18's
constraint — *"a mailbox is never loaded into memory"* — is about the
message *list*, which stays windowed over the paged store regardless of how
much is on disk. Backfill is a disk-and-index axis; the list's memory
budget is a separate axis that this decision does not touch. A fully
backfilled 100,000-message account and a freshly-added one both render the
same windowed list at the same budget — that is the property `postio-storage`
and `postio-index` already hold, and nothing here asks them to hold
anything more.

**It does not mean unconditionally, regardless of cost.** `BackfillPolicy`
(`postio-sync::backfill`) does the right things:
`max_body_bytes` (5 MB default) skips the outlier attachment nobody may ever
open, `pause_on_metered` and `pause_when_active` keep the background lane
out of a data plan and out of the user's way, and `background: false` — the
existing `[sync] body_fetch` config knob — turns the whole lane off for
someone who wants lazy-only, on-open fetching and nothing more. "Download
everything" describes the *target*, not a removal of the
throttles that get there responsibly.

## Folders can be excluded

Not every folder is worth backfilling by default forever — a shared
mailing-list archive folder with forty thousand messages nobody reads twice
is a real case, and so is a `Junk` folder whose contents are, definitionally,
not worth keeping locally in full. So:

- **Default: on.** Every selectable folder backfills to completion unless
  told otherwise. The default is not "ask the user during onboarding" — that
  would be a step ADR 0012 already decided against adding to the one-screen
  flow — it is simply *on*, discoverable and reversible from Settings.
- **Opt-out is per folder, explicit, and reversible.** Turning backfill off
  for a folder does not touch what has already been pulled and does not
  stop interactive, on-open fetches — the same distinction
  `BackfillPolicy::background` draws for the account-wide knob, scoped
  narrower.
- **Where it lives:** `mailboxes.backfill_excluded`. `backfill::seed` queues
  nothing for an excluded folder; `request_body` and `request_whole` still
  answer an on-open fetch. In Focus it is Settings' Sync & storage, "Back up
  locally", a check per folder.

## Consequences

- `docs/PRODUCT.md` §14: backfill continues until every selectable,
  non-excluded folder is fully local, not a fixed initial pull.
- `BackfillPolicy`'s `max_body_bytes`, `pause_on_metered`,
  `pause_when_active` and `background` are the pacing; the target is not
  theirs to change.

## What would falsify this

If backfilling a real large account (this project already has one on hand —
engineering-notes.md's 81,716-message account) turns out to make the
background lane starve interactive fetches or the sync engine's own
housekeeping under the concurrency rules (`MAX_CONCURRENT_PASSES`, the
`WriteGate` in `crates/postio-storage/src/store.rs`, and the sync-lanes
constraints), that is a
throttling-policy bug to fix in `BackfillPolicy`, not a reason to reintroduce
a horizon. The target stays "everything"; only the pacing is up for
adjustment.
