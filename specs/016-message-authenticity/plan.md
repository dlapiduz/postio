# Implementation Plan: Message authenticity

**Branch**: `feature/message-authenticity` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: `specs/016-message-authenticity/spec.md`; issue #11 and its
investigation comment; `origin/main` at `17fff8da`.

The maintainer answered Q1 to Q3 on 2026-11-22 and scheduled the build. The
Phase 1 artifacts are [data-model.md](data-model.md),
[contracts/](contracts) and [quickstart.md](quickstart.md), and the queue is
[tasks.md](tasks.md). Phase 0's research is in this file (R1 to R8).

## Summary

Grow `V`'s raw-source dialog into one **Source** surface with three tabs:

- **Verification**: the provider's checks in words, the headers Postio does
  not believe, the signature, and what Postio checked itself;
- **Headers**: every header, decoded, with the `Received` hops and the trust
  boundary;
- **Raw**: every byte, with large parts folded.

Then verify **OpenPGP** signatures on this computer with **rPGP**. Keys come
from Autocrypt headers, attached keys, imported files and, when the person
turns it on, their **GnuPG keyring**, read in-process and read-only. A valid
signature puts the word **Signed** in the reader's header, and "Signed ·
accepted" for a key the person accepted. Then **S/MIME**, if a spike proves
memory-safe path validation. Postio never re-checks DKIM and never fetches a
key (the maintainer's Q2 = A and Q3 = C). Decryption and outgoing signing are
a later spec.

The first slice (A) needs no cryptography and no network. It is the Source
surface over what the store already has, and it answers the maintainer's
"see the original message and the verification for dkim/etc" on its own.

## Technical Context

**Language/Version**: Rust (workspace `rust-version` 1.100, edition 2024);
Swift for the Mac frontend.

**Primary Dependencies**:

- existing: `mail-parser`, which the workspace already has, for the header
  block and part offsets; `postio_ui::reader::authenticity`'s RFC 8601
  parser;
- new: `pgp` (rPGP, 0.21, MIT OR Apache-2.0) in slice B;
- new: RustCrypto `cms` and `x509-cert`, with a path validator chosen by
  spike S2, in slice C.

**Storage**: Turso store (ADR 0038). New tables `public_keys`,
`public_key_addresses`, `signature_verdicts` and `contact_accepted_keys`
(a child of the contact, ADR 0048); see [data-model.md](data-model.md). Raw source stays in the blob store, under a changed keep
rule (D5).

**Testing**:

- the rule crates' `--lib` tests over the corpus;
- `postio-session` suites for the verdict job;
- counting assertions for the surface's reads;
- `focus_suite` cases, one per path, and the terminal's and Mac's
  equivalents;
- a storyboard, `storyboards/reader/source-surface.toml`.

**Target Platform**: Linux (GTK and terminal), macOS through `postio-ffi`.

**Project Type**: a desktop application over a shared engine.

**Performance Goals**:

- the Source surface opens within the 16 ms interaction budget when the
  bytes are local;
- verification is off the UI path;
- the counts are gated (constitution V).

**Constraints**:

- no network beyond the user's own server for the user's own message (D5);
- no DNS, ever (D6, Q2 = A);
- no key fetch, ever (D7, Q3 = C);
- the GnuPG keyring read only when switched on, read-only, with no `gpg`
  process (D12);
- no revocation fetch (D8);
- logs carry ids, counts and outcomes only.

**Scale/Scope**: signed mail is a small fraction of a mailbox. The key table
is bounded by correspondents who sign: hundreds, not hundreds of thousands.

## Constitution Check

| Principle | How this plan meets it |
|---|---|
| I. Local-first | Accept, forget and import key are store writes with undo. Verification is a background job on the body stage. The surface reads local rows and never awaits the network, except the raw fetch the person asks for by opening the Raw tab, which is today's behaviour, drawn as a state |
| II. Keyboard is a system | Every verb is a registry command: `source_tab_verification`, `source_tab_headers`, `source_tab_raw`, `copy_source`, `save_source`, `import_key`, `accept_key`, `forget_key`. Each has a key, a command-bar row and an accessible action. `view_source` keeps its id |
| III. One query language | No operator is added (spec, Assumptions) |
| IV. Test-first | Each slice starts from red corpus cases and a red storyboard |
| V. Performance | The surface's reads gain counting assertions. Verification is off the UI path (FR-042) |
| VI. Privacy | D3, D5, D6, D7, D8 and D12 each answer "did the user ask for it". The surface cannot reach the renderer or the network. No DNS and no key lookup exist on any path, and `check-no-silent-tracking.py` gains WKD, keyserver and `_domainkey` mechanisms so that none can arrive by accident. Reading `~/.gnupg` is a local read the person turns on, and it never opens secret-key files |
| VII. Boundaries; Pimalaya first | The new `postio-signature` crate is a pure leaf with a boundary-check entry. The Pimalaya survey is R1, and it is to be repeated before slice B, since a survey more than a few weeks old is stale |

**Gate result**: pass. One new crate is justified under "Complexity Tracking".
The "Signed" mark changes `PRODUCT.md` §21, which the maintainer decided and
this branch updates. No principle changes, and no network path is added.

## Phase 0: Research

### R1. Pimalaya first: what it offers

Surveyed 2026-11-22 from the Pimalaya organisation's repositories and
crates.io.

| Pimalaya piece | What it is | Fit |
|---|---|---|
| `mml` (2026-08 rewrite, `src/pgp.rs`) | PGP/MIME sign, encrypt, decrypt and verify, as properties of an MML part. The crypto is **outside** the pipeline: the interpreter yields, and the caller's backend answers. Only a `gpg` backend survives (gpgme, C, off by default). The "native" backend was dropped because it "needed a key-discovery stack including HTTP" | **Design adopted, crate not.** Postio reads MIME, not MML. Three of its rules are adopted: an invalid signature is a verdict, not a failure; reading degrades and writing refuses; signers come from `From` |
| `pgp-lib` 1.0.0 (`pimalaya/core`, 2024-10) | A high-level async API over rPGP 0.14-era, with WKD behind `key-discovery` | **No.** It has had no release since the family moved to I/O-free crates, and it pins an rPGP two years and several advisories old. #11's comment chose it; that choice is now stale |
| `mime-rs` (2026-11) | A byte-faithful MIME parser: lazy, I/O-free, no `unsafe`, "checked against mail-parser" | **Candidate for R3.** It gives the exact bytes of a signed part with no re-serialisation, which is what verification needs |

Conclusion: Pimalaya has **no maintained OpenPGP or S/MIME engine** that
fits. Its MIME parser and its I/O-free shape do fit. The engine comes from
R2.

### R2. The OpenPGP engine

| | rPGP (`pgp` 0.21.1, 2026-11) | Sequoia (`sequoia-openpgp` 2.4) | GnuPG via gpgme | `pgp-lib` |
|---|---|---|---|---|
| Licence | MIT OR Apache-2.0 | **LGPL-2.0-or-later**. Postio is MIT and links statically, so relinking terms would bind every binary | GPL/LGPL, a separate daemon | MIT |
| Memory-safe | Pure Rust | The default crypto is **Nettle (C)**. The pure-Rust backend needs `allow-experimental-crypto` and `allow-variable-time-crypto` | C | wraps rPGP |
| Standards | RFC 9580 (v6, AEAD) and RFC 4880 | RFC 9580 and RFC 4880 | RFC 4880, with 9580 partial | old rPGP |
| Reviews | 2019 assessment (OTF), 2024 pen test (Radically Open Security), 2024 ETH analysis; the findings fixed (for example RUSTSEC-2024-0447, panics on malformed input) | Long-standing, well reviewed | Mature | inherits old rPGP |
| Users | Delta Chat (the main Autocrypt implementation) | many | many | himalaya ≤ 1.0 |
| Caveats | Low-level by its own description: its APIs "do not claim" to be secure or spec-following when used. The `rsa` crate's Marvin timing issue does not touch **verification** with public keys | Higher-level policy (`StandardPolicy`) is a real plus | System dependency; reads `~/.gnupg` | stale |

**Decision: rPGP**, in a pure leaf that applies its own policy on top:

- reject SHA-1 and MD5 signatures;
- reject RSA under 2048 bits;
- check key validity times;
- check revocation signatures already held.

Sequoia's `StandardPolicy` is the model for that policy, and its rules are
copied as tests, not linked.

**Rejected**:

- Sequoia, for the licence and the C default;
- gpgme, for C and a daemon. The GnuPG keyring is read without it (R7);
- `pgp-lib`, as stale.

The renderer's no-C rule (ADR 0042) binds only `postio-render`. The new leaf
holds itself to it anyway, since it parses attacker-controlled packets.

### R3. Exact bytes and canonicalisation

RFC 3156 signs the first part of `multipart/signed` **as it appears on the
wire**: its MIME headers, its transfer encoding, and lines with CRLF. The
classic failure is re-serialising the part, so that a good signature reads as
bad. The corpus README's `pgp-signed.eml` row says exactly this.

- **Decision**: take the signed part's bytes as a slice of the raw source.
  Use `mime-rs` if it gives part offsets byte-faithfully (spike S1 confirms
  this). Otherwise use `mail-parser`'s part offsets, already in the graph.
  Normalise bare LF to CRLF only, as RFC 3156 requires.
- The raw source must be present, hence D5's keep rule in `postio-sync`'s
  body stage. The top-level `Content-Type` is known from the header block
  the body fetch stores. The rule adds an arm to "store raw?" and adds no
  second fetch.
- **Eviction order** (ADR 0017's `[storage] max_bytes`): raw source of
  unsigned mail first, then attachment payloads, then raw source of signed
  mail. Text is never evicted, as today. ADR 0017 is rewritten in place in
  the slice that builds this, because the rule outlives the feature.

**S1 verdict (2026-11-22): go, with `mime-rs`.** A scratch crate took the
signed part of each of T002's PGP fixtures by `mime-rs` 0.2.1's
`MimeCst::sections` spans and by `mail-parser` 1.0's part offsets. Both gave
bytes identical to the RFC 3156 slice found by hand, in every fixture: the
plain, quoted-printable and list-footer (nested) cases included. `pgp` 0.21.1
with default features off then verified them against `keys/`: good and
quoted-printable `Good`; body, header and re-encoded tampering `Bad`; the
no-key fixture had no issuer key; the inline signature verified through
`CleartextSignedMessage`. `cargo tree -e normal` shows no `-sys` crate for
either. **`mime-rs` is chosen**: it is Pimalaya's, `#![forbid(unsafe_code)]`,
pulls only `memchr` and a quoted-printable codec under its `parser` feature,
and documents its part span as exactly RFC 2046's (delimiters and the CRLF
before the next one excluded). `mail-parser` has 113 `unsafe` sites, which a
leaf parsing hostile bytes should not inherit. One finding for the policy
layer: rPGP verifies the SHA-1 fixture as cryptographically valid, so the
weak-hash refusal is Postio's own, checked before the signature is.

### R4. Authentication-Results, all of it

The parser in `postio_ui::reader::authenticity` already reads methods,
results and `ptype.property` pairs.

- Slice A extends the *model*, not the trust rule. It keeps:
  - `header.d`, `header.s` and `header.b` (DKIM);
  - `policy.*` (DMARC);
  - `smtp.mailfrom` and `smtp.helo` (SPF);
  - `arc` with `header.oldest-pass`.
- It also parses `ARC-Seal`, `ARC-Message-Signature` and
  `ARC-Authentication-Results` sets by instance, to name the sealers. Postio
  **does not verify** ARC seals, which is a DNS question like DKIM. It
  reports the provider's `arc=` result.
- **Alignment** is computed as DMARC defines it (relaxed: the organisational
  domain). It is a pure function. The existing `has_public_suffix` heuristic
  is enough for a display statement, so no public-suffix list is added.
- The words live in `postio-ui`, in one string table, so the terminal and the
  Mac say the same thing.

### R5. Local DKIM: surveyed and declined for good

`mail-auth` (Stalwart, 0.15.1, MIT OR Apache-2.0) verifies DKIM, ARC, SPF
and DMARC, and is the obvious crate. It is **not adopted**, and the
maintainer closed the question on 2026-11-22 (Q2 = A):

- the lookup is a beacon (per-recipient selectors);
- rotated keys fail old mail;
- the provider's check at arrival is the meaningful one.

The DNS-free body-hash check is rejected too. It shows that the body matches
a hash the signature *claims*, which proves nothing without the key, and
reads as a pass. No `dns` value joins `egress_log.subsystem`.

### R6. S/MIME

- **Parsing**: RustCrypto `cms` 0.2 (SignedData) and `x509-cert`, both pure
  Rust. Signature checks use the RustCrypto `rsa`, `p256` and `p384` crates.
- **Path validation** is the hard part. `rustls-webpki` validates paths and
  takes a required extended key usage, so `id-kp-emailProtection` may be
  expressible. It does not check that the certificate's email address
  matches From, so Postio would check that itself.
- **Trust anchors**: the system store, read from disk (`/etc/pki`,
  `/etc/ssl`; on the Mac, the keychain through its own frontend).
  `webpki-roots` is TLS-only, and is wrong for email.
- **Spike S2** proves this over test chains: good, expired, wrong EKU,
  unknown anchor, and address mismatch.
- **Rejected**: `openssl` (C).
- Revocation is never fetched (D8).

**S2 verdict (2026-11-22): go.** Test chains were made with the `openssl`
CLI (root, intermediate, five leaves) because `rcgen` pulls `ring`. A
scratch crate verified `openssl cms -sign` output with `cms` 0.2.3 and
`x509-cert` 0.2.5: the content digest against the signed `messageDigest`
attribute, then the RSA signature over the DER `SET OF` signed attributes
with the RustCrypto `rsa` crate. `rustls-webpki` 0.103 with default
features off validated the path, given a RustCrypto-backed
`SignatureVerificationAlgorithm`. All six outcomes were right: good,
expired, wrong EKU, unknown anchor, address mismatch, and tampered content.
No `ring`, `aws-lc` or `-sys` crate is in the graph. Two findings:

- webpki checks the EKU at every level, and a CA certificate with no EKU
  extension fails `KeyUsage::required`. The path takes
  `required_if_present(id-kp-emailProtection)`, and Postio checks that the
  leaf itself carries `emailProtection`;
- webpki ships no crypto without `ring` or `aws-lc-rs`, so Postio supplies
  the algorithms (RSA PKCS#1 and PSS with SHA-256/384/512, ECDSA P-256 and
  P-384) from RustCrypto, one small impl each.

### R7. Keys

- **Autocrypt** (Level 1): parse the `Autocrypt:` header (`addr`,
  `keydata`, `prefer-encrypt`) on mail already downloaded, at the body
  stage. Record the key only when `addr` equals the From address. It is
  trust-on-first-use material, and is never trusted by arrival (D7).
- **Attached keys**: an `application/pgp-keys` part is offered as "Import
  key". An imported file takes the same path.
- **Store**: see [data-model.md](data-model.md). Acceptance is a row in
  `contact_accepted_keys`, a child of the contact (ADR 0048: the contact is
  the home of what a person decided about a correspondent), with undo
  through the settings verb.
- **No lookup.** WKD and keyservers are out for good (Q3 = C), and the
  tracking check refuses their mechanisms.

**The GnuPG keyring (D12).** GnuPG keeps public keys in one of three formats,
depending on its version and configuration. `GNUPGHOME` is honoured, and
`~/.gnupg` is the default.

| File | Who writes it | What it is | How Postio reads it |
|---|---|---|---|
| `pubring.gpg` | GnuPG 1.x and 2.0, and later versions that inherited one | Transferable public keys back to back, with GnuPG's own trust packets (tag 12) between them | Skip tag-12 packets at the packet layer, then hand each keyblock to rPGP |
| `pubring.kbx` | GnuPG 2.1 to 2.3 by default, and 2.4 without keyboxd | A *keybox*: a header blob, then one blob per key. Each blob gives its type (2 = OpenPGP, 3 = X.509) and the offset and length of its keyblock. The format is described in GnuPG's `kbx/keybox-blob.c` | A small bounds-checked parser in `postio-signature` takes each type-2 keyblock to rPGP, and skips X.509 blobs, which are S/MIME's and out of R7 |
| `public-keys.d/pubring.db` | GnuPG 2.4+ with `use-keyboxd` (new installs) | An SQLite database owned by the `keyboxd` daemon, holding a table of keyblocks | Opened **read-only** by the host with the store's own engine, the keyblob column read, and the keyblocks handed to rPGP. **Spike S3** confirms the table's shape and that Turso opens it read-only, with no write to its WAL. If S3 fails, this format gets the "Postio can't read this keyring" state with the export command |

**Decision: parse the files in-process. Do not run `gpg --export`.**

- **Flatpak.** Reading a file needs only a read-only filesystem permission
  for `~/.gnupg`, which is asked for when the person turns the switch on.
  Running the host's `gpg` from inside the sandbox needs `flatpak-spawn
  --host`, which is a full sandbox escape. Bundling `gpg` would mean shipping
  and patching a C program to read a file.
- **No process and no daemon.** `gpg` may start `keyboxd` or `gpg-agent`, and
  its behaviour follows the person's `gpg.conf`. A file read does neither.
- **Not even installed.** A person can have a keyring copied from another
  machine and no `gpg`.
- **The cost** is three formats, two of them small. Every parse is
  memory-safe, bounds-checked and fuzzed. A format Postio cannot read is a
  visible state, with the export command as the person's own way out.

**Safety of the read.**

- The whole file is read into memory, and never memory-mapped. It is never
  locked or written.
- A torn read (a size or checksum that does not add up) is retried once, and
  the read then reports what it could not parse.
- Secret-key files (`private-keys-v1.d/`, `secring.gpg`) are never opened.
- A keyblock rPGP refuses is skipped and counted in the result, with no
  fingerprint or user id logged.
- **When**: the keyring is read on switching on, on app start if a file's
  modification time changed, and on "Reload GnuPG keys". Each key's source is
  `gnupg`. Switching off removes keys whose only source is `gnupg`, unless the
  person accepted them.

**Web of trust and ownertrust: ignored for the mark (D12).**

- GnuPG's validity is computed state, held in `trustdb.gpg` (or in keyboxd's
  database). It is built by rules Postio would have to re-implement to show
  faithfully: ownertrust levels, marginal counts and certification depth.
- Importing it would make "Signed · accepted" mean two things.
- The Verification tab says "From your GnuPG keyring", and "Accept" is one
  key press.

**A possible later question**, not blocking: whether a key the person has
*certified* in GnuPG with their own key, a certification visible in the
keyblock itself without the trust database, may count as accepted.

**S3 verdict (2026-11-22): go.** GnuPG 2.4.9 with `use-keyboxd` in a scratch
`GNUPGHOME` wrote `public-keys.d/pubring.db` in rollback-journal mode
(`journal_mode=delete`), with tables `pubkey(ubid, type, ephemeral, revoked,
keyblob)`, `fingerprint`, `userid` and `issuer`. `pubkey.type` 1 is OpenPGP,
and `keyblob` is the binary transferable key, which rPGP parses whole. The
workspace's Turso, opened with `Builder::read_only(true)`, read both keys,
with `keyboxd` stopped and again with it running; the directory's files,
sizes and mtimes were the same before and after, and no `-wal` or `-journal`
appeared. GnuPG's dotlock (`pubring.db.lock`) is never touched. The read is
`SELECT type, keyblob FROM pubkey WHERE ephemeral = 0`.

### R8. The surface on three frontends

- The view model lives in `postio-ui::source`: tabs, sections, rows, folds,
  and the strings for every state. Behaviour lives in `postio-focus`: which
  tab, `Esc`, and what `1`, `2` and `3` do.
- GTK draws it in a `postio-widgets` dialog, replacing today's
  `postio-gtk/src/source.rs`.
- The terminal draws three frames. The Mac draws through `postio-ffi`.
- Folding keeps the Raw tab's text buffer small. A 25 MB message is drawn as
  its header block, its text parts and one line per large part.

## Phasing

| Slice | What | Stories, FRs | Depends on |
|---|---|---|---|
| **S1 (spike, gates B)** | rPGP verifies generated-key signatures over corpus-style messages, and the signed part's bytes come out byte-exact (`mime-rs` part offsets against `mail-parser`'s). The leaf's graph passes the boundary check | — | — |
| **S2 (spike, gates C)** | Memory-safe S/MIME path validation with the email-protection EKU, over test chains | — | — |
| **S3 (spike, gates keyboxd only)** | keyboxd's `pubring.db` shape, read-only through Turso | — | — |
| **A. Source surface** (first) | Three tabs over stored headers and raw source; R4's report model and words; the trust boundary; folding; copy and save; the terminal and Mac adopt; storyboard; counting | US1, US2; FR-001 to FR-015, FR-040, FR-041 | none |
| **B. OpenPGP and the mark** | `postio-signature` leaf; D5's whole-message fetch, keep rule and eviction order (ADR 0017 rewritten); Autocrypt recording; verdict job off the UI path; the "Signed" mark and the warning lines (D4); import, accept and forget key; the encrypted state (D9) | US3, US5 (1, 2); FR-020 to FR-025a, FR-027, FR-028, FR-030 to FR-033, FR-042 | S1, A |
| **B2. GnuPG keyring** | The Settings switch and Flatpak permission; `pubring.gpg` and `pubring.kbx` readers; keyboxd after S3; reload; "From your GnuPG keyring" | US5 (3 to 5); FR-030, FR-031a | B; S3 for keyboxd |
| **C. S/MIME** ⛔ S2 | CMS verify, path validation, the address match, "Revocation not checked", and the mark for S/MIME | US4; FR-026 | S2 passes, B |
| *Later spec* | Decryption, secret keys, sending signed or encrypted mail, outgoing Autocrypt | D9 | `specs/015-native-composer` |

The network-lookup slice of the roadmap draft is removed (Q2 = A, Q3 = C).

## Crate placement

| Crate | What it gains | Boundary it keeps |
|---|---|---|
| `postio-signature` also gains | the keybox and `pubring.gpg` parsers (bytes in, keyblocks out; no I/O) | pure leaf |
| `postio-model` | Corpus fixtures (`/add-fixture`): ARC chain, two DKIM signatures, PGP good, bad and no-key with generated keys, inline PGP, Autocrypt, S/MIME good, bad, expired and unknown anchor, encrypted. Plus the `Signed`, `Encrypted` and `SignatureScope` types | no ammonia, rusqlite, gtk4 or tokio |
| **`postio-signature`** (new, slice B) | `verify(raw, part_range, keys) -> Verdict`; the policy; Autocrypt and key parsing; S/MIME in slice C | **pure leaf**: no store engine, toolkit, async runtime or network crate. A new `check-crate-boundaries.py` entry, with the same rule as `postio-calendar` |
| `postio-ui` | `reader::authenticity` grows the report model and alignment. A new `source` module holds the view model and every string. D4's header-line rule joins `warning()` | no GTK, no SQL |
| `postio-focus` | The Source surface's behaviour: tabs, keys, `Esc` (ADR 0045) | — |
| `postio-core` | The new commands, in the registry | no GTK |
| `postio-storage` | `public_keys`, `public_key_addresses`, `signature_verdicts`, `contact_accepted_keys`; the eviction order (data-model.md) | — |
| `postio-sync` | D5's raw-keep arm in the body stage | `MailBackend` only |
| `postio-host` / `postio-session` | The verdict job after body arrival; `Req::SourceView` returning the view model; the key import, accept and forget verbs; the GnuPG keyring read (file I/O, and keyboxd's database read-only) | — |
| `postio-client` | The new Req/Resp variants | no toolkit, engine or protocol |
| `postio-widgets`, `postio-gtk` | The dialog; today's `source.rs` replaced | as today |
| `postio-tui`, `postio-ffi`, `macos/` | The three tabs, from the view model | as today |
| `scripts/checks/check-no-silent-tracking.py` | New refused mechanisms: WKD paths (`/.well-known/openpgpkey/`), `hkps://`, `_domainkey` lookups, and `gpg` process spawns | — |
| `flatpak/*.json` | `--filesystem=~/.gnupg:ro`, explained in the manifest | — |

## Testing (ADR 0049: each layer asserts its own question)

- **Rule crates** (`postio-signature`, `postio-ui`) hold every branch:
  - each verdict and reason, over fixtures;
  - alignment;
  - each state's strings;
  - SC-003's "never good when uncheckable", as a sweep over every fixture
    and every key subset.
- **Counting**: the Source surface's read is a bounded number of statements,
  and the verdict job's statements per signed message are bounded.
- **Session**: a signed message arrives, the job writes its verdict, and the
  reader's header line shows it. An evicted message goes to "Not here yet",
  then resolves.
- **Frontends**: one `focus_suite` case per path a person takes (`V` from
  the list, `V` from the message, the tabs, `Esc`, save), plus the
  terminal's and the Mac's equivalents. The storyboard is red on `main`, and
  `/ux-review` judges it.
- **Privacy**: SC-002's zero connections and zero renderer calls over the
  hostile fixtures, through a counting transport.

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Canonicalisation mistakes make good signatures read bad | People learn to ignore the warning | S1 proves byte-exact extraction over the tricky fixtures (8bit, quoted-printable, nested multipart) before slice B starts |
| rPGP is low-level, and a policy gap lets a weak signature read as good | A false sense of authenticity | Postio's own policy layer, with Sequoia's `StandardPolicy` rules as tests; SC-003's sweep |
| Keeping raw source for signed mail grows the store | Disk use | Signed mail is a small share. It is evicted last but still evicted, and "Not here yet" covers the gap |
| A provider stamps no trusted header (Microsoft today) | The Verification tab is mostly "not believed" | The tab says so plainly, and the user's own `providers.toml` can add an authserv-id |
| Autocrypt TOFU is misread as trust | False confidence | D4: no header mark for a key the person did not accept; D10's fixed line |
| S/MIME path validation in memory-safe code is immature | Slice C slips | D8: S/MIME becomes its own spec, and OpenPGP ships alone |
| The "Signed" mark is read as "safe" | False confidence in a key that arrived in the message it signs | D4a: a word, not a lock or a tick; neutral style; D10's line on the tab; "accepted" only from the person's decision; SC-008 |
| GnuPG's on-disk formats change | The keyring stops being read | Three formats are known, and each read failure is a visible state with the export command. S3 covers keyboxd |
| A torn read of a keyring GnuPG is writing | Keys missing for one read | Retry once; report what was skipped; never lock |
| The Flatpak filesystem permission widens the sandbox | Postio could read more of `~/.gnupg` than it needs | Read-only, `~/.gnupg` only, and the code opens public-key files only (asserted by a test that lists every path opened) |

## Project Structure

### Documentation (this feature)

```text
specs/016-message-authenticity/
├── spec.md              # the roadmap spec
├── plan.md              # this file
├── data-model.md        # tables, types and states
├── contracts/
│   ├── source-surface.md   # the surface, its keys and states
│   ├── header-mark.md      # the "Signed" mark and the warning lines
│   ├── verdict.md          # postio-signature's interface and policy
│   └── gnupg-keyring.md    # the keyring reader
├── quickstart.md        # how to see each slice working
├── checklists/
│   └── requirements.md  # the spec quality checklist
└── tasks.md             # the queue
```

### Source Code (repository root)

```text
crates/postio-signature/                     # new pure leaf (slice B)
crates/postio-ui/src/reader/authenticity.rs
crates/postio-ui/src/source.rs               # new view model
crates/postio-focus/src/                     # Source surface behaviour
crates/postio-core/src/registry.rs
crates/postio-storage/src/schema.rs
crates/postio-sync/src/backfill.rs
crates/postio-host/src/                      # verdict job, source view, key verbs
crates/postio-widgets/src/                   # the dialog
crates/postio-gtk/src/source.rs              # replaced
crates/postio-tui/src/view/
crates/postio-ffi/src/focus_reader.rs
crates/postio-model/tests/corpus/
storyboards/reader/source-surface.toml
```

**Structure Decision**: the engine-plus-frontends layout of
`docs/ARCHITECTURE.md`, with one new pure leaf.

## Complexity Tracking

| Addition | Why needed | Simpler alternative rejected because |
|---|---|---|
| New crate `postio-signature` | It parses hostile OpenPGP and CMS packets, and must be a pure leaf with its own boundary entry, like `postio-calendar` | Putting it in `postio-body` or `postio-ui` would pull a crypto stack into crates every frontend compiles, and blur the boundary around pure parsing of hostile bytes |
