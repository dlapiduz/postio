# ADR 0027 — The header index is budgeted per message, not against the body index

- **Status:** Accepted (2026-09-04)
- **Date:** 2026-09-04
- **Decision by:** `/ux-architect`, on [#1041](https://github.com/dlapiduz/postio/issues/1041), which #926 raised when it built the index ADR 0025 specified and could not meet the budget ADR 0025 set for it.
- **Issue:** [#1041](https://github.com/dlapiduz/postio/issues/1041)
- **Amends:** [ADR 0025](0025-arbitrary-headers-are-indexed-rows.md) Q3, whose budget — *"`message_headers` ≤ 25% of what `message_bodies_fts` costs on the same corpus"* — is replaced by the per-message ceiling below; and ADR 0025 Q2's claim that headers under those caps are *"metadata scale"*, which the measurement contradicts.
- **Related:** [ADR 0016](0016-full-mailbox-backfill-by-default.md) (the duplication that was refused), [ADR 0017](0017-backfill-cost-attachments-memory-disk-encryption.md) (the store's byte budget and its eviction policy), [ADR 0020](0020-where-message-bodies-live.md) (`body_headers`, the trained dictionary), [ADR 0014](0014-encryption-at-rest.md), `PRODUCT.md` §18 (why this project gates counts and not stopwatches), [#926](https://github.com/dlapiduz/postio/issues/926) (the index and the operator), [#884](https://github.com/dlapiduz/postio/issues/884)
- **Decision:** **the two caps stay exactly where they are, the operator ships as ADR 0025 specifies it, and the size gate becomes a ceiling on `message_headers` + `idx_message_headers_name` in bytes per message — 5 KiB — measured on the heavy fixture the test already builds.** The ratio against `message_bodies_fts` survives as a printed observation and asserts nothing. `header:` costs **3.72 KiB per message** on that fixture, which is a line item this project now has written down rather than a claim it failed.

---

## What the budget was for, and why it could not work

ADR 0025 Q3 set the gate as a share of `message_bodies_fts` and gave the reason: *"a relative figure rather than an absolute one, because it is the ratio that stays true on another machine and another mailbox."* The instinct is right and is this project's own — `PRODUCT.md` §18 gates counts rather than milliseconds precisely because a count is the same number on a developer's machine and on a shared runner. The mistake is in which relative figure.

**The two objects are not the same kind of thing, and the difference is the entire ratio.** `message_bodies_fts` is `content = ''` — an inverted index that holds no text at all, which was the whole point of ADR 0016 moving the bodies there. `message_headers` holds every value verbatim, because ADR 0025 Q2 rejected an FTS table here on the ground that a substring match needs the string. So the ratio is not "how expensive is the header policy"; it is "how well does FTS5 compress this fixture's bodies", asked in a way that charges the answer to headers.

**It moves for reasons that have nothing to do with the policy under test.** #926's own measurements make the point without needing an argument:

| corpus | share of `message_bodies_fts` |
|---|---|
| long threaded bodies, no signatures, 3 `Received` hops | 64% |
| long threaded bodies, one DKIM signature, 4 hops | 107% |
| long threaded bodies, three ARC/DKIM signatures, 8 hops | 205% |
| **short** bodies, one DKIM signature, 4 hops | 269% |
| the fixture now committed in `header_index_size.rs` | **636%** |

The fourth row is the tell. Nothing about the header policy changed between rows two and four; the *bodies* got shorter, the denominator shrank, and the header index "regressed" by 2.5x. A mailbox of two-line notifications — a real and common shape — fails hardest, and no cap can save it, because the cap is on the numerator. A gate a corpus can fail for a reason unrelated to what it gates is not a gate.

The fifth row is the same effect seen once more: it differs from #926's table because the fixture's bodies differ, which is the property being complained about.

**And the caps genuinely are not the lever.** Simulated over the committed fixture's own header block, summing name and value bytes at each cap:

| value cap | payload per message | what it costs |
|---|---|---|
| 512 (today) | 2,793 B | — |
| 256 | 2,025 B | the tail of a long `X-Spam-Report` or `Authentication-Results` |
| 128 | 1,630 B | the id and date at the end of every `Received` hop |
| 64 | 1,246 B | `header:x-mailer=mutt 1.5.24` stops matching its own value |

The cap can take about half, once, and never an order of magnitude, because what remains under it is 22 rows of ordinary MIME furniture — the names, the `Received` chain's front half, `Content-Type`, `Message-ID`, `List-Id`. ADR 0025 Q3 is right that reaching 25% needs roughly fifty bytes of header per message, and right that fifty bytes is an allowlist arrived at by arithmetic instead of by name. It set a target its own instruments could not reach, and told the implementer to reach it with the one lever that does not move far enough.

## Q1 — Which of the two statements to revise: the budget

**Decision: the budget. The operator ships as specified and the caps do not move.**

#1041 offers three doors. Taking them in turn:

**The operator is not too broad.** Narrowing it structurally means a much lower value cap, and the table above prices that: 512 → 128 saves 42% of the payload and costs the identifiable part of every `Received` hop. 512 → 64 saves 55% and costs the motivating example from ADR 0025 Q6 — `x-mailer` is `Mutt 1.5.24 (2015-08-30)` and at 64 bytes a `Received` line is `from relay0.example.net (relay0.example.net [192.0` and nothing else. Halving a cost is not worth breaking the feature's own worked example, and it would still be six times a budget that was unreachable for a reason no cap addresses.

**Nor do the caps move for tidiness.** 512 bytes is doing real work at the far end: `X-Spam-Report` and `X-Spam-Status` carry the rule names people actually grep for several hundred bytes in, and a DKIM `d=` domain sits in the first sixty. The 27% that dropping to 256 would save is not worth losing the one class of header where the interesting substring is deep. **This paragraph exists so the cap question is not re-opened every time the number is measured: it has been priced, and the answer is no.**

**There is no cheaper shape.** Interning names and values into side tables is the only idea that changes anything without changing behaviour, and #1041 already prices it at about a third — `Received`, `Message-ID` and the signatures are unique per message and are most of the bytes. Answering `header:name=value` by narrowing on a value-less index and post-filtering in Rust collapses on exactly the queries people write: `header:content-type=multipart/signed` would decompress every message in the mailbox, because every message has a `Content-Type`. ADR 0025 rejected that shape and it stays rejected.

So the cost is intrinsic. **Substring-matching arbitrary header values requires the values on disk**, and the only decision left is whether the values are worth their bytes. ADR 0025 already decided they are. What was missing was the number.

## Q2 — What the gate measures: bytes per message, table and index

**Decision: `message_headers` plus `idx_message_headers_name`, in `dbstat` page bytes, divided by the number of messages, must stay under 5 KiB.**

Bytes per message is relative in the way that matters and absolute in the way that does not. It is invariant to the one quantity that differs between mailboxes — how many messages there are — and it is the number that multiplies straight into disk: a user with 80,000 messages can be told what `header:` costs them without anyone re-running anything. It is also, unlike the ratio, a number the two caps actually control.

It gives up portability across *corpora*: a personal mailbox on a small server carries ten fields where a Gmail-delivered list message carries twenty-five, so the figure is a property of the fixture as well as of the policy. That is a real loss and it is the smaller one, because the fixture is committed, deliberately heavy, and read as a **ceiling** — a lighter mailbox passes by definition. The ratio failed the opposite way: a lighter mailbox failed harder.

**Count the index too.** The measurement as landed asks `dbstat` for `name = 'message_headers' OR name LIKE 'message\_headers\_%'` — a pattern written for FTS5's shadow tables, which `idx_message_headers_name` does not match. The index is 221 KB against the table's 1.30 MB on 400 messages: **the cost was being understated by 17%**, and a secondary index is part of what a policy costs. Both b-trees by name, and any future one with them.

**The ceiling is 5 KiB and the headroom is deliberate.** Measured on the committed fixture, 400 messages: table 1,302,528 B, index 221,184 B, **3,809 B per message**. 5 KiB is about a third above that — enough to absorb ADR 0017's move to `page_size = 8192`, a b-tree fanout change, or a SQLite upgrade, and far too little to absorb a policy change.

**What each of the three tests catches, since one number cannot catch everything:**

| Test | Catches |
|---|---|
| `no_message_may_contribute_more_than_the_two_caps_allow` | either cap ceasing to bind — asserted exactly, on a pathological message, and it is what fails the instant truncation is removed |
| the per-message ceiling here | the aggregate cost of the policy: a new column, a second index, names that stop being shared, a value that is stored twice |
| `search_statement_budget.rs` | the read path, which size does not describe |

The share of `message_bodies_fts` stays in the output as a printed line, labelled as an observation. It is worth knowing and it is not worth failing a build over.

## Q3 — What it costs, stated rather than implied

**3.72 KiB per message**, on a fixture carrying three signature sets, a three-hop `Received` chain and a full mailing-list header set. Against ADR 0017's reference account — 81,744 messages — that is **approximately 310 MB**, or roughly a third on top of that ADR's projection for a fully backfilled store.

That number is the reason this ADR exists, and it is written here rather than left in a test's output because of what it obliges:

- **`[storage] max_bytes` must count it.** ADR 0017 gives the store a byte budget whose eviction reclaims *refetchable* blobs. `message_headers` is not a blob and is not refetchable in that sense — it rebuilds from `body_headers` with no network, which is cheaper than a refetch, not more expensive. It is therefore **fixed overhead** against that budget, not something the sweep can take, and the budget's accounting must include it from the start rather than discover it.
- **Dropping it is not a silent option.** ADR 0025 Q5's rule is that nothing may *silently* answer "no such mail". A store that evicted the header index would have to say so, the way Q4 already has the backfill status line say how much of the mailbox `body:` can see. That is a coherent future design and it is not this decision; if `max_bytes` ever makes it necessary, it is a new question with the mechanism already in place.
- **Encryption is unaffected.** ADR 0014's gate is the 100 ms search budget, and SQLCipher decrypts the pages a query touches. `header:` narrows on `name` through `idx_message_headers_name`, so it touches one name's range and not the table; the size shows up on disk and in `VACUUM`, not in the search budget.

## Q4 — ADR 0025 Q2's "metadata scale" is wrong, and the right claim is stronger

Q2 defended a content table on the ground that *"headers under Q3's caps are metadata scale"*, contrasted with `search_documents.body`, which was corpus scale. Measured on one corpus, per message:

| | bytes per message |
|---|---|
| `search_documents` + `messages_fts` — the metadata half of the same index | 184 B |
| `messages` | 225 B |
| `message_bodies_fts` | 524 B |
| **`message_headers` + its index** | **3,809 B** |

The header index is seventeen times the metadata half of the index it sits in, and seven times the body index. It is not metadata scale by any reading.

The claim Q2 needed is a different one and it survives the measurement: **`message_headers` is bounded per message and `search_documents.body` was not.** A body is capped at 5 MB and a mailbox has no bound at all; a message's header rows cannot exceed 64 × 512 bytes however pathological the mail, and the fixture lands at 3.7 KiB against that 32 KiB ceiling. Boundedness, not smallness, is what made the duplication acceptable — and boundedness is a structural guarantee a test can hold, which is what `no_message_may_contribute_more_than_the_two_caps_allow` does. Q2's conclusion stands; its reason is replaced.

---

## Alternatives

**Keep the ratio and lower the caps until it passes.** What ADR 0025 Q3 instructs. Rejected in Q1: the caps reach about half, the target needs a twentieth, and the arithmetic converges on a two-field allowlist — the thing Q3 itself refuses. Following the instruction produces the outcome the instruction forbids.

**Keep the ratio and raise the number to 650%.** The smallest possible edit, and it looks honest because the measurement is real. Rejected: it certifies a quantity nobody wants to know. A 650% gate passes when the bodies are long and fails when a corpus of short mail arrives, so the first person to point it at a notifications mailbox gets a red build and no defect. Worse, it teaches the next reader that this is how sizes are budgeted here.

**A share of the whole SQLite file.** Comparable in kind, portable, and it is how ADR 0017 talks — `idx_recipients_draft` at 3.9% of the database, `recipients` at 34%. Rejected because the denominator is whatever the fixture happens to create: this test's store holds no compressed body text and no blobs, so the header half measures 66% of the database, a figure about the fixture's omissions. Making the denominator honest means building a realistic store in a test that exists to measure one table, and the per-message figure is the same information without the machinery.

**A share of `messages.body_headers`, the column the rows derive from.** The most attractive rejected option: same data, same database, same corpus, and it measures the real quantity — how much indexing headers costs over keeping them, which is where the two caps bite. It would be immune to body length entirely. Rejected on instrumentation: `dbstat` reports page usage per b-tree, not per column, so `body_headers` cannot be separated from the rest of the `messages` row without a second store built to hold nothing else. Worth revisiting if a per-column measurement ever becomes cheap; the per-message ceiling gates the same regressions today.

**Ship `header:` without a size gate at all.** ADR 0025 Q3's third forbidden door, and it stays shut. The reason to gate is not that the number might be large — it is 310 MB and that is now recorded — but that nothing else in the tree notices when a schema change doubles it.

## Consequences

- `crates/postio-index/tests/header_index_size.rs` replaces its ratio assertion with the per-message ceiling, counts `idx_message_headers_name` alongside the table, and runs in the default suite: the `#[ignore]` and its pointer at #1041 both go.
- ADR 0025 Q3's budget sentence and Q2's "metadata scale" sentence are marked amended in place, pointing here.
- `HEADERS_SCHEMA_VERSION` does **not** move. Neither cap changes, so no store is refilled and nothing re-indexes — which is the cheapest possible outcome of this decision and a reason to prefer it on its own.
- #926's last acceptance criterion is met by that test change, and #926 closes with it.
- ADR 0017's `[storage] max_bytes` work gains a fixed overhead line it did not have: the header index, at ~3.7 KiB per message, is not evictable.
