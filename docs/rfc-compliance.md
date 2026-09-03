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
| **2.2** Field bodies | No CR or LF in a field body except folding | **Gap — [#864](https://github.com/dlapiduz/postio/issues/864).** A header value reaching the generator with a line break in it is written verbatim, so the rest of that value becomes additional headers. See below. |
| **2.2.3** Folding | Long fields fold; a reader unfolds them | **Compliant.** Received chains and folded subjects unfold to one line with each fold as a single space, in the parsed value *and* in the raw header block the reader shows under "view source". |
| **3.3** Date | `date-time` with a zone | **Compliant.** UTC, `+0000`, from `mail-builder`. |
| **3.4** Address specification | `mailbox` / `group`, US-ASCII only | **Compliant.** A display name needing quotes round-trips; non-ASCII names and subjects are RFC 2047 encoded, and the generated header block is pure ASCII. |
| **3.4** Groups, and **RFC 6854** groups in `From:` | A group is a named list of mailboxes | **Compliant, with a named exception:** groups are flattened to their members and the group *name* is not modelled. See below. |
| **3.6.4** `Message-ID` | Globally unique, `id-left@id-right` | **Compliant.** Unique across a run, `dot-atom@sending-domain`, no hostname. See below on the uniqueness argument. |
| **3.6.4** `In-Reply-To` / `References` | Reply carries the parent's id; `References` is the parent's `References` plus the parent's id | **Compliant.** Both asserted by round-trip, including that the parent's id is last in the chain. |
| **3.6.2** `Reply-To` | Present when the author suggests a different address | **Compliant, with a named exception:** written on every message, even when it equals `From`. See below. |
| **RFC 2047** encoded words in headers | Adjacent encoded words join without the whitespace between them | **Compliant.** Both the folded and the same-line spelling decode without an inserted space, and ordinary text between two encoded words keeps its spaces. |

### The gap: a line break in a header value ([#864](https://github.com/dlapiduz/postio/issues/864))

The audit found one thing that can reach a person, and it is worth stating
precisely because the obvious route is closed and the real one is not.

A subject cannot acquire a line break from the composer — that field is a
single-line entry. It can acquire one from **received mail**: RFC 2047 encodes
arbitrary octets, CR and LF among them, so a `Subject` can arrive whose
*decoded* value contains real line breaks. That is not a parser bug. Unfolding
cannot remove them, because those octets were never folding whitespace, and
`mime::parse` promises to report what arrived. Reply copies the subject into a
draft; the draft is built; the text after the break is written as a header.

The consequence is not cosmetic: the injected line is a *header*, so a message
can acquire a recipient the sender never saw — one the composer's own
recipient chips could not show, because it was never in `draft.to`,
`draft.cc` or `draft.bcc`.

The fixture is `crates/postio-model/tests/corpus/encoded-word-crlf-in-header.eml`
and `rfc5322.rs` asserts the *input* half — that a decoded header value really
does arrive with line breaks in it. The outgoing half lands with the fix,
because a test that asserts the bug is worthless and a test that asserts the
fix cannot pass before it.

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
carries this machine's hostname — CLAUDE.md's "nothing leaves this machine that
the user did not ask for" makes a header on every outgoing message the wrong
place for it. The domain component is what carries the global part of the
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
  which owns dot-stuffing and the terminator. **#682 should confirm it**
  against a body whose last line has no CRLF and a body containing a line that
  is a bare `.` — both of which `postio-model` will happily produce.
- **`Content-Transfer-Encoding` selection and `format=flowed`.** The generator
  declares `text/plain; charset=utf-8; format=flowed` unconditionally (ADR
  0017, #333) and lets `mail-builder` choose the encoding. That is **#680**'s.

---

## RFC 2045–2049 — MIME

**Not yet reviewed.** [#680](https://github.com/dlapiduz/postio/issues/680).

Add verdicts in the table format above, covering content-type and
transfer-encoding correctness, multipart boundary handling, and charset
declaration versus the actual bytes. Two things are already waiting for this
section: the unconditional `format=flowed` declaration described above, and
`crates/postio-model/tests/corpus/`'s charset fixtures, which exist precisely
because mislabelled charsets are the ordinary case rather than the exception.

## RFC 3501 / RFC 9051 — IMAP, and the extensions in use

**Not yet reviewed.** [#681](https://github.com/dlapiduz/postio/issues/681).

Covers `io-imap`'s behaviour and Postio's own assumptions where they diverge
from spec, beyond what ADR 0001's spike already found for iCloud specifically.
The extensions already relied on: RFC 4315 UIDPLUS, RFC 7162
CONDSTORE/QRESYNC, RFC 6851 MOVE.

## RFC 5321 — SMTP

**Not yet reviewed.** [#682](https://github.com/dlapiduz/postio/issues/682).

Covers envelope versus header address handling and response-code handling
beyond the happy path — temporary versus permanent failure, which is #461's
retry question. The two concrete checks the 5322 pass routed here are in
"Routed to the sub-issues" above: the missing final CRLF, and a body line that
is a bare `.`.

---

## Keeping this true

Add to a section rather than starting a new document. A verdict that changes
changes here and in `crates/postio-model/tests/rfc5322.rs` in the same commit —
the point of having both is that neither can drift alone.

---

## RFC 4315 — UIDPLUS

**Reviewed:** 2026-09-02, against `postio-imap`'s `dispatch` and `mutate`, and
`postio-sync`'s `drain`. This is the first slice of #681; **CONDSTORE/QRESYNC
(7162), MOVE (6851), List-Id (2919) and the base protocol (3501/9051) are not
reviewed yet** and no verdict below should be read as covering them.

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
