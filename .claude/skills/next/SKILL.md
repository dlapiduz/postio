---
name: next
description: Pick up the next bead and keep working without waiting to be told. Finds unclaimed, unblocked work inside your lane, claims it, and continues. Run this whenever you finish a bead — do not stop and wait for instructions while there is work you can safely take.
---

# Next

**Finishing a bead is not finishing a session.** Run this and keep going. The
user is not watching, and a session that stops with work available has wasted
the rest of its context.

## 1. Know your lane

Your lane is the crates you have been working. It is not "whatever is ready".
Other sessions are live in this same working tree, and taking a bead in their
crate causes exactly the collisions the crate split exists to prevent.

```bash
git status --porcelain | awk '{print $NF}' | cut -d/ -f1-2 | sort | uniq -c | sort -rn
bd list --status=in_progress
```

Crates dirty from *your* edits are yours. Crates dirty from someone else's, or
carrying someone else's claimed bead, are not.

## 2. Pick

```bash
bd ready
```

Ignore `[epic]` rows — they are containers, not work. From the leaves, take the
first that is **in your lane** and **unclaimed**, ranked by:

1. **Priority.** A P0 outranks anything, always.
2. **What it unblocks.** `bd show <id>` lists what it blocks; a bead holding up
   six others beats one holding up none. Ignore leverage numbers pointing only
   into the post-v1 backlog (epic `postio-z3b`) — that work is deliberately
   gated and not urgent.
3. **Continuity.** A bead adjacent to what you just built is cheaper for you
   than for a cold session — you already have the context.

Then:

```bash
bd show <id>          # read the NOTES field too, not just the description
bd update <id> --claim
```

Claim **before** writing code. An unclaimed bead you are working looks
available to everyone else.

## 3. Work it

Test-first, per `CLAUDE.md`. Load `/ux-architect` before designing a surface
and `/gtk-design` before building one. When done, `/land` and close the bead.

Then run this skill again. Keep going until one of the stopping conditions
below is genuinely true.

## 4. When you find something rather than finish something

Do not stop to report. Record it and carry on:

- **A bug, or work the bead revealed** → `bd create` it, with enough detail
  that a cold session could pick it up, and link it to what you were doing.
- **A decision future work should follow** → `bd remember`, or a note on the
  bead. An invariant nobody wrote down lasts exactly as long as this session.
- **A bead that turns out to be already done** → verify against `git log`,
  then close it with the commit named.
- **A bead you cannot finish** → commit what you have as work-in-progress,
  leave the bead open with the remaining criterion spelled out in its notes,
  un-claim it, and move on. Half a bead recorded honestly is worth more than a
  session that stalled on it.

## 5. Stop only for these

- **`bd ready` has nothing in your lane.** Say which crates you own and what is
  left elsewhere, so the next lane can be assigned. Do not wander into another
  session's crate to stay busy.
- **The next bead needs a decision only the user can make** — a product choice,
  a credential, something outward-facing. File it, say so, stop.
- **Your crate is red for a reason you did not cause.** Report it rather than
  fixing someone else's in-flight work.
- **Context is nearly gone.** Land what you have first. Never end with
  uncommitted work — that is the one unrecoverable way to finish.

Anything else is not a reason to stop.
