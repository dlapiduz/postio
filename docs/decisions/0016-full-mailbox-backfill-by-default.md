# ADR 0016 — Full-mailbox backfill by default, folders optionally excluded

- **Status:** Accepted — **GO** (2026-08-25)
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
  by the policy that already exists.** A folder can be excluded from
  backfill explicitly; nothing is excluded unless the user says so. This
  answers #318's own "deliberately not `ready`: the horizon is the
  maintainer's call" — the horizon is *the whole mailbox*.

---

## The question this settles

#318 found that `postio-app/src/lib.rs` seeds the backfill queue once, at
startup, with the newest 200 messages per selectable folder, and nothing
ever tops it up. The cap itself was a reasonable guess — the comment at the
call site says so, in terms of not downloading a 40,000-message archive
unprompted — but nobody had actually decided what the *steady state* should
be, and #318 said so explicitly rather than guessing: *"How far back Postio
should pull bodies unprompted is a real decision... the horizon is the
maintainer's call."*

**The answer: there is no horizon.** Every message in a folder Postio
backfills gets its body, not the newest N. "Eventually" is doing real work
in that sentence — this is a background-lane, throttled, resumable process,
not a promise about how fast a 40,000-message account catches up.

## Why the whole mailbox, not a cap

Two things in the product's own promises already assume this and were
quietly relying on nobody noticing the gap:

- **§15: "Fully usable offline after the first sync."** A message whose
  body was never pulled is not usable offline — it is a placeholder that
  turns into a network request the moment someone opens it, exactly #318's
  symptom ("every older message pays a round trip when it is opened").
- **§7: "Search is a defining feature and a primary way to navigate."**
  FTS5 indexes what is locally parsed. A body that never arrived is a body
  that can never match a search term. A 200-per-folder cap does not mean
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
message *list*, which stays windowed over paged SQLite regardless of how
much is on disk. Backfill is a disk-and-index axis; the list's memory
budget is a separate axis that this decision does not touch. A fully
backfilled 100,000-message account and a freshly-added one both render the
same windowed list at the same budget — that is the property `postio-storage`
and `postio-index` already hold, and nothing here asks them to hold
anything more.

**It does not mean unconditionally, regardless of cost.** `BackfillPolicy`
already exists (`postio-sync::backfill`) and already does the right things:
`max_body_bytes` (5 MB default) skips the outlier attachment nobody may ever
open, `pause_on_metered` and `pause_when_active` keep the background lane
out of a data plan and out of the user's way, and `background: false` — the
existing `[sync] body_fetch` config knob — turns the whole lane off for
someone who wants lazy-only, on-open fetching and nothing more. None of that
changes. "Download everything" describes the *target*, not a removal of the
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
  stop interactive, on-open fetches — exactly the same distinction
  `BackfillPolicy::background`'s doc comment already draws for the
  account-wide knob. This is that same knob, scoped narrower.
- **Where it lives is implementation's call, not this ADR's.** The natural
  shape is a column on `mailboxes` (a per-folder analogue to
  `accounts.enabled`) surfaced in the folder's own settings, but the exact
  schema and surface belong to whoever builds it, not to this decision.

## Consequences

- **#318's scope is now decided rather than open.** Its acceptance criteria
  stand; "the horizon" in its own text is answered here. It should move to
  `ready`.
- **A new issue is needed for the exclusion mechanism** — a per-folder
  backfill toggle, its storage, and its Settings surface. Not part of #318,
  which is about making the *existing* one-shot cap into a continuing
  process; excluding a folder from that process is separable work.
- **`docs/PRODUCT.md` §14** gains a line: backfill continues until every
  selectable, non-excluded folder is fully local, not a fixed initial pull.
- **No schema change to `BackfillPolicy` itself** — `max_body_bytes`,
  `pause_on_metered`, `pause_when_active` and `background` all already say
  the right thing and are unchanged by this decision.
- **`docs/engineering-notes.md`** should record the chosen horizon per
  #318's own acceptance criteria, once #318 lands, next to whatever
  re-seed mechanism is chosen.

## What would falsify this

If backfilling a real large account (this project already has one on hand —
engineering-notes.md's 81,716-message account) turns out to make the
background lane starve interactive fetches or the sync engine's own
housekeeping under the existing pool-sizing rules (`ARCHITECTURE.md`, the
sync-lanes constraints), that is a throttling-policy bug to fix in
`BackfillPolicy`, not a reason to reintroduce a horizon. The target stays
"everything"; only the pacing is up for adjustment.
