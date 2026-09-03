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

**Reviewed:** 2026-09-03, over `postio-model`'s `mime` module — the parser
every other crate's view of a message comes through. Backed by
`crates/postio-model/tests/rfc2045.rs`, one test per row.

Parsing is `mail-parser` 0.11.8, which was the latest published version at the
time of the review; the mapping into the domain model is Postio's. Where a row
says the fix is upstream, that is why.

### Verdicts

| § | What it says | Verdict |
|---|---|---|
| **2045 §5.1** Content-Type parameters | Quoted values, `;` inside quotes | **Compliant.** A boundary containing a semicolon and a space round-trips, which is the case a splitter that splits before it looks at quotes gets wrong. |
| **2045 §5.2** Default type | Absent `Content-Type` is `text/plain; charset=us-ascii` | **Compliant**, for the message and for a part inside a multipart alike. |
| **2231** Parameter continuations | `name*0*=`/`name*1*=`, charset-tagged values | **Compliant.** Continuations reassemble and percent-decode against the declared charset; the corpus already carries the three spellings a non-ASCII filename arrives in. |
| **2045 §6.7** quoted-printable | Soft line breaks, undecodable sequences | **Compliant.** A soft break joins with nothing between; `=ZZ` survives as itself, which is what "show what arrived" means. |
| **2045 §6.8** base64 | Illegal characters, bad padding | **Compliant, with a named exception:** an undecodable payload degrades to its own raw text and sets `encoding_problems`. The degradation is right — raw text beats an empty body — but see the gap below on where that flag goes. |
| **2045 §6.2, §6.8** `8bit` / `binary` | Not encoded | **Compliant.** Both arrive as their own bytes. |
| **2045 §6.4** unknown `Content-Transfer-Encoding` | Treat as `application/octet-stream` | **Compliant, with a named exception:** shown verbatim as text instead, and **not** flagged. Recorded on [#901](https://github.com/dlapiduz/postio/issues/901) as the second half of that fix. |
| **2046 §5.1.1** Delimiter placement | `CRLF "--" boundary` — the delimiter begins a line | **Gap — [#899](https://github.com/dlapiduz/postio/issues/899).** `--boundary` **anywhere on a line** ends the part, so a body quoting one mid-line is silently truncated. |
| **2046 §5.1.1** Unusable boundary | Missing or unrecognisable ⇒ treat the entity as `text/plain` | **Gap — [#900](https://github.com/dlapiduz/postio/issues/900).** The body is lost and the container is offered as an attachment. |
| **2046 §5.1.1** Preamble and epilogue | Not part of any body | **Compliant.** `multipart-alternative.eml` carries both so it cannot regress unnoticed. |
| **2046 §5.1** Transport padding | Whitespace after a delimiter is not part of it | **Compliant.** |
| **2046 §5.1** Nesting | Arbitrary depth | **Compliant.** Forty levels deep neither panics nor loses the leaf — the correctness half of the surface #147's fuzzer treats as adversarial. |
| **2046 §4.1.2 / 2049** Charset versus bytes | A declared charset that does not match | **Compliant, with a named exception:** mojibake rather than an error, and silent. See below. |

### The gaps

Three, and the third is what makes the first two feel worse than they are.

**A `--boundary` mid-line truncates the body ([#899](https://github.com/dlapiduz/postio/issues/899)).**
RFC 2046 puts the delimiter at the start of a line; the parser matches the
two-dash sequence wherever it falls. `one\r\nnot --SEP here\r\ntwo` keeps
`one\r\nnot `. Real boundaries are long random tokens, so this is not an
everyday message — but the mail that embeds *another* message's delimiters is a
bounce, a digest, a forwarded raw source, or mail about MIME, which is to say
mail somebody is reading because something already went wrong. The fix is
upstream in `mail-parser`; there is no newer version to move to.

**An unusable boundary loses the body and offers the container as an
attachment ([#900](https://github.com/dlapiduz/postio/issues/900)).** RFC 2046
says a multipart whose boundary is missing or unrecognisable must be treated as
`text/plain`. Postio produces no body and one part typed `multipart/mixed` —
not a file, no name, opens in nothing. Of the three spellings (absent, empty,
never appears) the last is the one that will actually arrive: it is a message
that was well-formed when it left and met a gateway that rewrote the header and
not the body. `multipart-boundary-never-appears.eml` is the fixture. This one
is Postio's to fix, not the parser's: it is a fallback decision, and
`parse_inner` can see the case exactly.

**`encoding_problems` is computed and read by nothing
([#901](https://github.com/dlapiduz/postio/issues/901)).** Two occurrences in
the whole workspace — the field, and the line that sets it. `into_message` does
not carry it, so there is no column, no event and no surface. It is the only
signal Postio has that a message did not decode cleanly, and it is what every
correct-but-lossy degradation above depends on: the base64 case hands a person
`aGVsbG8g*d29ybGQ` with nothing to say those were meant to be words. Same
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
