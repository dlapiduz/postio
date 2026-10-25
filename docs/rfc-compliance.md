# What Postio actually implements, against what the RFCs say

Postio picked up RFC compliance the way most mail clients do: one standard at
a time, each added when a specific bug or feature made it necessary. That is a
reasonable way to get here and it is exactly the shape that leaves gaps nobody
went looking for. This document is the deliberate pass — #462 and its three
sub-issues (#680 MIME, #681 IMAP, #682 SMTP).

## How to read a row

Every claim below is one of three things, and never silence:

| Verdict | What it means |
|---|---|
| **Compliant** | Postio does what the RFC says, and a test says so. |
| **Compliant, with a named exception** | Postio deviates on purpose. The row says what the deviation is, why, and what it costs. A named exception is a decision; an unnamed one is a bug that has not been found yet. |
| **Gap** | Postio does not do what the RFC says, and the difference can reach a person. Every gap here has its own issue — the row links it. |

A row that says "compliant" without a test behind it is a wish. The RFC 5322
section is backed by `crates/postio-model/tests/rfc5322.rs`, one test per
claim, so a verdict that stops being true fails a run rather than quietly
becoming a wrong sentence in a file nobody re-reads.

**Scope.** The standards Postio's own surface actually touches, not email
standards at large. Postio speaks IMAP and SMTP to servers, parses RFC 5322
with `mail-parser` and generates it with `mail-builder`; it does not implement
a transfer agent, a DKIM signer, or a spam filter, and this document says
nothing about those.

---

## RFC 5322 — Internet Message Format (and RFC 6854)

**Reviewed:** 2026-09-02, against `postio-model`'s `mime`, `outgoing`,
`address`, `ids` and `message` modules. #461 (a `Message-ID` reserved once per
send attempt series and stored on the draft, rather than minted per build) had
landed and is what was audited; #673 had not, and nothing here depends on it.

### Verdicts

| § | What it says | Verdict |
|---|---|---|
| **2.1** Line endings | Every line ends CRLF | **Compliant.** Nothing generated carries a bare CR or LF. |
| **2.1.1** Line length | 998 octets MUST, 78 SHOULD | **Compliant.** Forty recipients with long display names and a 26-deep `References` chain both fold below 78. |
| **2.2** Field bodies | No CR or LF in a field body except folding | **Compliant.** A line break in an unstructured value folds to a single space on the way into the generator — which is what unfolding a legitimately folded header produces — so nothing after it can become a header. A `Message-ID` carrying one is dropped instead of folded, since a space inside `<...>` makes a different invalid id rather than a valid one. Was a gap ([#864](https://github.com/dlapiduz/postio/issues/864)); see below for what it cost and why folding rather than refusing. |
| **2.2.3** Folding | Long fields fold; a reader unfolds them | **Compliant.** Received chains and folded subjects unfold to one line with each fold as a single space, in the parsed value *and* in the raw header block the reader shows under "view source". |
| **3.3** Date | `date-time` with a zone | **Compliant.** UTC, `+0000`, from `mail-builder`. |
| **3.4** Address specification | `mailbox` / `group`, US-ASCII only | **Compliant.** A display name needing quotes round-trips; non-ASCII names and subjects are RFC 2047 encoded, and the generated header block is pure ASCII. |
| **3.4** Groups, and **RFC 6854** groups in `From:` | A group is a named list of mailboxes | **Compliant, with a named exception:** groups are flattened to their members and the group *name* is not modelled. See below. |
| **3.6.4** `Message-ID` | Globally unique, `id-left@id-right` | **Compliant.** Unique across a run, `dot-atom@sending-domain`, no hostname. See below on the uniqueness argument. |
| **3.6.4** `In-Reply-To` / `References` | Reply carries the parent's id; `References` is the parent's `References` plus the parent's id | **Compliant.** Both asserted by round-trip, including that the parent's id is last in the chain. |
| **3.6.2** `Reply-To` | Present when the author suggests a different address | **Compliant, with a named exception:** written on every message, even when it equals `From`. See below. |
| **RFC 2047** encoded words in headers | Adjacent encoded words join without the whitespace between them | **Compliant.** Both the folded and the same-line spelling decode without an inserted space, and ordinary text between two encoded words keeps its spaces. |

### The fix: a line break in a header value ([#864](https://github.com/dlapiduz/postio/issues/864))

**What it was.** A value reaching `postio_model::outgoing` with a CR or LF in
it was written into the message verbatim, so everything after the break became
an additional *header*. Not a theoretical shape: replying to the corpus'
`encoded-word-crlf-in-header.eml` generated a message with a **`Bcc` header the
draft never set** — a copy of the reply going somewhere the sender did not
choose, which the composer's own recipient chips could not show, because it was
never in `draft.to`, `draft.cc` or `draft.bcc`.

**How it was reachable.** Not through the composer, whose subject field is a
single-line entry. Through received mail: an RFC 2047 encoded word encodes
arbitrary octets, CR and LF included, so a `Subject` arrives whose *decoded*
value has real line breaks in it, because those octets were never folding
whitespace, and `reply` then copied that subject into a draft. The parser now
folds a decoded subject and display name to one line before anything draws or
quotes it ([#1770](https://github.com/dlapiduz/postio/issues/1770): the same
break drew a forged `Bcc:` line under the subject in the reader), but the
generator does not rely on that: the same shape reaches a draft from a pasted
subject, its stored row, a `mailto:` URL, a rules action, and the FFI and MCP
surfaces.

**Folded, not refused, and that is a decision.** Refusing reads as the safer
answer and is the wrong one: the hostile value arrives from somebody else's
message, so refusing would mean a message you cannot reply to — handing the
sender a veto over your mail client. A single space is what unfolding a
legitimately folded header produces (§2.2.3), so a value that only ever
contained folding whitespace is unchanged in meaning, and a hostile one
becomes visible text in the subject rather than a header. A run of breaks and
the whitespace around them collapses to one space; a break at either end adds
nothing.

**In one place.** `outgoing::assemble` applies it to every unstructured value
handed to the builder, rather than at each caller — sanitising in the composer
would have left every other source open, and those are the ones nobody thinks
of.

**`Message-ID` is dropped rather than folded.** It is structured, so a space
inside `<...>` makes a different invalid id, not a valid one; and one hostile
id reaches both `In-Reply-To` and `References`. Dropping loses nothing, since
a threading header naming a malformed id threads nothing. This one is not
reachable through `mime::parse` today — a `Message-ID` header with a literal
break ends at the break — and is defended anyway, because the parser is not
the only way a `Message` is built.

### The named exceptions, and what each costs

**Groups are flattened; the group name is dropped from the model.** `Board:
ada@example.com, grace@example.net;` becomes two addresses with no display
names, in `From:` (RFC 6854) exactly as anywhere else. Postio shows and replies
to people, and a group is not somebody you can send to. What it costs: the
reading pane cannot say "to the Board", and a reply-all to a group message
addresses the members individually — which is what actually happens on the
wire anyway, since the group name is not a deliverable address. The name is
still in the raw header block, so nothing is lost, only unmodelled. An empty
group — `undisclosed-recipients:;`, which is what a bcc-only message carries —
yields no recipients and keeps its header, which is the honest answer rather
than an invented address.

**`Reply-To` is written on every outgoing message, even when it equals
`From`.** §3.6.2 makes the field optional and means it as a signal: its
presence says the author suggests replying elsewhere. Postio writes
`identity.effective_reply_to()` unconditionally, which for an identity with no
reply-to of its own is the `From` address again. Not a violation — a
`Reply-To` equal to `From` is legal and changes nothing about where a reply
goes — but it is noise on every message, and it spends a signal some list
managers and clients read. Cheap to change; nobody has needed to.

**Comments in an address are folded into the display name.** `ada (Ada
Lovelace) <ada@example.com>` comes back with the name `ada (Ada Lovelace)`
rather than `ada`, and a comment inside the angle brackets is appended to the
name instead. §3.4's CFWS says a comment is not part of the display name.
Inherited from `mail-parser`, cosmetic in the reading pane, and it affects only
mail that uses obsolete comment syntax — which is rare enough that no fixture
had it before this audit. Recorded rather than filed: the cost is a slightly
wrong name on a rare message, and the fix is upstream.

**`postio_model::address::parse_list` is not an RFC 5322 parser and does not
claim to be.** It parses what a person types into a composer field, on every
keystroke, where the text is mid-edit far more often than it is finished — so
it is deliberately forgiving: `,` and `;` both separate, an unterminated
`<` is accepted, a half-typed address is kept rather than dropped. Received
mail never goes through it; that is `mime::addresses`, which is `mail-parser`.
The two are separate on purpose and conflating them would make the composer
reject text a person is still typing.

### On `Message-ID` uniqueness

§3.6.4 asks for globally unique, and "globally" is the part that is easy to
claim and hard to hold. Postio's id is `make_boundary(".")@<sending domain>`:
three hex components from `mail-builder`'s own boundary generator, under the
identity's domain, with `FALLBACK_DOMAIN` when the identity has none.

Deliberately **not** the id `mail-builder` would generate on its own, which
carries this machine's hostname — `docs/PRODUCT.md` §21's "nothing leaves this
machine that the user did not ask for" makes a header on every outgoing message
the wrong place for it. The domain component is what carries the global part of the
uniqueness argument: two Postio installations sending as the same domain would
have to collide within that domain's namespace, and the local part is 48 hex
digits of which two components vary per call.

64 ids in one run are distinct, which is what the test checks. It does not
check across processes, and cannot: that is an argument about the generator's
seeding, not a property a test can observe. Under ADR 0021 the id is reserved
once per send attempt series and stored on the draft (#461), so a retried send
reuses its id rather than minting a second one — which is the property that
actually matters for a mail client, because two ids for one message is how a
recipient ends up with two copies of it.

### What was checked and found unremarkable

Stated so the next reader knows these were looked at rather than skipped:
`Date` generation (UTC, correct form); the terminating blank line and the
absence of bare LF in generated bytes; `Message-ID` normalization (angle
brackets always, case-insensitive equality, so a chain rewritten in transit
still matches); `References` entries that cannot identify a message (an empty
`<>`, a bare token, an unterminated angle-addr) being dropped rather than
carried as edges the threading pass can only match against other garbage.

### Routed to the sub-issues

Two things surfaced here that belong to another section, recorded so they are
not found twice:

- **The generated message does not end with CRLF.** Its last body line has
  none. Whether that matters is an SMTP question, not a 5322 one: the `DATA`
  terminator is `CRLF "." CRLF`, and a correct sender appends the CRLF the
  body lacks before it. Postio hands the bytes to `io-smtp`'s `SmtpData`,
  which owns dot-stuffing and the terminator. **#682 confirmed both** against
  the wire — the missing CRLF is supplied before the terminator, and a
  bare-dot body line (which `postio-model` really does produce) is stuffed.
  See the RFC 5321 section.
- **`Content-Transfer-Encoding` selection and `format=flowed`.** The generator
  declares `text/plain; charset=utf-8; format=flowed` unconditionally (ADR
  0017, #333) and lets `mail-builder` choose the encoding. That is **#680**'s.

---

## RFC 2045–2049 — MIME

**Reviewed:** 2026-09-03, over `postio-model`'s `mime` module — the parser
every other crate's view of a message comes through. Backed by
`crates/postio-model/tests/rfc2045.rs`, one test per row.

Parsing is `mail-parser`, reviewed against 0.11.8 and moved to 1.0.0 on
2026-10-24 (the mapping into the domain model is Postio's). Where a row says
the fix is upstream, that is why; 1.0 closed the delimiter-placement gap
([#899](https://github.com/dlapiduz/postio/issues/899)) and reads around a
damaged base64 or quoted-printable body instead of handing it back encoded.

### Verdicts

| § | What it says | Verdict |
|---|---|---|
| **2045 §5.1** Content-Type parameters | Quoted values, `;` inside quotes | **Compliant.** A boundary containing a semicolon and a space round-trips, which is the case a splitter that splits before it looks at quotes gets wrong. |
| **2045 §5.2** Default type | Absent `Content-Type` is `text/plain; charset=us-ascii` | **Compliant**, for the message and for a part inside a multipart alike. |
| **2231** Parameter continuations | `name*0*=`/`name*1*=`, charset-tagged values | **Compliant.** Continuations reassemble and percent-decode against the declared charset; the corpus already carries the three spellings a non-ASCII filename arrives in. |
| **2045 §6.7** quoted-printable | Soft line breaks, undecodable sequences | **Compliant.** A soft break joins with nothing between; `=ZZ` survives as itself, which is what "show what arrived" means. |
| **2045 §6.8** base64 | Illegal characters, bad padding | **Compliant, with a named exception:** a damaged payload (bytes outside the alphabet, a bad final quantum) is decoded around the damage, as a reader would, and sets `encoding_problems`. The degradation is right — the words arrive — but see the gap below on where that flag goes. |
| **2045 §6.2, §6.8** `8bit` / `binary` | Not encoded | **Compliant.** Both arrive as their own bytes. |
| **2045 §6.4** unknown `Content-Transfer-Encoding` | Treat as `application/octet-stream` | **Compliant, with a named exception:** shown verbatim as text instead, and **not** flagged. Recorded on [#901](https://github.com/dlapiduz/postio/issues/901) as the second half of that fix. |
| **2046 §5.1.1** Delimiter placement | `CRLF "--" boundary` — the delimiter begins a line | **Compliant** since `mail-parser` 1.0 ([#899](https://github.com/dlapiduz/postio/issues/899)). `--boundary` ends a part only at the start of a line, so a body quoting one mid-line is kept whole. |
| **2046 §5.1.1** Unusable boundary | Missing or unrecognisable ⇒ treat the entity as `text/plain` | **Compliant.** All three spellings (absent, empty, never appears) fall back to the entity's own content as the text body; the container is never offered as an attachment. Was a gap ([#900](https://github.com/dlapiduz/postio/issues/900)). |
| **2046 §5.1.1** Preamble and epilogue | Not part of any body | **Compliant.** `multipart-alternative.eml` carries both so it cannot regress unnoticed. |
| **2046 §5.1** Transport padding | Whitespace after a delimiter is not part of it | **Compliant.** |
| **2046 §5.1** Nesting | Arbitrary depth | **Compliant.** Forty levels deep neither panics nor loses the leaf — the correctness half of the surface #147's fuzzer treats as adversarial. |
| **2046 §4.1.2 / 2049** Charset versus bytes | A declared charset that does not match | **Compliant, with a named exception:** mojibake rather than an error, and silent. See below. |

### The gaps

One.

**`encoding_problems` is computed and read by nothing
([#901](https://github.com/dlapiduz/postio/issues/901)).** Two occurrences in
the whole workspace — the field, and the line that sets it. `into_message` does
not carry it, so there is no column, no event and no surface. It is the only
signal Postio has that a message did not decode cleanly, and it is what every
correct-but-lossy degradation above depends on: a base64 body with a stray
byte is read around it, and the person gets words with nothing to say a byte
was skipped. Same
failure the reading pane already learned once in #70 — *"nothing rendered"* and
*"nothing was there"* look identical and are opposite facts. Same shape as
#327, #416 and #421 one step down, and #421's own check cannot see it: that
counts `pub fn`, and this is a `pub` field.

### The named exception worth reading twice

**A mislabelled charset produces mojibake, silently, and that is the right
answer.** UTF-8 bytes labelled `us-ascii` come back as `Ã©tÃ©`; Latin-1 bytes
labelled `utf-8` come back with replacement characters; a charset nothing has
heard of falls through as UTF-8. None of them errors and none of them is empty,
which is the property that matters: an empty reading pane is the one answer
worse than wrong characters, because it cannot be told from a message that had
nothing in it. The corpus says the same thing from the other side —
`charset-windows-1252-mislabeled.eml` exists because *"the mislabelling every
real client silently forgives"* is the ordinary case rather than the exception.

What it costs is that Postio cannot tell the user their message is being shown
in the wrong encoding. That is the same surface #901 is about, and if that flag
ever grows a home this is the second thing to report through it.

### Routed elsewhere

- **`format=flowed` is declared unconditionally on outgoing text** (ADR 0017,
  #333) and `mail-builder` chooses the transfer encoding. Correct per RFC 3676
  because `postio_body::render` is the only path that fills `draft.body.text`,
  but it is a claim about a caller rather than about the bytes — worth
  restating here rather than leaving it in the RFC 5322 section where the
  generator was audited.
- **The body renderer** (`postio-body`) turns a decoded body into what the
  reading pane draws. Nothing in it decides a MIME question — by the time it
  runs, the charset and the transfer encoding are already resolved — so it is
  out of scope for this section and belongs with the reader's own surfaces.

## RFC 3501 / RFC 9051 — IMAP, and the extensions in use

**Reviewed:** 2026-10-16, against `postio-account`'s `imap` module
(`selection`, `mutate`, `fetch`, `dispatch`, `skip_counter`) and
`postio-sync`'s `resync`, with the RFC 6851, 7162 and 2919 text read rather
than recalled. RFC 4315 (UIDPLUS) is its own section below. This closes
[#681](https://github.com/dlapiduz/postio/issues/681).

Postio never sends `ENABLE` and never issues `SELECT (QRESYNC …)`:
`Dispatch::extensions_to_enable` and `resync_strategy`
(`dispatch.rs:144`, `:197`) are policy with no caller in production code.
What it uses is `SELECT (CONDSTORE)` and `UID FETCH … (CHANGEDSINCE n)`
(`selection.rs:251`, `resync.rs:572`), and it finds deletions by counting
rather than from `VANISHED` — ADR 0001 found `io-imap` drops it. Every QRESYNC
row below is therefore about what the *absence* of QRESYNC costs.

Backed by loopback tests in `crates/postio-account/tests/` against an
in-process server, and by `MockBackend` in `crates/postio-sync/tests/sync_suite/resync.rs`.

### RFC 3501 / 9051 — base protocol

| § | What it says | Verdict |
|---|---|---|
| **2.3.1.1** `UIDVALIDITY` | A changed value invalidates every cached UID | **Compliant, with a named exception.** A change is acted on only after the server says it twice (`selection.rs:369`), because a live server contradicted itself from one `SELECT` to the next (#1538). The cost is one extra `SELECT` on the path that was about to discard a mailbox. A real renumber still wipes and rebuilds (`resync.rs:492`); `uidvalidity.rs` and `a_uid_validity_change_wipes_and_rebuilds_the_mailbox` assert both halves. |
| **6.3.1** `SELECT` response | `UIDVALIDITY` `MUST` be present | **Compliant.** An omission is asked again, then refused as permanent — or transient when `io-imap` dropped the line (`selection.rs:297-350`; `a_select_that_omits_uidvalidity_is_asked_again`, `a_uidvalidity_lost_in_transit_is_transient`). |
| **6.3.1 / 6.3.2** `SELECT` vs `EXAMINE` | `EXAMINE` is read-only and cannot serve a write | **Compliant.** A cached selection is reused only if it is read-write (`selection.rs:232`), so a write never rides an `EXAMINE`. |
| **6.4.5** `FETCH` and `\Seen` | Fetching `BODY[…]` sets `\Seen`; `BODY.PEEK` does not | **Compliant.** Every body, header and section fetch is `BODY.PEEK` (`body.rs:292,415`, `fetch.rs:290`); reading in the background never marks mail read. |
| **7.4.2** Unsolicited `FETCH` | A server may report a flag change on any message at any time | **Compliant.** An entry with neither `INTERNALDATE` nor `ENVELOPE` is passed over, not treated as a malformed answer (`fetch.rs:368`; `an_unsolicited_flag_update_inside_the_reply_is_not_a_message_that_lost_its_date`). |
| **7.4.2** Undecodable untagged line | (not a client rule) `io-imap` drops a line it cannot decode and completes `Ok` | **Compliant, with a named exception.** Counted around every `CHANGEDSINCE` fetch and `SELECT`; a nonzero count refuses the answer and rebuilds (`skip_counter.rs`; `a_line_io_imap_could_not_decode_forces_a_full_resync_not_a_silent_success`). |
| **2.3.2** `\Recent` | Only the server sets it | **Was a latent gap, fixed here.** Nothing in the tree asked for it, but `wire_flags` would have put a `\Recent` on a `STORE` or `APPEND` (`mutate.rs:397`); it is now dropped, asserted by `recent_is_never_put_on_the_wire`. |
| **6.3.11** `APPEND` literal | A synchronising literal waits for `+` | **Compliant.** `non_sync: false` (`mutate.rs:227`): the server may refuse a message before all of it is sent, and `LITERAL+` is not assumed. |
| **6.4.8 / 9051 §6.4.9** `n:*` | A range ending in `*` always includes the highest UID, even if it is below `n` | **Compliant, with a named exception.** The arrivals fetch `floor:*` (`resync.rs:575-580`, `unaccounted_arrivals`) can therefore return one message already held; it is upserted again and counted as "changed". Costs a spare row, never a wrong one. Not fixed because the extra row cannot alter local state. |
| **6.4.3 / 6.4.8** Silent `UID` misses | A `UID` command naming a UID that is gone succeeds and does nothing | **Compliant.** `COPY`/`STORE`/`UID EXPUNGE` on a vanished UID are no-ops, which is the right outcome for a message another client already moved or deleted. |

### RFC 7162 — CONDSTORE / QRESYNC

| § | What it says | Verdict |
|---|---|---|
| **3.1.2.1** `HIGHESTMODSEQ` | A client `MUST` cache it after a sync and delete it when the mailbox cannot supply one | **Compliant.** It is recorded only after a pass succeeds, never before (`resync.rs:386`; the error arms return without calling `observe`), dropped on a `UIDVALIDITY` change (`sync.rs:120`), and a missing value plans a full pass (`FullResyncReason::NoModSeq`, `sync.rs:97`). |
| **3.1.2.2** `NOMODSEQ` | A mailbox that cannot keep mod-sequences reports none; the client must not use `CHANGEDSINCE` | **Compliant.** The absence of `HIGHESTMODSEQ` on `SELECT` is the signal, and `ResyncPlan::Full(NoModSeq)` re-reads the folder in place (`a_modseq_less_backend_resyncs_in_place_without_discarding_rows`). The `NOMODSEQ` code itself is never parsed; its meaning arrives as the missing value. |
| **3.1.2** Mod-sequences never decrease | A smaller `HIGHESTMODSEQ` than the one held means the store was rebuilt | **Compliant.** `ModSeqWentBackwards` plans a full pass (`sync.rs:106`). |
| **3.1.4.1** `CHANGEDSINCE` | Strictly greater than; implies the `MODSEQ` item; enables CONDSTORE | **Compliant.** Postio sends `SELECT (CONDSTORE)` first anyway (`selection.rs:293`) and the mock honours strictly-greater (`a_changedsince_fetch_returns_only_what_moved`). |
| **3.1.2.1** New messages | A new message's mod-sequence exceeds the previous `HIGHESTMODSEQ` | **Compliant, with a belt and braces.** A server that breaks this would lose new mail silently, so `UIDNEXT` is a second witness (`resync.rs:775`; `a_message_the_change_feed_never_mentions_still_arrives`). |
| **3.2** Expunge raises `HIGHESTMODSEQ` | Required of a server **that supports QRESYNC**; stated for no one else | **Was a gap, fixed here.** On a CONDSTORE-only server a message deleted by another client leaves the value unchanged, so the pass planned `UpToDate` and the message stayed in the list indefinitely. The `UpToDate` arm now compares local rows to `EXISTS` and reconciles when there are more (`resync.rs:430-460`). Asserted by `a_deletion_that_leaves_highestmodseq_alone_still_reflects_locally`, which uses the new `MockBackend::expunge_keeps_mod_seq`. |
| **3.2 / 5** `VANISHED (EARLIER)` and `SELECT (QRESYNC …)` | A QRESYNC client learns deletions from the server | **Not used, by design.** Deletions are found by the arithmetic in `resync.rs`'s module docs: exact, and costs one extra fetch only when the count disagrees. The price is that a mailbox whose count disagrees re-fetches every known UID. |
| **3.1.3** Conditional `STORE` | `UNCHANGEDSINCE` makes a flag write lose to a newer change | **Compliant, with a named exception.** Last writer wins (`mutate.rs:53`): `io-imap` cannot send the modifier (ADR 0001), and a flag lost to a race comes back on the next resync. |
| **3.2.3 / 3.2.5** `ENABLE QRESYNC` | Enabling changes `EXPUNGE` into `VANISHED` and adds `MODSEQ` to `FETCH` | **Compliant.** Never enabled, so no response shape differs from what the parser expects; asserted by `enable_is_not_sent_to_a_server_that_does_not_advertise_it` for the gate in case it is ever wired. |
| **Advertised but answered incompletely** | (not an RFC rule) | **Compliant.** A `CHANGEDSINCE` that lost a line is refused (`skip_counter.rs`) and the mailbox is re-enumerated without wiping it (`resync.rs:492`). |

### RFC 6851 — MOVE, and the fallback

| § | What it says | Verdict |
|---|---|---|
| **3.1** `UID MOVE` | Atomic on the server | **Compliant.** Used whenever `MOVE` is advertised (`mutate.rs:104`; `moving_uses_move_where_the_server_has_it`). A tagged `NO` surfaces as an error to the caller; §3.3 obliges the server to leave each message in at least one of the two mailboxes. |
| **3.3 / 4.3** `COPYUID` and `EXPUNGE`/`VANISHED` order | `COPYUID` arrives untagged, before the expunge responses | **Compliant.** `io-imap` collects the code from either position, and `mapping_from` pairs it positionally (`mutate.rs:339`). An absent code is [#903](https://github.com/dlapiduz/postio/issues/903), above. |
| **4.4** `MOVE` with QRESYNC | A QRESYNC client must handle both `VANISHED` and `EXPUNGE` | **Compliant.** QRESYNC is never enabled. |
| *fallback* `COPY` then `STORE \Deleted` then `UID EXPUNGE` | (not in 6851: this is the RFC 3501 spelling of a move) | **Compliant, with a named exception.** The copy comes first and a failure stops before anything is flagged (`mutate.rs:112`; `moving_without_move_copies_then_deletes`). `UID EXPUNGE` names only the moved UIDs, so another client's `\Deleted` mail is not taken. Without UIDPLUS the source is left flagged, not expunged — the named exception in the UIDPLUS section. A failure between copy and flag leaves the message in both mailboxes, which §3.3 forbids a server from doing but a client-side sequence cannot avoid; the cost is a duplicate, not a loss. |

### RFC 2047 and RFC 2919 as they arrive over IMAP

| § | What it says | Verdict |
|---|---|---|
| **RFC 2047 §5** Encoded words in `ENVELOPE` | `ENVELOPE` hands over header text undecoded | **Compliant.** Subject and display names go through `decode_header_text`; `mailbox` and `host` do not, since an encoded word cannot appear there (`fetch.rs:463,504-530`; `mime.rs` tests `decode_header_text_agrees_with_a_full_parse_of_the_same_subject`). A decoded line break is folded to a space (#1770). |
| **RFC 2919 §3** Grammar | `[phrase] "<" list-id ">"`; the phrase may be a quoted string | **Was a gap, fixed here.** The parser took everything between the first `<` and the last `>`, so a `<` inside a quoted phrase, or a second `List-Id` in the same `HEADER.FIELDS` block, produced a garbage identifier. It now takes the first bracket outside a quoted string (`mime.rs`, `first_angle_bracketed`; `list_id_from_text_reads_the_first_bracket_outside_a_quoted_phrase`). |
| **RFC 2919 §3** Phrase optional; brackets | A value with no brackets is not a valid `List-Id` | **Compliant, with a named exception.** A bare value is kept as the identifier (`list_id_from_text_keeps_the_bracketed_part_and_trims_the_rest`) because some senders omit the brackets and the list is still recognisable. |
| **RFC 2919 §6** Comparison | Case-insensitive; ignores everything outside the brackets | **Compliant for what compares.** Search is a case-insensitive full-text match on the stored identifier (`postio-index` `list:`), and the phrase is discarded at parse time. The identifier is stored as sent, and the unsubscribe record keys on the stored text, so `News.Example` and `news.example` are two lists to it; a list that changes its capitalisation between messages is vanishingly rare, and nothing in the tree compares two identifiers for equality. |
| **RFC 2919 §3** One field | "No more than one of each field" | **Not Postio's to enforce.** A violating message is read from its first field. |

### What was not reviewed

Literal handling inside `io-imap` itself (the `{n}` parse and the continuation
exchange) is the dependency's; Postio's own surface is the `APPEND` choice
above and the body streamer's handling of a literal that stops short
(`body.rs:256`). The Gmail, JMAP and Maildir backends speak none of this and
are outside the section.

## RFC 5321 — SMTP

**Reviewed:** 2026-09-03, against `postio-smtp`'s `session`, `error` and
`transport`, and the `postio-model` generator whose bytes they carry. #673 had
landed, which is what unblocked the response-code half — auditing the error
classification before ADR 0021's boundary existed would have documented a
classification that was about to change.

Backed one-test-per-row by `crates/postio-smtp/tests/rfc5321.rs`, which runs
every exchange against `io-smtp`'s sans-I/O transport with no socket involved.

### Verdicts

| § | What it says | Verdict |
|---|---|---|
| **§4.1.1.1, §4.1.3** `EHLO` identity | The client's FQDN, or an address literal when it has none | **Compliant.** A desktop behind a NAT has no FQDN of its own, so the greeting is `EHLO [127.0.0.1]`. It used to be the account's mail domain, which is the provider's name and not this machine's, or `localhost`, which Stalwart refuses with `550 5.5.0`. |
| **§4.1.1.4** `DATA` terminator | The payload ends `CRLF "." CRLF` | **Compliant.** The generator produces a message whose last body line has *no* CRLF; `io-smtp` supplies it before the terminator, so the dot begins its own line. Asserted against the bytes on the wire, not read from the crate. |
| **§4.5.2** Transparency | A line beginning `.` is stuffed to `..` | **Compliant.** `postio-model` really will produce a body line that is a bare `.`; it reaches the wire as `..`. Unstuffed it *is* the terminator, and the message would be delivered truncated with no error anywhere. |
| **§3.3, §4.1.1.2–3** Envelope versus headers | The reverse-path and forward-paths are the transaction's addresses, independent of the message's own headers | **Compliant.** Every `To`, `Cc` *and* `Bcc` address gets its own `RCPT TO`; the `DATA` bytes carry no `Bcc` header and not the bcc'd address. See below — this one is a privacy claim, so it earns more than a row. |
| **§4.5.3.1.6** Line length | 1000 octets including CRLF | **Compliant.** Nothing folds the body, so this holds because `mail-builder` picks a transfer encoding that bounds the line: a 1,200-character line and a 4,000-character unbroken word both come out quoted-printable under 78. |
| **§4.2.1** Reply codes | The first digit decides retryability | **Compliant.** `classify` branches on `code / 100 == 4` and nothing else; `smtp_session.rs` covers a rejection at each step — auth, sender, recipient, payload — and ADR 0021's "was the payload on the wire" question separately. |
| **§4.2.1** Multiline replies | A reply is one *or more* lines carrying the same code | **Compliant.** Every line reaches the reason, joined as one sentence: `io-smtp` keeps only the first (its rejections are built from `response.text()`), so `postio_smtp::reply` taps the bytes Postio already reads and re-parses them with that crate's own `SmtpResponse::parse`. The RFC 3463 code every line repeats is quoted once, and a line break can never reach a log. |
| **RFC 3463** Enhanced status codes | A supplementary `class.subject.detail` code | **Compliant, with a named exception:** parsed by nobody. The code reaches the reason as text — `450 4.2.1 …` and `550 5.2.1 …` both keep it — and the *basic* first digit is what decides retries, which is what §4.2.1 makes decisive. See below. |
| **RFC 1870** `SIZE` | Declare the message size if the server offers it | **Compliant.** Postio passes no ESMTP parameter on `MAIL FROM` or `RCPT TO` for an ordinary message, so it cannot rely on an extension the server did not advertise. Asserted by checking the commands carry no `SIZE=`, `BODY=`, `SMTPUTF8`, `RET=` or `ENVID=`. The capability list *is* now read, for the one case where relying on nothing is not available — see RFC 6531 below — and `SMTPUTF8` is the only keyword anything asks about. |
| **RFC 6152** `8BITMIME` | 8-bit content requires the server to advertise it | **Compliant.** The generator encodes anything needing 8 bits — a body of `Grüße, Grace — φ ≈ 1.618` goes out base64 — so no octet above 127 reaches a connection that never negotiated one. |
| **RFC 6531** `SMTPUTF8` | A non-ASCII address requires the server to advertise it | **Compliant.** The two halves get different answers, because they are different problems. A non-ASCII *domain* is punycoded (RFC 5891) before it reaches `MAIL FROM`/`RCPT TO` — a conversion, needing no extension, so `ada@例え.test` goes out as `ada@xn--r8jz45g.test` whatever the server offered. A non-ASCII *local part* cannot be encoded away, so it is sent only when the EHLO reply advertised `SMTPUTF8`, with the parameter §3.4 requires on `MAIL FROM`, and otherwise refused with an error naming the address. |

### The envelope/header split, and why it is a privacy verdict

`PRODUCT.md` §21 says a bcc'd address never travels inside the message. That is
two claims, and only both together are the feature:

- **It must not be in the headers.** `postio-model::outgoing` has two entry
  points rather than a boolean — `build` omits `Bcc`, `build_draft` includes it
  — because the wrong answer to that boolean discloses a bcc'd address to every
  other recipient. The Drafts copy needs the header (a draft picked up on a
  phone would otherwise silently send to fewer people); the sent bytes must not
  have it.
- **It must be in the envelope.** `Draft::all_recipients` chains `to`, `cc` and
  `bcc`, and that list is what the send path hands to `RCPT TO`. Drop it here
  and the message is simply not delivered to the person, with nothing to show
  for it.

Neither half was tested before this audit. The failure modes are opposite —
one discloses, one silently under-delivers — and each looks correct from the
other's side, which is why the row is a verdict rather than a reading of the
code.

### The named exception: enhanced status codes are carried, not parsed

RFC 3463 gives a server a second, finer code (`5.1.1` — "bad destination
mailbox address") beside the basic three-digit one. Postio does not parse it,
and that is deliberate rather than an oversight:

- §4.2.1 makes the **first digit of the basic code** the retry decision, and
  that is the decision Postio has to make. A `4.x.x` inside a `550` does not
  make the rejection temporary.
- The distinctions Postio acts on beyond retryability — is this an
  authentication failure, was the payload already on the wire — are answered by
  which `SmtpError` variant was built and by ADR 0021's boundary, not by a
  subject code.

What the exception costs: nothing today, because the enhanced code is preserved
verbatim in the reason a log or a person sees. It would start costing something
the day Postio wanted to act on a specific subject code — "mailbox full" versus
"no such user" are both `550` and differ only in the enhanced code — and that
day the parse is a small addition, not a redesign.

---

## Keeping this true

Add to a section rather than starting a new document. A verdict that changes
changes here and in `crates/postio-model/tests/rfc5322.rs` in the same commit —
the point of having both is that neither can drift alone.

---

## RFC 4315 — UIDPLUS

**Reviewed:** 2026-09-02, against `postio-account`'s `dispatch` and `mutate`, and
`postio-sync`'s `drain`. This is the first slice of #681; the other IMAP
RFCs are in the section above.

### Verdicts

| § | What it says | Verdict |
|---|---|---|
| **2.1** Removal without UIDPLUS | Fall back so a removal does not take other `\Deleted` messages with it | **Compliant, with a named exception.** With UIDPLUS the removal is `UID EXPUNGE`, which names its UIDs. Without it Postio does not expunge at all rather than doing the RFC's STORE-dance — safer, and it leaves the source copy behind flagged `\Deleted`. See below. |
| **2.1** `UID EXPUNGE` gating | Only meaningful when the capability is present | **Compliant.** `Dispatch::move_strategy` gates on it, asserted by `a_move_never_falls_back_to_an_untargeted_expunge`. |
| **3** `COPYUID` on a copy | A UIDPLUS server *SHOULD* return it; **MAY** omit it for a `UIDNOTSTICKY` mailbox, and **SHOULD NOT** send it without `SELECT`/`EXAMINE` rights | **Gap — [#903](https://github.com/dlapiduz/postio/issues/903).** An empty mapping from a UIDPLUS server is read as "the message is no longer in that mailbox on the server". See below. |
| **3** Absent response code | "If the server does not return the APPENDUID or COPYUID response codes, the client can discover this information by selecting the destination mailbox" | **Gap — [#903](https://github.com/dlapiduz/postio/issues/903).** Absence means the UIDs are unknown; Postio treats it as evidence about where the message *is*. |

### The named exception: a move without UIDPLUS leaves the source behind

`MoveStrategy::CopyThenDelete { uid_expunge: false }` copies, sets `\Deleted`,
and stops. The RFC suggests a client instead clear `\Deleted` from everything
it does *not* want removed, expunge, and put the flags back — a dance that is
correct and races every other client touching that mailbox.

Postio declines the dance. Nothing in the tree enqueues an untargeted
`EXPUNGE` after a move, so no message a person marked `\Deleted` elsewhere can
be removed by a move they made here. The cost is that the move is incomplete
on the server: the source copy remains, flagged, until something expunges that
mailbox. Another client will show the message in both places until then.

That is a deliberate trade — an incomplete move is recoverable and a
collaterally-expunged message is not.

### The gap: an absent COPYUID is not a missing message ([#903](https://github.com/dlapiduz/postio/issues/903))

`drain` reads an empty mapping from a UIDPLUS server as proof the source
message was already gone, settles the operation as obsolete, and records that
sentence as the reason. RFC 4315 §3 licenses neither half: the server is
allowed to omit the code for a `UIDNOTSTICKY` destination or one the account
cannot `SELECT`, and absence says only that the UIDs are unknown.

Two ordinary server configurations therefore produce a successful move that
Postio files as "already gone" — and, more importantly, a move that genuinely
failed looks identical, so it is dropped rather than retried.
