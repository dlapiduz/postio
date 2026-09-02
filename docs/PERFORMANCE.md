# Performance: budgets and the measured baseline

Performance is a functional requirement in Postio, enforced by `cargo bench`
rather than checked by hand at the end:

| Budget | Target | Measured |
|---|---|---|
| Startup to usable UI (populated DB) | < 500 ms | **427 ms** |
| Ordinary UI interaction | < 16 ms | **0.3 ms** typical, one case over — see below |
| Local search | < 100 ms | **42 ms** worst shape |
| Memory, 100,000 messages | no full-mailbox load | **55 MiB**, flat past 100k |

Transitions are ≤ 100 ms or absent entirely, and `prefers-reduced-motion` is
always honored. A mailbox is never loaded into memory in full — the message
list is windowed over paged SQLite.

**These numbers are from an encrypted store.** Since ADR 0014 the database is
SQLCipher and there is no unencrypted configuration to compare against in
normal use, so every figure here already carries the cost of decrypting each
page on the way in. Where that cost is separable it is stated.

## How to read these numbers

Measured on one developer machine, which makes them a regression guard rather
than a promise about anybody else's hardware — and that machine routinely runs
several build and test sessions at once. Wall-clock figures below are
therefore the **floor of repeated runs**, not the mean: the floor is the
least-contended sample and the most reproducible statistic available here. The
spread is reported where it is wide enough to matter.

Reproduce them:

```sh
cargo run -p postio-runtime --example seed_store -- /tmp/postio.db 20000
POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 POSTIO_STARTUP_EXIT=1 \
  cargo run --release -p postio-app

cargo bench -p postio-runtime --bench store_reads   # the database read
cargo bench -p postio-index   --bench search_budget # the query
cargo bench -p postio-gtk     --bench list_scroll   # the row draw
```

`seed_store` needs this installation's store key, which lives in the OS
keyring — so run `postio` once before seeding, or there is no key to encrypt
the scratch store with. The tool deliberately never mints one.

## Startup

On a 20,000-message store with an account and six folders:

| | floor | spread over 5 runs |
|---|---:|---|
| Startup to usable UI | **427 ms** | 427 – 723 ms |
| of which `adw::init` | 66 ms | |
| of which window construction | 228 ms | |
| of which first frame | 106 ms | |
| fonts and styles together | 9 ms | |

Two things about this deserve saying plainly rather than being averaged away.

**Encryption costs about 78 ms of it.** Measured by disabling `PRAGMA key` in
`db::configure` and re-running the same binary against an equivalent
plaintext store: floor 350 ms against 427 ms, and the difference sits almost
entirely in window construction, which is where the first store reads happen.
That is the price ADR 0014 said would land on this budget, and it lands inside
it.

**Most of the rest is GTK's own first-realize cost, not a Postio widget.**
#636 bisected window construction by timing each pane's constructor in
isolation (`Shell`, `Sidebar`, `MessageListView`, `Finder`, `CheatSheet`,
`SettingsPanel`, the composer's `Editor` and its WebView) — none cost more
than a few milliseconds, and they sum to well under 30 ms. What actually
costs the rest is `GtkWindow::present()` itself, and only for **whichever
widget is presented first in the process**: presenting a bare `Sidebar`
alone first cost 110 ms; presenting it second, after something else had
already triggered a first realize, cost 12 ms. Forcing `GSK_RENDERER`
confirmed it further — `cairo` (software) measured ~181 ms end to end where
the platform default measured ~295 ms, and `ngl` measured ~1.9 s. GSK's
GPU renderer backends compile their shaders on first use; that compile is
what the "window" phase is mostly paying for, once.

That makes this a rendering-backend tradeoff, not a bug in any one widget:
trading it away buys faster startup at the cost of GPU-accelerated runtime
compositing, and nothing in this investigation measured whether that costs
the 16 ms interaction budget anywhere real use would notice. Filed as
[#790](https://github.com/dlapiduz/postio/issues/790) rather than decided here. This document previously
recorded 147 ms for the whole figure; nothing in that measurement survives
to compare against — different commit, different schema, and no record of
what else the machine was doing — so the honest statement is 427 ms today,
of which 78 ms is encryption and the great majority of the remainder is the
first-realize cost above. The worst of five runs exceeded the 500 ms
budget. See [#636](https://github.com/dlapiduz/postio/issues/636) for the full investigation.

## Interaction: the page read and the row draw

An ordinary interaction is a scroll, and a scroll is two things — a page read
and a screenful of rows drawn.

| | 1,000 messages | 100,000 messages |
|---|---|---|
| Message page, top of the folder | 249 µs | 302 µs |
| Message page, scrolled to the middle | — | 343 µs |
| Message page, *jumped* to the middle | — | 233 ms |
| Thread page, top of the folder | 1.14 ms | 1.40 ms |
| Thread page, ten pages down | — | 1.42 ms |
| Unified page, two accounts | — | 18.7 ms |

Flat against mailbox size, which is the claim that matters: reading page one
of a hundred thousand messages costs what reading page one of a thousand
costs.

Two exceptions, both real:

- **A *jump* to a page nobody has scrolled through** — the store has no
  boundary to seek from and falls back to walking. It happens once per jump,
  and every page after it is the 343 µs row. It is far more expensive than it
  was when this document last recorded it (28 ms), and under page encryption
  a walk pays a decrypt per page, so this is the case where the cipher costs
  most. [#638](https://github.com/dlapiduz/postio/issues/638).
- **The unified page is over budget** at 18.7 ms against 16 ms, and encryption
  is not why: the same bench against a plaintext store measures 18.1 ms, and
  raising `cache_size` from 16 MiB to 64 MiB changes nothing measurable
  (p = 0.67). The time is CPU in the query itself.
  [#619](https://github.com/dlapiduz/postio/issues/619).

## Search

Over a 120,000-message index, by query shape:

| Query shape | Measured |
|---|---|
| Composed — an operator plus free text, what the search bar usually produces | 3.0 ms |
| Simple term — a word matching about 1% of the corpus | 4.3 ms |
| Account-scoped common word | 9.7 ms |
| Unified common word, across accounts | 15.0 ms |
| Operator only — `from:`, no free text and no FTS join | 18.8 ms |
| Common word — a word in every message, the worst case | 27.8 ms |
| Common word, with facet counts | 41.8 ms |

The worst shape is the one to watch: `MATCH` and the `count(*)` behind it have
to walk effectively the whole corpus, and it is where a missing index would
show up first. It is inside the 100 ms budget with room, which is what matters
now that every page of that walk is decrypted on the way in.

## Memory, and the claim it tests

A mailbox is never loaded into memory in full. Measured rather than asserted,
by sampling `/proc/<pid>/status` on a store of each size, **after the startup
passes have settled**:

| | 1,000 | 100,000 | 400,000 |
|---|---|---|---|
| Store on disk | 0.9 MiB | 75 MiB | 234 MiB |
| Anonymous — the application's own heap | 39.6 MiB | 55.3 MiB | 55.3 MiB |
| File-backed — shared libraries | 121.0 MiB | 121.4 MiB | 121.6 MiB |
| Resident total | 160.8 MiB | 176.7 MiB | 176.9 MiB |

**The anonymous figure is the claim**, and the shape of it is the answer: it
steps up once between a thousand messages and a hundred thousand — the SQLite
page cache filling, bounded by `cache_size` — and then does not move at all
between a hundred thousand and four hundred thousand, against a store that
tripled. Bounded, not proportional.

**The file-backed half is now flat, and that is new.** It used to grow from
83 MiB to 167 MiB with mailbox size, because `PRAGMA mmap_size` was 256 MiB
and SQLite mapped as much of the store as it touched. That pragma is gone:
memory-mapping is meaningless over encrypted pages, since SQLCipher has to
decrypt each one into the page cache, so there is no version of "the file is
the buffer" (ADR 0014). What is left in this row is shared libraries.

Net effect at 100,000 messages: resident total went from 215 MiB to 177 MiB.
Removing mmap moved memory out of the file-backed half and gave a little of it
back to the anonymous one, and the total improved.

**A transient worth knowing about.** During the first minute on a large store,
anonymous memory peaks well above the settled figure — 86 MiB on the 400,000
store — while the body-index catch-up and the compression-dictionary trainer
run. The trainer reads up to 4,096 bodies or 32 MiB of samples, whichever
comes first (`postio_storage::body`), and frees them when it is done. Both are
idle-time passes on a worker; neither is on the startup path.

Reproduce it:

```sh
cargo run --release -p postio-runtime --example seed_store -- /tmp/big.db 100000
POSTIO_STORE=/tmp/big.db cargo run --release -p postio-app &
sleep 45   # let the catch-up passes settle, or you measure the transient
grep -E '^(VmRSS|RssAnon|RssFile):' /proc/$(pgrep -n postio)/status
```
