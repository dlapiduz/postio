# The WAL is not the startup cost, and measuring it took ten minutes (2026-09-05, #1175)

The maintainer reported a five-second first run against a live install whose
store looked alarming: an 868 MB database with a **676 MB write-ahead log**
beside it, and no `wal_autocheckpoint` or `journal_size_limit` anywhere in
`postio-storage`. The inference wrote itself — a WAL that nothing checkpoints,
recovered and re-indexed under SQLCipher before the first frame — and it was
wrong.

## What the numbers say

`cp --reflink=always` of the live store (instant on btrfs, and it leaves the
original alone — opening read-write checkpoints, which destroys the state
being measured), then `Database::open` on the copy exactly as the app does,
plus one real query so the WAL index is genuinely paid for:

```
keyring round trip:              28 ms
postio.db 917.8 MB · postio.db-wal 675.6 MB

open #1 (as found):              34 ms      (108 ms cold page cache)
after wal_checkpoint(TRUNCATE):
open #2 (WAL truncated):          2 ms
open #3 (WAL truncated):          2 ms
```

The whole of `Phase::Store` — the keyring D-Bus round trip *and* opening an
868 MB encrypted database with a 676 MB WAL in front of it — is under 150 ms,
in a **debug** build, against a release install that takes five seconds.

## Two beliefs that did not survive

- **"Nothing checkpoints it."** The open checkpointed it unprompted: the WAL
  went from 675.6 MB to gone and the database grew 868 → 917.8 MB as the
  frames folded in. The default autocheckpoint works.
- **"A checkpoint cannot pass a still-open reader, so a pooled connection
  pins the WAL."** A plausible mechanism, reasoned from the pool's existence,
  needed to explain nothing, and never tested before it went into an issue as
  the likely cause.

What is true is smaller and duller: `journal_size_limit` is unset, so a
checkpointed WAL is *reused* rather than truncated and keeps its high-water
mark for the life of the process (#1187). Disk, not time.

## The instruments, and why they are in the tree

`crates/postio-runtime/examples/time_store_open.rs` and
`inspect_mailboxes.rs` both exist because a personal store is the only place
these questions have real answers, and both are built to be safe to point at
one: read-only where reading is enough, a refusal to touch the live path
where it is not, keyring entries retrieved and never minted, and ids, paths
and counts selected rather than mail.

`time_store_open` refuses to run against `~/.local/share/postio/postio.db` on
purpose. Opening read-write is what checkpoints the WAL, so the obvious way
to measure the problem is also the way to erase it — and it would have taken
the evidence for this note with it.

## The general lesson

The file sizes were real, the missing pragmas were real, and the mechanism
joining them to the symptom was invented. An issue was filed at `p1` with a
confident cause, a proposed fix, and no measurement — and the fix would have
bought nothing, because the thing it addressed cost 32 ms of a five-second
startup.

A copy of a store is free on this filesystem and a timing harness is twenty
minutes. Neither is expensive next to fixing the wrong thing. This is the
same shape as "Four build-time tips that did not survive being measured":
the plausible cause and the actual cause are different questions, and only
one of them is answerable by reading code.
