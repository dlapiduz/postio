# ADR 0001 — IMAP library: `io-imap`

- **Status:** Accepted — **GO**
- **Date:** 2026-08-22
- **Bead:** `postio-yop` (SPIKE), parent epic `postio-wy2` (E4), affects epic E5 (incremental sync)
- **Decision:** adopt `io-imap` and **pin `= 0.6.0`**.

---

## Context and method

`io-imap` is days old and pre-1.0 (0.1.0 on 2026-06-03, 0.6.0 on 2026-08-22 — six
minor releases in eleven weeks). Committing `postio-sync` to it before checking it
against the real target server would be the most expensive possible mistake.

The scope of this spike was cut on 2026-08-22 because part of it was already
answered by an existing install: `himalaya-tui` is installed on this box from git,
depends on `io-imap` 0.5 / `io-smtp` 0.3 / `io-sasl` 0.1 / `pimalaya-stream` 0.3,
and works against the owner's live iCloud account. **"Does the Pimalaya stack talk
to iCloud at all" is therefore already answered: yes.**

Important qualifier on that evidence: `himalaya-tui` exercises only the *plain*
command set. Its adapter
(`~/.cargo/git/checkouts/himalaya-tui-5ba74af2831f7542/beed4ed/src/imap/backend.rs`)
calls `select(.., ImapMailboxSelectOptions::default())` everywhere and never
touches ENABLE, CONDSTORE, QRESYNC or IDLE. So its success proves *session open +
SASL + SELECT + FETCH + STORE + COPY/MOVE + LIST* against iCloud, and proves
nothing at all about the extensions our sync design depends on.

This spike therefore answers the four remaining questions **by source
inspection**, per the instruction not to touch the live account. No credentials
were read and no authenticated connection was made. Sources read:

- `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/io-imap-0.5.0/` (whole tree)
- `io-imap` 0.6.0 crate tarball from `static.crates.io`, unpacked and diffed
  against 0.5.0 (scratchpad only, nothing added to the workspace)
- `io-imap` `CHANGELOG.md` (0.3.0 → 0.6.0)
- `imap-types` 2.0.0-alpha.7 (`src/command.rs`, `src/response.rs`, `src/fetch.rs`,
  `src/status.rs`)
- `himalaya-tui` at `beed4ed` as a working reference wiring
- Apple Developer Forums thread 694251 (iCloud CONDSTORE/QRESYNC conformance)

---

## Q1 — API churn: 0.5 vs 0.6. Adopt 0.6 or pin 0.5?

**Answer: adopt 0.6.0. The breakage between 0.5 and 0.6 is confined entirely to
the one component we were never going to use.**

A full tree diff of `io-imap-0.5.0/src` against `io-imap-0.6.0/src` shows **three
changed files**:

| File | Changed lines | What changed |
|---|---:|---|
| `src/watch.rs` | 668 | rewritten: options struct, IDLE-vs-poll, QRESYNC no longer required, UIDVALIDITY check |
| `src/client.rs` | 68 | `watch_mailbox` takes `ImapMailboxWatchStreamOptions`; new options struct |
| `src/lib.rs` | 7 | doc comment for the above |

Everything else is **byte-identical**: `session.rs`, `send.rs`, `coroutine.rs`,
all of `rfc3501/` (select, examine, fetch, store, status, …), `rfc2177/idle.rs`,
`rfc5161/enable.rs`, `rfc4315/`, `rfc6851/`, `sasl/`, `rfc7628/`, `rfc7677/`.
Dependencies are unchanged (`imap-codec 2.0.0-alpha.8`, resolving to alpha.9;
`imap-types 2.0.0-alpha.7`; `io-sasl 0.1`; `pimalaya-stream 0.3`; MSRV 1.88).

The three breaking items in the 0.6.0 changelog are all `watch`:

1. `ImapMailboxWatch::new` gained an `ImapMailboxWatchOptions` argument and now
   returns `Self` instead of `Result<Self, _>`;
2. `ImapMailboxWatchError::QresyncUnsupported` is gone (a non-QRESYNC server now
   falls back to re-reading the whole mailbox);
3. `ImapClientStd::watch_mailbox` takes `ImapMailboxWatchStreamOptions`.

We do not use `ImapMailboxWatch` (see Q2 caveat 4), so all three are free.

0.6 also brings two things we do want: `idle_timeout` is now configurable
(0.5 hard-coded 29 s, ~120 re-IDLE round trips per hour per mailbox), and a
polling watch mode as a fallback for a server that accepts IDLE then goes silent.

**Churn assessment — this is the real risk, not any single release.** The 0.5.0
changelog is enormous and almost entirely breaking: every SASL mechanism moved to
`io-sasl`, command methods moved off `ImapClientStd` onto new `ImapClient` /
`ImapClientAsync` traits, `ImapClientStdError` renamed to `ImapClientError`,
`connect()` signature replaced, the whole `session` module added, MSRV raised. A
release of that size landed seven days before 0.6.0. Expect another within two
weeks.

The repository invariant in `CLAUDE.md` — *"`postio-sync` talks to the
`MailBackend` trait, never to `io-imap` types directly — that crate is pre-1.0 and
moving fast"* — is exactly the right mitigation and is now load-bearing rather
than stylistic. Keep the blast radius inside `postio-imap`.

**Pin:** `io-imap = { version = "=0.6.0", default-features = false }`. Use an
exact `=` pin, not a caret: caret ranges on `0.x` already lock the minor, but the
exact pin documents that a bump is a deliberate, reviewed event. Take
`imap-codec`/`imap-types` from `io_imap::codec` / `io_imap::types` re-exports —
never depend on them directly, or a codec bump inside io-imap becomes a type
mismatch in our tree.

---

## Q2 — Are QRESYNC / CONDSTORE actually reachable through the public API?

# ✅ YES. QRESYNC and CONDSTORE are first-class in `io-imap`'s public API. Epic E5's incremental-sync design does not need to change.

This was the question that could have killed the plan. It does not. There is no
`rfc7162` module — which looks alarming at first glance — because the extension is
deliberately surfaced as *parameters and response fields* of the existing
coroutines. From `src/lib.rs`:

> "The CONDSTORE and QRESYNC extensions (RFC 7162) have no module of their own:
> they surface as parameters and response fields of the [`rfc3501`] select,
> examine and fetch coroutines, and power [`watch`]."

Verified in source, not just docs:

**ENABLE** — `rfc5161::enable::ImapExtensionEnable::new(Vec1<CapabilityEnable>)`
takes an arbitrary capability list. `CapabilityEnable::CondStore` is a typed
variant; QRESYNC is not in the enum but routes through
`CapabilityEnable::from(Atom::try_from("QRESYNC"))`, which is exactly what
`watch.rs:220-227` does upstream.

**SELECT / EXAMINE with parameters** —
`ImapMailboxSelectOptions { parameters: Vec<SelectParameter> }`
(`src/rfc3501/select.rs:127-131`), passed straight into
`CommandBody::Select { parameters, .. }`. `imap_types::command::SelectParameter`
(`imap-types-2.0.0-alpha.7/src/command.rs:1862`) is:

```rust
pub enum SelectParameter {
    CondStore,
    QResync {
        uid_validity: NonZeroU32,
        mod_sequence_value: NonZeroU64,
        known_uids: Option<SequenceSet>,
        seq_match_data: Option<(SequenceSet, SequenceSet)>,
    },
}
```

The full RFC 7162 §3.2.5 parameter set, including `known_uids` and the
`seq_match_data` sequence-match pair. `ext_condstore_qresync` is a *non-optional*
feature of the `imap-codec` dependency, so it is always compiled in.

**QRESYNC response data is decoded, not dropped** —
`ImapMailboxSelectData` (`src/rfc3501/select.rs:91-113`) carries:

- `highest_mod_seq: Option<u64>` — from `Code::HighestModSeq`
- `vanished_earlier: Vec<NonZeroU32>` — from `* VANISHED (EARLIER) <uid-set>`,
  expanded from the sequence set by `expand_uid_set` at `select.rs:248`
- `changed: Vec<ImapMailboxSelectFetch>` — the implicit FETCHes of a QRESYNC SELECT
- plus `uid_validity`, `uid_next`, `exists`, `permanent_flags`

`rfc3501/examine.rs` mirrors it exactly (`examine.rs:166`, `:185`).

That is the complete QRESYNC resync payload: *what changed*, *what vanished*, and
*the new HIGHESTMODSEQ to persist*. This is precisely what E5 needs.

**FETCH modifiers** — `ImapMessageFetchOptions { uid: bool, modifiers: Vec<FetchModifier> }`
(`src/rfc3501/fetch.rs:140-147`), with
`FetchModifier::{ChangedSince(NonZeroU64), Vanished}`
(`imap-types/src/command.rs:1878`). So `UID FETCH 1:* (FLAGS) (CHANGEDSINCE n)` is
expressible.

**Per-message MODSEQ** — `MessageDataItemName::ModSeq` and
`MessageDataItem::ModSeq(NonZeroU64)` (`imap-types/src/fetch.rs:246`, `:387`) are
requestable and decoded; FETCH returns
`BTreeMap<NonZeroU32, Vec1<MessageDataItem>>` verbatim, so nothing is filtered.

**STATUS (HIGHESTMODSEQ)** — `ImapMailboxStatus::new` takes an arbitrary
`Cow<'static, [StatusDataItemName]>` and `StatusDataItemName::HighestModSeq`
exists (`imap-types/src/status.rs:38`). Cheap per-mailbox change detection for
the mailboxes we are not currently watching. Works.

**SEARCH MODSEQ** — `SearchKey::ModSequence` exists
(`imap-types/src/search.rs:167`).

**Convenience helper** — `ImapClient::select_qresync(mailbox, uid_validity,
highest_mod_seq, capability)` (`src/client.rs:320-334`) builds the parameter for
you and guards it, erroring `QresyncNotSupported` if `capability` lacks
`Capability::QResync` and `InvalidModSeq` on a zero mod-seq.

### Four real gaps to design around (none is a blocker)

**1. `STORE` cannot send `UNCHANGEDSINCE`. Confirmed defect for our use.**
Both `ImapMessageStore::new` and `ImapMessageStoreSilent::new` hard-code
`modifiers: Default::default()` (`src/rfc3501/store.rs:160` and `:242`), and
`ImapMessageStoreOptions` has exactly one field, `uid: bool` (`store.rs:138-141`).
`StoreModifier::UnchangedSince` exists in `imap-types` and `Code::Modified` exists
in the response types (`imap-types/src/response.rs:936`), but io-imap wires
neither — the store coroutine never inspects the response code.

Consequence: **RFC 7162 §3.1.3 conditional STORE is unavailable.** We cannot do
optimistic-concurrency flag writes ("set `\Seen` only if nothing changed since
modseq N") and get the `MODIFIED` rejection list back. Our flag writes are
last-writer-wins.

Impact is small and acceptable for v1: our architecture is local-first with an
operation queue, so a lost race means a flag flips back on the next QRESYNC pull,
which is the same outcome most clients ship. Workarounds if it ever matters:
`ImapClient::raw()` sends verbatim tagged command lines and returns the raw
response (`src/client.rs:307-315`), so `UID STORE ... (UNCHANGEDSINCE n) ...` can
be issued by hand; or upstream a one-line PR adding `modifiers` to
`ImapMessageStoreOptions`. **File a bead; do not block E4.2 on it.**

**2. `ImapMessageFetch` sends `VANISHED` but discards the response.**
`FetchModifier::Vanished` is accepted on the wire, but the response loop at
`src/rfc3501/fetch.rs:203-208` keeps only `Data::Fetch` and drops everything else
— so the `* VANISHED (EARLIER) …` lines produced by
`UID FETCH … (CHANGEDSINCE n VANISHED)` are silently thrown away.

Consequence: **do not build the expunge half of resync on FETCH+VANISHED.** Get
expunges from `SELECT (QRESYNC …)` / `EXAMINE (QRESYNC …)` instead, which does
decode `vanished_earlier` correctly. That is the better shape for us anyway: one
command returns changes, vanishes and the new HIGHESTMODSEQ together.

**3. Streamed FETCH cannot carry modifiers.** `fetch_stream.rs:162` and
`fetch_stream_batch.rs:128` hard-code `modifiers: Vec::new()`. Irrelevant in
practice — those fetch `BODY.PEEK[]`, and bodies are immutable; CHANGEDSINCE
belongs on the metadata fetch, which is the non-streaming one.

**4. `watch::ImapMailboxWatch` is not usable by Postio.** It seeds an in-memory
`shadow: BTreeMap<NonZeroU32, Vec<Flag>>` from a full `FETCH 1:* (UID FLAGS)`
over the entire mailbox (`watch.rs:378-405`) and holds it for the life of the
watch. That violates the `CLAUDE.md` invariant *"Never load a whole mailbox into
memory"* and would cost a full-mailbox scan on every reconnect. It also uses
EXAMINE (read-only), so it cannot serve a read-write session.

This is fine: we build `postio-sync`'s loop from the primitives — `ENABLE
CONDSTORE QRESYNC` → `SELECT (CONDSTORE)` → persist HIGHESTMODSEQ → `IDLE` →
`SELECT (QRESYNC …)` → apply deltas — which is the same state machine `watch.rs`
implements and which we can read as a reference. Treat `watch.rs` as documentation,
not as a dependency. **This is also why the 0.6 breakage costs us nothing.**

---

## Q3 — Does `io-imap` re-issue CAPABILITY after auth?

**Answer: yes, if you open the session through `session::ImapSessionOpen` (or
`ImapClientStd::connect`). No, if you call the auth coroutines directly with
default options — and that is a live footgun for iCloud specifically.**

iCloud does not advertise its full capability set in the pre-auth banner; CONDSTORE,
QRESYNC, UIDPLUS and IDLE only appear once authenticated, and the Apple Developer
Forums thread on this notes iCloud also requires clients to *ask again* after
login rather than pushing a new list. So getting this wrong means silently losing
every extension E5 depends on.

**The safe path.** `session::ImapSessionOpen` hard-codes
`let ensure_capabilities = true;` (`src/session.rs:366`) and passes it into every
SASL/LOGIN variant (`session.rs:368-423`). `ImapSessionOpenData.capability` is
documented as *"the capabilities advertised once the session reached its final
state, after authentication when one took place"* (`session.rs:260-269`). The
auth coroutines implement it correctly: `ImapLogin` reads a `Code::Capability`
from the tagged OK **and** any untagged `* CAPABILITY` line
(`src/rfc3501/login.rs:190-206`), and if neither is present it transitions to
`State::Capability(ImapCapabilityGet::new())` and issues a real `CAPABILITY`
round-trip (`login.rs:141-144`, `:207-211`, `:224-233`).

`ImapClientStd::connect` is a pump over that coroutine and returns the post-auth
list as its second tuple element — which is exactly how `himalaya-tui` uses it:

```rust
let (inner, _capabilities) = Inner::connect(&server, &tls, sasl, opts)?;
```
(`himalaya-tui/src/imap/client.rs:62`)

**The footgun.** `ImapLoginOptions::ensure_capabilities` and its
`ImapAuthPlainOptions` / `ImapAuthLoginOptions` siblings **default to `false`**
(`src/rfc3501/login.rs:95-105`: *"Fetch capabilities explicitly when the LOGIN
response carries none. Defaults to skipping the extra round-trip."*). A caller
that drives `client.auth_plain(.., Default::default())` by hand against a server
that returns no capability code gets an **empty `Vec<Capability>`** back, no error.
Feed that to `select_qresync()` and it returns `QresyncNotSupported`; feed it to
any of our own capability gates and we silently degrade to full resync forever.

**Rule for `postio-imap`:** capabilities come from `ImapSessionOpen` /
`ImapClientStd::connect` only. If a code path ever constructs an auth coroutine
directly, it must set `ensure_capabilities: true`. Add a unit test against the
`MailBackend` mock asserting we never accept an empty post-auth capability list,
and a `#[ignore]`d live test asserting `QResync`, `CondStore`, `Idle` and
`UidPlus` are all present after login to `imap.mail.me.com`.

Note also that after a STARTTLS upgrade the capability list must be re-read;
`ImapSessionOpen` handles that too (and, since 0.5, refuses the upgrade outright
if the server appends bytes to the STARTTLS tagged response — a plaintext
injection signal). We use implicit TLS on 993, so this is not on our path.

---

## Q4 — Is IDLE exposed and usable?

**Answer: yes, as a first-class coroutine. But `ImapClientStd` does not expose a
plain `idle()` method — we drive the coroutine ourselves, which we were going to
do anyway.**

`rfc2177::idle::ImapIdle::new(shutdown: Arc<AtomicBool>, ImapIdleOptions)`
(`src/rfc2177/idle.rs`). Details:

- It declares its own yield type, `ImapIdleYield::{WantsRead, WantsWrite, Event}`
  (`idle.rs:130-139`), so it falls outside the `ImapClient` blanket trait
  (the trait bounds on `run` are `Yield = ImapYield`). Deliberate, per the crate
  docs: the five coroutines with their own yield vocabulary — watch, idle,
  streamed APPEND and the two streamed FETCHes — are the ones implementations are
  expected to wire differently.
- `ImapIdleEvent { untagged: Vec<StatusBody>, data: Vec<Data> }` (`idle.rs:120-127`)
  hands back the **raw** untagged responses, so `EXISTS`, `EXPUNGE`, `FETCH` and
  `VANISHED` all reach us undigested. Good — we want to trigger a QRESYNC pull,
  not consume a pre-chewed diff.
- Wind-down is a shared `Arc<AtomicBool>`; flip it and the coroutine sends `DONE`
  and completes cleanly.
- Refresh interval: 29 s by default (`IDLE_DEFAULT_TIMEOUT`, `idle.rs:78`) —
  "survives NAT middle-boxes and stays well under the 29-minute RFC 2177 §3 cap".
  **0.6.0 makes this configurable** via `ImapMailboxWatchOptions::idle_timeout`;
  29 s is ~120 round trips/hour/mailbox, which is more than iCloud needs and more
  than a laptop on battery wants. Tune it.
- `ImapClientStd` exposes IDLE only indirectly through `watch_mailbox`
  (`src/client.rs:839`); there is no bare `idle()`. Since we are on tokio and
  `ImapClientStd` is blocking, this is moot — see below.

### Async: we implement the pump, and it is short

`ImapClientStd` is blocking. Postio is tokio. `io-imap` anticipates this:
implement `client::ImapClientAsync`'s single `run` method over our own transport
and the forty-odd commands come with it. The upstream reference is
`examples/tokio_session.rs:203-243` — the entire implementation is a ~40-line
`loop { match coroutine.resume(..) }` over `tokio::io` plus `tokio-rustls`.
`examples/tokio_watch.rs` shows the IDLE/watch wiring on tokio, and
`examples/tokio_fetch_stream.rs` the streamed body fetch.

This is a feature, not a tax: it means `postio-imap` owns the socket, the TLS
stack and the timeouts, and `io-imap` contributes only protocol reasoning. It also
means **no `pimalaya-stream` dependency and no io-imap TLS feature** — build with
`default-features = false` and the `client` feature only, or with no features at
all if we drive coroutines directly. Fewer moving parts under a crate that
reshuffles its client layer every fortnight.

---

## iCloud-specific hazards found

Not blockers, but they must be encoded in `postio-imap` rather than discovered in
production.

**1. iCloud omits the untagged `* ENABLED` response.** Per Apple Developer Forums
thread 694251, `ENABLE CONDSTORE QRESYNC` against `imap.mail.me.com` historically
returned only `<tag> OK ENABLE completed`, with no `* ENABLED CONDSTORE QRESYNC`
line — a violation of RFC 5161 §3.1. Reportedly corrected around server revision
2204B190 (Dec 2021), but do not rely on it.

`io-imap` survives this correctly: `ImapExtensionEnable`'s return type is
`Result<Option<Vec<CapabilityEnable>>, _>` and a missing `Data::Enabled` yields
`Ok(None)`, not an error (`src/rfc5161/enable.rs:110-131`). `watch.rs` likewise
only logs the value. **Our rule: treat a `None`/empty ENABLED echo as success.
Never gate QRESYNC on the echo — gate it on the post-auth CAPABILITY list.**

**2. iCloud has shipped malformed FETCH sequence numbers under QRESYNC.** The same
thread reports `SELECT (QRESYNC …)` producing FETCH responses with sequence number
`-1`. `imap-types` models a sequence number as `NonZeroU32`, so such a line cannot
be decoded.

`io-imap`'s framing is forgiving here, which is good and bad. `send.rs:217-231`
skips any *untagged* line it cannot decode rather than failing the command, with
only a `debug!("skipping undecodable untagged response")` and a `trace!` of the
bytes (added for pimalaya/himalaya#641). So a malformed line is **silently
dropped** — the command succeeds with missing data. **Enable `log` at `debug` for
the `io_imap` target in dev builds, and treat a skip during a QRESYNC pull as a
signal to fall back to a full resync.** Consider a bead for a resync-integrity
counter.

**3. Untagged `VANISHED` without `EARLIER` is dropped by SELECT/EXAMINE.**
`select.rs:181-187` matches `Data::Vanished { earlier, .. } if earlier`. A
real-time `* VANISHED` arriving during a selected session is not surfaced there —
it arrives through the IDLE event stream instead (as raw `Data`), which is where
we should read it.

**4. Unverified here.** The exact post-auth CAPABILITY string of
`imap.mail.me.com` was **not** confirmed in this spike — a live banner probe was
attempted and blocked by the sandbox, and connecting with credentials was out of
scope by instruction. Published reports and the parent bead agree that CONDSTORE,
ENABLE, QRESYNC, UIDPLUS, IDLE, ID, NAMESPACE, UNSELECT, MOVE, SORT, THREAD and
ESEARCH are all present post-auth. **Confirm this once, at the top of E4.2, with a
`#[ignore]`d live test that prints the capability list** — it is a five-minute
check and it is the last remaining assumption.

---

## Decision

**GO on `io-imap`.** No evaluation of `async-imap` is required; the trigger
condition in the bead ("if it fails") did not occur.

Rationale in one paragraph: every extension E5 depends on — ENABLE, SELECT/EXAMINE
`(CONDSTORE)` and `(QRESYNC …)`, `VANISHED (EARLIER)` decoding, HIGHESTMODSEQ,
per-message MODSEQ, `FETCH CHANGEDSINCE`, `STATUS HIGHESTMODSEQ`, IDLE, UIDPLUS,
MOVE — is reachable through the public API, verified in source. The crate's
sans-I/O coroutine design means we own the socket and the runtime, which both
suits our tokio architecture and shrinks our exposure to the layer that churns
most (the client/transport layer). The two genuine gaps (no `UNCHANGEDSINCE` on
STORE; FETCH discards `VANISHED`) have clean workarounds and neither touches the
critical path.

The one real risk is **velocity, not capability**: six minor releases in eleven
weeks, with 0.5.0 breaking nearly every public signature. Mitigation is already an
architectural invariant — `postio-sync` talks to `MailBackend`, never to
`io-imap` types — and this ADR promotes it from convention to hard requirement.

### Pin

```toml
# workspace Cargo.toml
io-imap = { version = "=0.6.0", default-features = false, features = ["client"] }
```

- Exact `=` pin. A version bump is a reviewed change with its own bead, never a
  drive-by `cargo update`.
- `default-features = false` drops `rustls-ring` + `scram`, and with them
  `pimalaya-stream`, `url`, `anyhow` and `rand`. We bring tokio + tokio-rustls
  ourselves.
- Re-export `imap-codec` / `imap-types` through `io_imap::codec` and
  `io_imap::types`. **Never add them as direct dependencies** — they are
  `2.0.0-alpha.*` and io-imap version-locks them on purpose.
- Same discipline for `io-smtp` (himalaya-tui is on 0.3) when E4 reaches SMTP.

### Binding rules for `postio-imap`

1. Capabilities come from `ImapSessionOpen` / `ImapClientStd::connect` only. Any
   directly-constructed auth coroutine sets `ensure_capabilities: true`. An empty
   post-auth capability list is an error, never a silent downgrade.
2. Gate QRESYNC on the post-auth CAPABILITY list, never on the `* ENABLED` echo.
3. Expunges come from `SELECT`/`EXAMINE (QRESYNC …)`.`vanished_earlier`, never
   from `FETCH … (VANISHED)`.
4. Do not use `watch::ImapMailboxWatch`. Build the loop from ENABLE / SELECT /
   IDLE / SELECT(QRESYNC) primitives, using `watch.rs` as a reference.
5. Log `io_imap` at `debug` in dev builds; a skipped undecodable untagged response
   during a resync forces a full resync.
6. No `io-imap` type crosses the `MailBackend` boundary.

### Follow-up beads to file

- `io-imap`: no `UNCHANGEDSINCE` on STORE — decide `raw()` workaround vs upstream PR.
- E4.2: `#[ignore]`d live test printing iCloud's post-auth CAPABILITY; assert
  QRESYNC / CONDSTORE / IDLE / UIDPLUS present.
- E5: resync-integrity counter — a skipped undecodable untagged line triggers full resync.
- Track `io-imap` releases; a bump is a reviewed bead, not a `cargo update`.

### Spike hygiene

No spike code was written into the repository or the cargo workspace. The 0.6.0
tarball and its unpacked tree live only in the session scratchpad
(`/tmp/claude-1000/.../scratchpad/`) and need no cleanup. Acceptance criterion
"spike code deleted or clearly quarantined" is satisfied by there being none.

---

## Sources

- [io-imap on crates.io](https://crates.io/crates/io-imap) — version/date history
- [pimalaya/io-imap](https://github.com/pimalaya/io-imap) — `CHANGELOG.md` 0.3.0–0.6.0
- [icloud imap access using QRESYNC and CONDSTORE — Apple Developer Forums 694251](https://developer.apple.com/forums/thread/694251)
- [RFC 7162 — CONDSTORE / QRESYNC](https://www.rfc-editor.org/rfc/rfc7162.html)
- [MailKit #970 — iCloud SELECT command issue](https://github.com/jstedfast/MailKit/issues/970)
- Local source: `io-imap` 0.5.0 and 0.6.0, `imap-types` 2.0.0-alpha.7,
  `himalaya-tui` @ `beed4ed`
