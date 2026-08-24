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
git log --oneline -15 --name-only | grep '^crates/' | cut -d/ -f1-2 | sort -u
bd list --status=in_progress
```

Three signals, not one:

- **Crates dirty from your edits** are yours. Dirty from someone else's are not.
- **Crates your recent commits touched** are also yours. Do not conclude your
  lane is empty just because you committed everything — a clean tree means you
  finished, not that you are done.
- **Beads you filed are yours to pick up**, wherever they live. Work you
  discovered often lands in another crate: a bug found while packaging can sit
  in `postio-imap`. Take it if that crate is free — nobody else has the context,
  and leaving it orphans the finding.

A crate is free if nothing of someone else's is dirty in it and no claimed bead
names it. Check before taking work outside the crates you have been editing;
do not check and then avoid it out of caution.

## 2. Pick

**We are finishing MVP. Take only `mvp`-labelled work.**

```bash
bd list --label mvp --status open
```

That label is the scope, and it is deliberately short — the last things between
this and a mail client the maintainer uses daily:

| | |
|---|---|
| `postio-5w1.1` | search hits reach the message list |
| `postio-agr.1` | bulk actions over a whole-mailbox selection |
| `postio-agr.2` | folder picker for move |
| `postio-uoy` | the sidebar's Flagged folder |
| `postio-v62` | the parts panel and attachment chips reach the app |
| `postio-2ee` | the cheat sheet lists the box's prefixes |
| `postio-qhz.6` | the sync status stops contradicting itself |

Everything else — parallel sync, drag and drop, the accessibility pass,
detached composer, filters, MCP — waits. They are real work and several are
P1; that is not the same as being between here and a usable product.

**If a bug you hit is genuinely in the way of an `mvp` bead, fix it and label it
`mvp` too.** If it is merely nearby, file it unlabelled and move on. The
distinction is whether the MVP item can land without it.

When no `mvp` work is left, **stop and say so** — do not fall through to
`bd ready`. Report what is done, what is left in the wider backlog, and let the
maintainer decide what comes after MVP.

<details>
<summary>After MVP ships, the general rule below applies again.</summary>

```bash
bd ready
```
</details>

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
