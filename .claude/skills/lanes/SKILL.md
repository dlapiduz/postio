---
name: lanes
description: Report which Postio crates other Claude sessions are currently working, what beads are claimed, whether any claim is stale, and therefore what is safe to pick up. Use at the start of a session, before claiming a bead, and before touching a crate outside your lane.
---

# Lanes

Several sessions work this repository at once, in the same working tree. This
answers "who else is here, and what can I safely take".

## 1. What is claimed

```bash
bd list --status=in_progress
```

Each row is a bead someone is working — or was working when their session was
cut off. Leave them alone unless you can demonstrate the work is committed.

## 2. Which crates are hot

```bash
git status --porcelain | awk '{print $2}' | cut -d/ -f1-2 | sort | uniq -c | sort -rn
```

Uncommitted files mean an active or interrupted session. A crate with dirty
files is a crate to stay out of.

## 3. Recent activity

```bash
git log --oneline -15
```

Commit scopes show which lanes have been moving. A crate committed to minutes
ago probably still has someone in it.

## 4. Stale claims

A bead `in_progress` whose work is already committed means a session died
before it could close the bead. Check each claimed bead against the log:

```bash
git log --oneline --all -- crates/<crate-the-bead-touches>
```

If the implementation exists and the tests pass, the bead can be closed. Say
which ones qualify rather than closing someone's live work.

## 5. What is actually available

```bash
bd ready
```

Ignore `[epic]` rows — they are containers, not work. A leaf task in a crate
that is clean and unclaimed is safe to take.

## 6. Report

State which crates are occupied, which beads are claimed, which claims look
stale, and name the specific beads this session can safely start. If the only
available work sits in an occupied crate, say so plainly rather than
suggesting a collision.

The crate split is deliberately disjoint so lanes do not collide:
`postio-model`, `postio-storage`, `postio-search`, `postio-config`,
`postio-imap`, `postio-smtp`, `postio-sync`, `postio-core`, `postio-gtk`. If a
bead genuinely needs a crate another session owns, note it in the bead rather
than editing across the boundary.
