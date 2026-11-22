# Implementation Plan: Message authenticity

**Branch**: `feature/message-authenticity` | **Date**: 2026-11-22 | **Spec**: [spec.md](spec.md)

**Input**: `specs/016-message-authenticity/spec.md`; issue #11 and its
investigation comment; `origin/main` at `17fff8da`.

This is a **roadmap plan**. It records the research, the slices and the crate
placement, so that the feature can be scheduled and built on its own branch.
The Phase 1 artifacts (`data-model.md`, `contracts/`, `quickstart.md`) and
`tasks.md` are written when the maintainer schedules it and has answered Q1
to Q3. Until then nothing here is built.

## Summary

Grow `V`'s raw-source dialog into one **Source** surface with three tabs:

- **Verification**: the provider's checks in words, the headers Postio does
  not believe, the signature, and what Postio checked itself;
- **Headers**: every header, decoded, with the `Received` hops and the trust
  boundary;
- **Raw**: every byte, with large parts folded.

Then verify **OpenPGP** signatures on this computer with **rPGP**, from keys
already on this machine. Then **S/MIME**, if a spike proves memory-safe path
validation. Postio does not re-check DKIM, and fetches no key, unless the
maintainer's answers to Q2 and Q3 add an explicit per-action request.
Decryption and outgoing signing are a later spec.

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
`public_key_addresses` and `signature_verdicts`, and one `contact_settings`
column (ADR 0048). Raw source stays in the blob store, under a changed keep
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
- no DNS (D6);
- no key fetch (D7);
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
| VI. Privacy | D3, D5, D6, D7 and D8 each answer "did the user ask for it". The surface cannot reach the renderer or the network. `check-no-silent-tracking.py` gains WKD, keyserver and DNS mechanisms, so that a later Q2 or Q3 = B slice must carry a `POSTIO-CONSENT:` line |
| VII. Boundaries; Pimalaya first | The new `postio-signature` crate is a pure leaf with a boundary-check entry. The Pimalaya survey is R1, and it is to be repeated before slice B, since a survey more than a few weeks old is stale |

**Gate result**: pass. One new crate is justified under "Complexity Tracking".
Q2 = B or Q3 = B would add a network path, and each would need its consent
line. Neither needs a constitution amendment, because each is a request the
person makes.

## Phase 0: Research

### R1. Pimalaya first: what it offers

Surveyed 2026-11-22 from the Pimalaya organisation's repositories and
crates.io.

| Pimalaya piece | What it is | Fit |
|---|---|---|
| `mml` (2026-08 rewrite, `src/pgp.rs`) | PGP/MIME sign, encrypt, decrypt and verify, as properties of an MML part. The crypto is **outside** the pipeline: the interpreter yields, and the caller's backend answers. Only a `gpg` backend survives (gpgme, C, off by default). The "native" backend was dropped because it "needed a key-discovery stack including HTTP" | **Design adopted, crate not.** Postio reads MIME, not MML. Three of its rules are adopted: an invalid signature is a verdict, not a failure; reading degrades and writing refuses; signers come from `From` |
| `pgp-lib` 1.0.0 (`pimalaya/core`, 2024-10) | A high-level async API over rPGP 0.14-era, with WKD behind `key-discovery` | **No.** It has had no release since the family moved to I/O-free crates, and it pins an rPGP two years and several advisories old. #11's comment chose it; that choice is now stale |
| `mime-rs` (2026-11) | A byte-faithful MIME parser: lazy, I/O-free, no `unsafe`, "checked against mail-parser" | **Candidate for R3.** It gives the exact bytes of a signed part with no re-serialisation, which is what verification needs |
| `io-pim-discovery` 0.15 | DNS over TCP and DNS over HTTPS coroutines (for discovery) | **Candidate DNS client** if Q2 = B. It is already in Postio's graph |
| `io-http` 0.9 | The HTTP client `postio-ai` already uses | The WKD transport if Q3 = B |

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
- gpgme, for C and a daemon. GnuPG's keyring as a key source is Q3 C, a
  separate question;
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

### R5. Local DKIM: surveyed and declined

`mail-auth` (Stalwart, 0.15.1, MIT OR Apache-2.0) verifies DKIM, ARC, SPF
and DMARC, and is the obvious crate.

- It looks keys up through `hickory-resolver` (optional). It can also be
  handed TXT answers, so a Postio-supplied resolver (`io-pim-discovery`'s
  DNS) would work.
- **Not adopted** in any default slice (D6):
  - the lookup is a beacon (per-recipient selectors);
  - rotated keys fail old mail;
  - the provider's check at arrival is the meaningful one.
- If Q2 = B, a later slice adds `mail-auth` behind an explicit per-message
  action, with a new `egress_log.subsystem` value `dns`.
- **Rejected even then**: the DNS-free body-hash check (Q2 C). It shows that
  the body matches a hash the signature *claims*. That proves nothing
  without the key, and it reads as a pass.

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

### R7. Keys

- **Autocrypt** (Level 1): parse the `Autocrypt:` header (`addr`,
  `keydata`, `prefer-encrypt`) on mail already downloaded, at the body
  stage. Record the key only when `addr` equals the From address. It is
  trust-on-first-use material, and is never trusted by arrival (D7).
- **Attached keys**: an `application/pgp-keys` part is offered as "Import
  key". An imported file takes the same path.
- **Store**:
  - `public_keys`: fingerprint, armoured bytes, primary user id, source,
    first seen, last seen, revoked;
  - `public_key_addresses`: fingerprint and normalised address;
  - acceptance: a `contact_settings` column holding `{address:
    fingerprint}` (ADR 0048), with undo through the settings verb.
- **If Q3 = B**:
  - WKD (draft-koch-openpgp-webkey-service: the advanced URL, then the
    direct one) over `io-http`;
  - then the one keyserver named in `[signatures] keyserver`, none by
    default;
  - each attempt gets an `egress_log` row;
  - the action's text names who learns of it ("example.com will see that you
    looked up this address").
- **If Q3 = C**: read `pubring.kbx` or `pubring.gpg` read-only. That parses
  a format rPGP reads, and does not call `gpg`.

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
| **S1 (spike)** | rPGP verifies generated-key signatures over corpus-style messages. Signed bytes come out byte-exact (`mime-rs` against `mail-parser`). The leaf's graph passes the boundary check | — | — |
| **S2 (spike)** | Memory-safe S/MIME path validation with the email-protection EKU, over test chains | — | — |
| **A. Source surface** (first slice) | Three tabs over stored headers and raw source; R4's report model and words; the trust boundary; folding; copy and save; the terminal and Mac adopt; storyboard; counting | US1, US2; FR-001 to FR-015, FR-040, FR-041 | none |
| **B. OpenPGP** | `postio-signature` leaf; D5's keep rule and eviction order (ADR 0017 rewritten); Autocrypt recording; verdict job off the UI path; header lines (D4, Q1); import, accept and forget key; the encrypted state (D9) | US3, US5; FR-020 to FR-025, FR-027, FR-028, FR-030 to FR-033, FR-042 | S1, A, the maintainer's Q1 |
| **C. S/MIME** | CMS verify, path validation, the address match, "Revocation not checked" | US4; FR-026 | S2 passes, B |
| **D. On request** (only if Q2 or Q3 = B) | "Check DKIM from this computer"; "Look up a key"; `egress_log` `dns` and WKD rows; consent lines | FR-031, FR-043 | B, the maintainer's answers |
| *Later spec* | Decryption, secret keys, sending signed or encrypted mail, outgoing Autocrypt | D9 | `specs/015-native-composer` |

**The first slice is A.** It is useful alone, needs no new dependency, and
fixes the "one block of text" problem today's `V` has.

## Crate placement

| Crate | What it gains | Boundary it keeps |
|---|---|---|
| `postio-model` | Corpus fixtures (`/add-fixture`): ARC chain, two DKIM signatures, PGP good, bad and no-key with generated keys, inline PGP, Autocrypt, S/MIME good, bad, expired and unknown anchor, encrypted. Plus the `Signed`, `Encrypted` and `SignatureScope` types | no ammonia, rusqlite, gtk4 or tokio |
| **`postio-signature`** (new, slice B) | `verify(raw, part_range, keys) -> Verdict`; the policy; Autocrypt and key parsing; S/MIME in slice C | **pure leaf**: no store engine, toolkit, async runtime or network crate. A new `check-crate-boundaries.py` entry, with the same rule as `postio-calendar` |
| `postio-ui` | `reader::authenticity` grows the report model and alignment. A new `source` module holds the view model and every string. D4's header-line rule joins `warning()` | no GTK, no SQL |
| `postio-focus` | The Source surface's behaviour: tabs, keys, `Esc` (ADR 0045) | — |
| `postio-core` | The new commands, in the registry | no GTK |
| `postio-storage` | `public_keys`, `public_key_addresses` and `signature_verdicts` (message id, status, reason, fingerprint, checked at); one `contact_settings` column; the eviction order | — |
| `postio-sync` | D5's raw-keep arm in the body stage | `MailBackend` only |
| `postio-host` / `postio-session` | The verdict job after body arrival; `Req::SourceView` returning the view model; the key import, accept and forget verbs | — |
| `postio-client` | The new Req/Resp variants | no toolkit, engine or protocol |
| `postio-widgets`, `postio-gtk` | The dialog; today's `source.rs` replaced | as today |
| `postio-tui`, `postio-ffi`, `macos/` | The three tabs, from the view model | as today |
| `scripts/checks/check-no-silent-tracking.py` | New mechanisms: WKD paths (`/.well-known/openpgpkey/`), `hkps://`, DNS TXT lookups of `_domainkey` | — |

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
| The questions stay open | Slice B's header behaviour is undecided | Slice A depends on none of Q1 to Q3 |

## Project Structure

### Documentation (this feature)

```text
specs/016-message-authenticity/
├── spec.md              # the roadmap spec
├── plan.md              # this file
├── checklists/
│   └── requirements.md  # the spec quality checklist
└── (when scheduled) data-model.md, contracts/, quickstart.md, tasks.md
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
