# Feature Specification: Message authenticity — source, verification and signatures

**Feature Branch**: `feature/message-authenticity`

**Created**: 2026-11-22

**Status**: Roadmap draft. Not scheduled; no `tasks.md` until the maintainer
schedules it and answers the questions under "Clarifications"

**Input**: The maintainer, 2026-11-22: "I want to create roadmap specs/docs
for: PGP or some other signature validation mechanism, including a way to see
the original message and the verification for dkim/etc". This spec also takes
in issue #11, "PGP signature verification badge", and supersedes it (see
"What this spec supersedes").

## Context

Postio already does three things on this ground. This spec builds on them and
replaces none of them:

| What exists on `main` | Where |
|---|---|
| **One quiet warning line** in the reader's header when the account's provider stamped an `Authentication-Results` saying DMARC failed (or SPF and DKIM both failed for the From domain), or when the display name is itself another address or domain. Ordinary mail gets nothing, and there is **no "verified" badge** | #1928; `postio_ui::reader::authenticity`; `postio_session::reading::sender_warning`; `docs/PRODUCT.md` §21 |
| **Only the provider's header is believed.** The topmost `Authentication-Results` whose authserv-id is in the provider's preset row (`authserv_ids` in `providers.toml`) counts; everything below it was written by whoever sent the mail. With no such header, nothing is known and nothing is shown | same |
| **`V` shows the raw source** in a read-only dialog: every byte as the server sent it, shown as text and never interpreted. The bytes are read from this machine, or fetched from the account's own server on that press and no other | spec 007 FR-033; `postio-gtk/src/source.rs`; `Client::raw_source`; the terminal's `V` |

What is missing:

- the provider's verdict is reduced to one line or nothing. A person who wants
  to know *why* — which DKIM signature passed, for which domain, whether a
  mailing list re-signed it, what the provider did not check — has to read raw
  headers;
- the raw source is one undifferentiated block of text. Headers are not
  separated from the body, folded lines are not unfolded, encoded words are
  not decoded, and a 20 MB attachment is 27 MB of base64 to scroll past;
- an OpenPGP or S/MIME signature is a meaningless attachment. Postio cannot
  say whether it is good, bad or uncheckable;
- an encrypted message shows nothing useful.

`PRODUCT.md` §21 and §23 list PGP and S/MIME as unscheduled. This spec
schedules their **verification** half on the roadmap. It leaves decryption and
sending signed or encrypted mail to a later spec (D9).

### What this spec inherits

- **Constitution VI**: nothing leaves this machine that the user did not ask
  for. Every network question in this spec is answered against that rule:
  DKIM keys (D6), key discovery (D7) and certificate revocation (D8).
- **ADR 0042**: the reading renderer cannot run script or reach the network.
  The source surface is held to the same rule and does more: it never hands
  message bytes to the renderer at all (D3).
- **ADR 0017**: raw source is an evictable cache, fetched on demand.
  Verification needs the exact bytes, so this spec says when they are fetched
  (D5).
- **ADR 0048**: per-contact settings are the one home of sender-keyed
  settings. A key the user accepts for a correspondent is one of them (D7).
- **ADR 0045 and ADR 0049**: behaviour lives in `postio-focus` and the rule
  crates, and the frontends draw it. Each layer asserts its own question.
- **Spec 007**: the one dialog pattern, the one close, `Esc` back to the same
  row, and the Filtered view's digit tabs (`screens.md`, "Interaction rules").
- **Constitution II**: every new verb is a registry command with a key, a
  command-bar row and an accessible action.

### What this spec supersedes

- **Issue #11, "PGP signature verification badge"**, is folded in and
  superseded. Its two acceptance criteria are FR-020 and FR-021. Its "badge"
  is replaced by D4's rule. Its investigation comment (the Pimalaya survey,
  the key-source table and the evicted-bytes trap) is the starting point of
  `plan.md`'s research. The issue stays open until this spec lands, and then
  closes.

## Decisions this spec takes

These are design calls an agent can make. Each is reviewable. The calls only
the maintainer can make are under "Clarifications".

**D1. One surface, three tabs: Verification, Headers, Raw.** `V`
(`view_source`, whose id is a file format and stays) opens the **Source**
surface over the list in the one dialog pattern, on its **Verification** tab.
`1`, `2` and `3` switch tabs, as they do in the Filtered view, and `Esc`
closes back to the same row. The surface replaces today's "Raw source" dialog,
whose content becomes the Raw tab. The command's title becomes "Source and
verification". There is no second top-level key: the person asked for "the
original message and the verification" in one place, and one key that always
opens one surface on the same tab is predictable.

**D2. The word is "source", not "original".** "View original" (`mod+o`) means
the message as it painted itself rather than reader view (spec 006 FR-031).
Using "original" for the raw message as well would give one word two
meanings. The surface is "Source", its tabs are "Verification", "Headers" and
"Raw", and copy never says "original message".

**D3. The source surface is text, never rendered.** Raw bytes and header
values are drawn as plain text in the app's own font:

- no message byte reaches the reading renderer, a web view or any markup
  parser;
- a URL in the source is characters, not a link;
- no image, font or part is resolved;
- invalid UTF-8 becomes the replacement character, as it does today.

So the surface cannot run script or reach the network, by construction and
not by a setting. A test holds it: the corpus's hostile fixtures open in all
three tabs with zero connections and zero renderer calls.

**D4. Still no "verified" badge, and the header line stays a warning line.**
`PRODUCT.md` §21's rule holds:

- a passing SPF, DKIM or DMARC result draws nothing in the reader's header;
- a good signature draws nothing in the header either, unless the key belongs
  to the user's own decision (Q1's default, B): the user accepted that key
  for that address. Then the header carries one neutral fact line, "Signed
  with the key you accepted for ada@example.com". That line reports something
  the person established. It is not Postio vouching for anyone;
- a bad signature, meaning the message changed after it was signed, joins
  #1928's warning line: "The signature on this message does not match its
  content." A good signature from a key that is not the accepted one is
  a warning too: "Signed with a different key than the one you accepted for
  ada@example.com."

The header carries at most one line, so the parts join as #1928's do. The
Verification tab is where every positive fact is stated, in words that say
what it proves and what it does not (D10).

**D5. Exact bytes are kept for signed mail, and fetched from the user's own
server when they are not here.** ADR 0017 keeps raw source only as a cache.
This spec changes one thing in that rule. When the body stage sees that a
message's top-level type is `multipart/signed`, `multipart/encrypted`,
`application/pkcs7-mime` or `application/pkcs7-signature`, it keeps the raw
source. Such messages are a small fraction of a mailbox. That raw source is
still evictable, and it is evicted last among raw sources. When the bytes are
not on this machine, opening the message is a request for it, so the bytes
are fetched from the account's own server. That server already holds the
message, and no third party learns anything. Offline, the state is "The
signed message isn't fully on this computer yet; Postio will check the
signature when it is." That state is distinct from "cannot be checked"
(FR-023). It is the trap #11's investigation named.

**D6. Postio does not verify DKIM on this computer by itself.** DKIM needs a
DNS lookup of the signer's key. That lookup is a network request on open, and
it is a beacon:

- a sender can give every recipient their own selector, and see from their
  own DNS who opened the mail, and when;
- DKIM keys rotate, so mail older than the rotation fails a check made today,
  which would draw a false alarm on old mail.

The provider checked DKIM when the mail arrived, which is the moment the
check means something, and the Verification tab shows that result. Whether a
person may ask Postio to check one message again is Q2.

**D7. Keys come from what is already on this machine, and no key is fetched
unless the person asks.** The first slice takes a public key from three
sources, none of which reaches the network:

- an **Autocrypt** header on mail already downloaded;
- a key **attached to a message** (`application/pgp-keys`), imported by a
  command;
- a **file** the person imports.

A key is never trusted by being present. Each key records where it came from
and when it was first seen. The person may **accept** a key for an address.
That acceptance is a per-contact setting (ADR 0048), undoable, and the only
thing D4's neutral line reports. Looking a key up on the network (WKD, a
keyserver) is Q3.

**D8. S/MIME is in scope for verification, after OpenPGP, and behind a
spike.**

- The certificate chain is checked against the trust anchors already on this
  machine: the system store, read from disk.
- Revocation (OCSP, CRL) is a network request, so it is never automatic. The
  Verification tab says "Revocation not checked".
- S/MIME waits on a spike (plan, S2) that proves certificate-path validation
  for email certificates in memory-safe code. If the spike fails, S/MIME
  verification is filed as its own spec, and this spec ships OpenPGP alone.

**D9. Decryption and sending signed or encrypted mail are out of this spec.**
Each needs things verification does not:

- a **secret key** held in the OS keyring, and a way to unlock it;
- a rule for **where decrypted text lives**: in the store, where it is
  encrypted at rest and searchable, or nowhere;
- a rule for **replies**: a reply to encrypted mail must not quote it in the
  clear, and a draft of one must not sync to the server's Drafts in the clear;
- a composer surface for signing and encrypting (`specs/015-native-composer`
  is that surface's future).

Pimalaya's own rule is the one to keep when that spec is written: "reading
degrades, writing refuses". A message the person asked to encrypt never goes
out in the clear. Until then:

- an encrypted message opens on a clear state: "This message is encrypted.
  Postio can't decrypt mail yet." Its Source surface works;
- its key and recipients, where the packet says them, show on the
  Verification tab.

**D10. Every verdict is said in words that claim no more than it proves.**
The Verification tab is not a checklist of green ticks. Each line says who
checked, what passed or failed, and for which domain. Under all of it, a fixed
line says what none of it means: "These checks are about the sending domain
and the message's integrity. They don't tell you the person is who you
think." The vocabulary is in "The Verification tab".

**D11. BIMI and sender logos are rejected.** A BIMI logo is a remote image
and a "verified" badge in disguise. Postio fetches neither.

## Clarifications

### Open questions for the maintainer

- **Q1. What may the reader's header say about a good signature?** D4 takes B
  as its default.
  - **A: nothing.** Good signatures appear only on the Verification tab. This
    is the strictest reading of "no verified badge".
  - **B: a neutral fact line only for a key the person accepted** for that
    address ("Signed with the key you accepted for …"). Nothing for any other
    key. This is the spec's default.
  - **C: a "Signed" mark for any valid signature**, as issue #11 drew it.
    That mark is a badge in all but name: a key that arrived in the same
    message proves nothing about the person.
- **Q2. May a person ask Postio to check one message's DKIM again, from this
  computer?** (D6)
  - **A: never.** The provider's result at arrival is the only DKIM result
    Postio shows. This is the spec's default for every slice.
  - **B: on an explicit per-message action**, "Check DKIM from this
    computer". It makes one DNS lookup, recorded in the egress log as a new
    `dns` subsystem. The button says that the sender can see the lookup. It
    is never automatic and never a setting.
  - **C: B, plus showing the body-hash check that needs no DNS.** This is
    rejected in the plan as misleading, and listed only for completeness.
- **Q3. Where may a public key come from beyond what is on this machine?**
  (D7)
  - **A: nowhere.** The only sources are Autocrypt, an attached key and an
    imported file.
  - **B: a lookup the person asks for**, one key at a time: "Look up a key
    for ada@example.com". It tries WKD at the sender's own domain, then the
    one keyserver named in `config.toml` (none by default). It is
    egress-logged, and the action says who learns of the lookup. This is the
    spec's recommendation, for a later slice.
  - **C: reading the user's GnuPG keyring** (`~/.gnupg`). That is a local
    file read, with no network. It brings existing keys and their
    web-of-trust signatures, at the cost of depending on GnuPG's on-disk
    format or binary. It can be combined with A or B.

## The Source surface

The surface is the one dialog over the list, with the one close. It has a
header that reads "Source · <subject>" and a tab strip that reads
"Verification 1 · Headers 2 · Raw 3".

### The Verification tab

The tab has five sections, in this order. A section with nothing to say is
left out, except section 5.

1. **What your provider checked.** This is the trusted `Authentication-Results`
   (the topmost one whose authserv-id is the provider's, as today).
   - One line per method, in plain words:
     - "SPF passed for bounce.example.com, which is not the From domain";
     - "DKIM: 2 signatures. example.com passed and matches the From domain.
       esp.example passed";
     - "DMARC passed for example.com (policy: reject)";
     - "ARC: a forwarder, lists.example.org, sealed the results it saw. Your
       provider says the chain is intact".
   - Alignment with the From domain is stated for each result, because it is
     what makes a pass mean anything.
   - The provider is named by its preset's display name, and the authserv-id
     is shown in mono.
2. **Headers Postio does not believe.** These are the other
   `Authentication-Results` and `ARC-Authentication-Results` headers. They are
   drawn quieter, under "Written before your provider received the message.
   Anyone could have written these."
3. **Signature.** For OpenPGP or S/MIME, this section gives:
   - the status: Good, Bad, Can't check, or Not here yet;
   - the signer's key or certificate: fingerprint in mono, user ids or
     subject, where the key came from and when it was first seen;
   - whether the person accepted this key for the From address;
   - the actions: "Accept this key for ada@example.com", and "Import key"
     when the key is attached.
4. **What Postio checked on this computer.** This section lists:
   - the display-name check (#1928), with its result;
   - the signature check, when there was one;
   - "DKIM, SPF and DMARC were not checked on this computer. Your provider
     checked them when the mail arrived" (D6);
   - for S/MIME, "Revocation not checked".
5. **What this does not tell you.** D10's fixed line.

### The Headers tab

- Every header, in the order the message carries it, one row each. The name
  is in mono, and the value is unfolded with its encoded words decoded. A
  value that decoding changed shows its raw form beneath it on demand.
- The `Received` chain is drawn as hops, newest first, each with its time and
  the delay since the hop before.
- **A boundary is drawn** at the provider's trusted `Authentication-Results`.
  Headers above it were written by the provider. Headers below it came with
  the message, and the line says so.
- `ctrl+f` finds within the tab, as it does in a message.

### The Raw tab

- Every byte, as text, as today's dialog shows it.
- A part over 64 KiB is folded to one line, "base64 · 3.2 MB · report.pdf —
  Show". The bytes are all there, and folding is only how they are drawn.
- "Copy all" and "Save as .eml…" act on every byte, folded or not. Save goes
  through the existing export path.
- `ctrl+f` finds within the tab.

### Every state

| State | Verification | Headers | Raw |
|---|---|---|---|
| Body not downloaded yet (partial) | "The headers that carry these checks arrive with the message body." The tab names the sync state | The envelope's headers, marked "more arrive with the body" | "Fetching the message from <account>…", or offline: "This message isn't on this computer yet. Postio will show it when <account> is back online." |
| No trusted provider header (for example, a provider with no `authserv_ids`) | "Your provider didn't add a result Postio can trust." Section 2 still lists what the message carries | unchanged | unchanged |
| Encrypted (D9) | the recipients' key ids where the packet names them; "Postio can't decrypt mail yet" | unchanged | unchanged |
| Signed, bytes not here, offline (D5) | "Not here yet", never "Can't check" | unchanged | as partial |
| Sent by the user | "You sent this." Nothing is judged, as #1928 judges nothing either | unchanged | unchanged |
| Narrow (1024 px) | sections stack, and fingerprints wrap at group boundaries | the name column narrows to its widest name up to 30% | unchanged |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See why the provider believed or doubted a sender (Priority: P1)

A person gets an invoice from billing@example.com and wants to know whether
it really came from that domain before they pay it. They press `V` and read
the provider's checks in plain words, with alignment stated, and they see
which headers Postio does not believe.

**Why this priority**: It is the maintainer's "verification for dkim/etc". It
needs no cryptography and no network, and it makes #1928's one line
explainable.

**Independent Test**: Open each corpus fixture with Authentication-Results
(`auth-results-verified`, `auth-results-forged-lower`, a new ARC fixture and a
new two-signature DKIM fixture), press `V`, and assert on the words the tab
shows. Assert zero connections.

**Acceptance Scenarios**:

1. **Given** a message whose trusted header says `dmarc=pass` with
   `header.from=example.com`, **When** the person presses `V`, **Then** the
   Verification tab reads "DMARC passed for example.com", names the provider,
   and the reader's header still shows nothing.
2. **Given** a message carrying a forged `Authentication-Results` below the
   provider's, **When** the tab opens, **Then** the forged one appears only
   under "Headers Postio does not believe", and does not change section 1.
3. **Given** a mailing-list message whose DKIM passed only for the list's
   domain, **When** the tab opens, **Then** it says the passing signature is
   lists.example.org's, which is not the From domain, and it describes the
   ARC chain.
4. **Given** an account whose provider has no `authserv_ids`, **When** the tab
   opens, **Then** it says no trusted result exists, and lists the message's
   own headers as not believed.
5. **Given** the Source surface on any tab, **When** the person presses `Esc`,
   **Then** it closes to the same list row, with the cursor where it was.

---

### User Story 2 - Read the headers and the source without wading through them (Priority: P1)

A person debugging a delivery problem wants the `Received` hops and a header's
decoded value. Another wants to save the message as `.eml` to send to their
provider's abuse desk.

**Why this priority**: It is the maintainer's "a way to see the original
message", and it turns today's one text block into something readable. It
needs nothing new from the network.

**Independent Test**: For a fixture with folded and RFC 2047 headers and a
large attachment, assert that the Headers tab shows each header unfolded and
decoded, that the trust boundary sits at the right row, that the Raw tab folds
the attachment, and that "Save as .eml" writes bytes identical to the stored
raw source.

**Acceptance Scenarios**:

1. **Given** a message with five `Received` headers, **When** the person
   presses `V` then `2`, **Then** five hops appear newest first, with times
   and delays.
2. **Given** a 10 MB attachment, **When** the person presses `3`, **Then** the
   tab opens within the interaction budget with the part folded, and "Show"
   unfolds it.
3. **Given** the Raw tab, **When** the person runs "Save as .eml…", **Then**
   the file is byte-identical to the message's raw source.
4. **Given** the corpus's hostile HTML fixtures, **When** each opens in every
   tab, **Then** no connection is made, no renderer is invoked, and no URL is
   activatable.
5. **Given** a message whose raw source is not on this machine and the
   account offline, **When** the person presses `3`, **Then** the tab says so,
   and the Headers and Verification tabs still show what the stored headers
   allow.

---

### User Story 3 - Know whether an OpenPGP signature is good (Priority: P2)

A person corresponds with a maintainer of an open-source project who signs
every mail with OpenPGP. Postio tells them the signature is good, bad, or
can't be checked, and why. Once they have accepted that key, Postio tells
them when a message is signed with a different one.

**Why this priority**: It is issue #11, and the maintainer's first example.
It needs a key source (D7) and the exact bytes (D5), so it follows US1 and
US2.

**Independent Test**: Over new corpus fixtures (a good signature with its key
in an Autocrypt header, the same message tampered, a signature with no key on
this machine, a signature by a second key for the same address), assert each
status on the Verification tab and in the reader's header line.

**Acceptance Scenarios**:

1. **Given** a signed message and the signer's key from an earlier Autocrypt
   header, **When** the person opens it, **Then** the reader's header shows
   nothing (Q1 = B, key not accepted), and the Verification tab says "Good
   signature by key 1A2B … from ada@example.com's Autocrypt header, first
   seen 3 March".
2. **Given** the same message with one body byte changed, **When** it opens,
   **Then** the header shows "The signature on this message does not match
   its content."
3. **Given** a signature whose key is not on this machine, **When** it opens,
   **Then** the tab says "Can't check: no key for this signature is on this
   computer", offers "Import key" when one is attached, and never says
   "Good".
4. **Given** the person accepted key A for ada@example.com, **When** a message
   from that address arrives signed by key B, **Then** the header warns
   "Signed with a different key than the one you accepted for
   ada@example.com."
5. **Given** the person accepted a key, **When** they press `z`, **Then** the
   acceptance is undone.
6. **Given** a signed message whose raw source was evicted, and the account
   offline, **When** it opens, **Then** the status is "Not here yet", never
   "Can't check" or "Bad".

---

### User Story 4 - Know whether an S/MIME signature is good (Priority: P3)

A person at an organisation that signs mail with S/MIME certificates sees
whether a signature is good, and what chain it rests on.

**Why this priority**: It is less common than OpenPGP among Postio's users,
and it waits on the spike (D8).

**Independent Test**: Over S/MIME fixtures (a good signature with a chain to
a test anchor, a tampered one, an expired certificate, and an unknown
anchor), assert each status.

**Acceptance Scenarios**:

1. **Given** a message signed with a certificate that chains to a trust
   anchor on this machine, **When** the Verification tab opens, **Then** it
   reads "Good signature. Certificate for ada@example.com, issued by <CA>.
   Revocation not checked."
2. **Given** a certificate that chains to no anchor here, **When** the tab
   opens, **Then** it reads "Can't check: the certificate's issuer isn't
   trusted on this computer", with the chain shown.
3. **Given** a certificate whose email address is not the From address,
   **When** it opens, **Then** the header warns, as a key mismatch does.

---

### User Story 5 - Manage the few keys verification needs (Priority: P3)

A person sees which keys Postio knows for a correspondent, where each came
from, and accepts or forgets one. They do this from the contact's card and
from the Verification tab. There is no separate key manager.

**Why this priority**: It is the minimum key management #11 allowed ("no key
management beyond what verification needs").

**Independent Test**: Import a key file. Assert that it appears on the
contact's card with its source. Accept it, forget it, and assert each change
and its undo.

**Acceptance Scenarios**:

1. **Given** a `.asc` file, **When** the person runs "Import key…", **Then**
   each key in it is listed under the contacts whose addresses its user ids
   name. A user id that names no contact creates an unsaved contact (spec
   011 H2).
2. **Given** a key, **When** the person runs "Forget this key", **Then**
   signatures by it read "Can't check", and `z` restores it.

---

### Edge Cases

- **Several `Authentication-Results` from the provider** (some providers stamp
  twice): the topmost one is believed, as today. The tab names the count.
- **A `multipart/signed` nested inside `multipart/mixed`**, as a forwarder
  produces: the signature covers only that part, so the tab says "Part of this
  message is signed" and names the part. No header line appears.
- **Inline PGP** (`-----BEGIN PGP SIGNED MESSAGE-----` in a text part): it is
  verified the same way. The tab says it is inline, and that inline
  signatures cover only that text.
- **A signature made before its key's creation time, or after its
  expiry**: "Bad", with the reason in words.
- **A key that is revoked by a revocation signature already on this
  machine**: "Revoked key". Revocations are never fetched.
- **Weak algorithms** (SHA-1 signatures, RSA under 2048 bits): "Can't check:
  this signature uses SHA-1, which is no longer safe". It never reads "Good".
- **Autocrypt key change**: a new key in a later Autocrypt header is recorded
  beside the old one. It changes nothing in the header line unless the old
  key was accepted, and then D4's mismatch warning applies.
- **A message with an Autocrypt header that is not signed**: the key is
  recorded, and nothing is shown.
- **Very large raw source** (over 50 MB): the Raw tab opens on its folded form
  within budget. Copy all asks first, because it puts 50 MB on the
  clipboard.
- **The signature part itself** (`signature.asc`, `smime.p7s`): it is not
  listed as an attachment. The signature status stands for it.
- **A terminal, or the Mac**: the same three tabs. The terminal draws them as
  text frames, and every state and string comes from one view model.

## Requirements *(mandatory)*

### Functional Requirements

**The Source surface**

- **FR-001**: `V` (`view_source`) MUST open the Source surface over the list,
  in the one dialog pattern, on its Verification tab. `1`, `2` and `3` MUST
  switch to Verification, Headers and Raw, and `Esc` MUST close it to the same
  row with the cursor unchanged.
- **FR-002**: Every new action (switch tab, copy all, save as `.eml`, import
  key, accept key, forget key) MUST be a registry command with a key, a
  command-bar row and an accessible action. The command bar MUST offer each
  only where it applies.
- **FR-003**: The Source surface MUST NOT hand any byte of the message to the
  reading renderer, a web view or a markup parser. A URL MUST NOT be
  activatable. Opening any tab MUST make no network connection, except the
  fetch FR-004 allows.
- **FR-004**: When the raw source is not on this machine, the Raw tab MUST
  fetch it from the account's own server once, on the person's request, and
  MUST show the fetch's state. Offline, it MUST say so, and the other tabs
  MUST show what the stored headers allow.
- **FR-005**: The Headers tab MUST show every header in the message's order,
  unfolded and decoded, with the raw form available. It MUST draw the
  `Received` chain as hops, and MUST mark the boundary at the provider's
  trusted `Authentication-Results`.
- **FR-006**: The Raw tab MUST fold any part over 64 KiB to one line, and MUST
  let the person unfold it. "Copy all" and "Save as .eml…" MUST act on every
  byte, and the saved file MUST be byte-identical to the stored raw source.
- **FR-007**: The Source surface MUST be offered by every interface that
  offers `view_source` (GTK, terminal, macOS), from one view model, with the
  same strings and states.

**The Verification tab**

- **FR-010**: The tab MUST state, in words, each method result in the
  provider's trusted `Authentication-Results` (SPF, DKIM per signature,
  DMARC with its policy where stated, ARC). Each MUST state the domain it is
  about and whether that domain aligns with the From domain.
- **FR-011**: The tab MUST list every other `Authentication-Results` and
  `ARC-Authentication-Results` header as not believed, and these MUST NOT
  change any verdict.
- **FR-012**: The tab MUST say what Postio checked on this computer and what
  it did not, including that DKIM, SPF and DMARC were checked by the provider
  at arrival and not here (D6).
- **FR-013**: The tab MUST end with the fixed line of D10.
- **FR-014**: The reader's header MUST NOT show a positive mark for any
  SPF, DKIM, DMARC or ARC result (`PRODUCT.md` §21).
- **FR-015**: #1928's rule for the warning line MUST stay unchanged. The tab
  explains it and does not alter it.

**Signatures**

- **FR-020**: Valid, invalid and uncheckable signatures MUST be distinct, in
  words and not only in colour, on the Verification tab and in the reader's
  header wherever D4 lets the header speak (issue #11, first criterion).
- **FR-021**: A signature that cannot be checked MUST never be described as
  good (issue #11, second criterion). That covers no key, a weak algorithm,
  an unsupported format, or bytes not here.
- **FR-022**: OpenPGP signatures MUST be verified on this computer, over the
  exact signed bytes. That covers PGP/MIME (RFC 3156) and inline signatures,
  with RFC 9580 and RFC 4880 keys.
- **FR-023**: When the exact bytes are not on this machine, the status MUST
  be "Not here yet". It MUST be resolved when the bytes arrive, without the
  person doing anything more than having opened the message.
- **FR-024**: The body stage MUST keep the raw source of messages whose
  top-level type is signed or encrypted (D5). Eviction MUST take those raw
  sources last among raw sources.
- **FR-025**: The reader's header MUST warn on a bad signature, and on a good
  signature from a key or certificate other than the one the person accepted
  for that address. It MUST say nothing for other good signatures, unless Q1
  is answered C.
- **FR-026**: S/MIME signatures MUST be verified on this computer against the
  trust anchors on this machine, with no revocation check. The tab MUST say
  "Revocation not checked" (D8). This depends on the spike.
- **FR-027**: An encrypted message MUST open on the D9 state, never on a blank
  body. Its Source surface MUST work.
- **FR-028**: A signature part MUST NOT be listed as an attachment.

**Keys**

- **FR-030**: Postio MUST record public keys from Autocrypt headers on mail
  already downloaded, from attached keys the person imports, and from files
  the person imports. Each key MUST record its source and when it was first
  and last seen.
- **FR-031**: Postio MUST NOT fetch a key, certificate or revocation from the
  network unless Q3 is answered B. Then a fetch happens only on the person's
  explicit action for one address, and each fetch is recorded in the egress
  log.
- **FR-032**: Accepting a key for an address MUST be a per-contact setting
  (ADR 0048), and MUST be undoable. A key MUST NOT become accepted by any
  other means.
- **FR-033**: Keys MUST be shown on the contact's card and on the
  Verification tab. This spec MUST NOT add a separate key-manager surface.

**Privacy and performance**

- **FR-040**: Logs MUST carry no addresses, user ids, fingerprints, header
  values or message content from this feature: ids, counts and outcomes only.
- **FR-041**: Opening the Source surface on a message whose bytes are local
  MUST stay within the interaction budget. Its reads MUST be gated as counts:
  a bounded number of statements and no full-mailbox scan.
- **FR-042**: Verification MUST never run on the UI path. A message MUST be
  listable, openable and actionable before its signature is checked.
- **FR-043**: This feature MUST NOT make a DNS lookup unless Q2 is answered
  B. Then a lookup happens only on the person's explicit action for
  one message, is recorded in the egress log, and is announced as visible to
  the sender.

### Key Entities

- **Authentication report**: what one `Authentication-Results` header says.
  It holds the authserv-id, whether it is trusted, and its method results.
  Each result holds the method, the result, the domain it is about, and its
  alignment with From.
- **Signature verdict**: the format (OpenPGP, inline OpenPGP, S/MIME), the
  status (good, bad, can't check, not here yet, revoked), the reason, the
  signer's key or certificate, the signed scope (whole message or one part),
  and the time it was checked.
- **Public key**: the fingerprint, user ids, the addresses they name, the
  source (Autocrypt, attachment, file, or lookup if Q3 = B), first seen, last
  seen, and revocation state.
- **Accepted key**: a per-contact setting holding one fingerprint for one
  address.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For every corpus message that carries an Authentication-Results
  header, the Verification tab states each method's result and its domain,
  and the statements match a hand-written expectation for 100% of them.
- **SC-002**: Opening any tab of the Source surface on any corpus message
  makes zero network connections and zero renderer calls, apart from the raw
  fetch FR-004 allows.
- **SC-003**: Across the signature fixtures, no message that cannot be
  checked is ever shown as good: zero false "Good".
- **SC-004**: A tampered signed message is reported as bad in 100% of the
  tampering fixtures (one header byte in the signed part, one body byte, a
  re-encoded transfer encoding).
- **SC-005**: The Source surface opens within the interaction budget for a
  message whose bytes are local, including one with a 25 MB attachment. That
  is gated as counts.
- **SC-006**: A person can find out why #1928's warning line appeared in two
  keystrokes from the open message (`V`, reading the first section).
- **SC-007**: The reader's header shows no new line for 100% of corpus
  messages that are ordinary mail: no signature, and no failed or doubted
  sender.

## Assumptions

- The provider-trust rule of #1928 (the topmost header from `authserv_ids`) is
  right, and is not reopened here. A user who runs their own mail server adds
  its authserv-id in their own `providers.toml`, as they can today.
- No new search operator is added. A later spec may add `is:signed` or
  `is:encrypted`, under constitution III's rules for an operator.
- The GTK app, the terminal and the Mac each adopt the surface in the same
  slice, from one view model. A frontend that cannot draw a part yet lists it
  as debt in its command-coverage sweep, as today.
- Autocrypt is used only as a key source. Postio does not send Autocrypt
  headers until the outgoing spec (D9) does.
- The corpus gains fixtures through `/add-fixture`: a real signature over
  synthetic content, made with a test key generated for the corpus. No real
  person's key or mail is used.
