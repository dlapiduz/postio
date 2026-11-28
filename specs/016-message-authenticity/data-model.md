# Data model: Message authenticity

The spec's "Key Entities", made concrete. Types are named in the crate that
owns them; tables are in `crates/postio-storage/src/schema.rs`'s one
declared `HEAD` (a store an older schema wrote is resynced, as `PRODUCT.md`
§6 says). No compatibility shims.

## Types

### `postio_ui::reader::authenticity` (slice A)

The parser that exists today keeps its trust rule. Its model grows:

```text
AuthReport {
    authserv_id: String,
    trusted: bool,                 // topmost header whose authserv-id is the provider's
    position: usize,               // index among the message's Authentication-Results
    results: Vec<MethodResult>,
}
MethodResult {
    method: Method,                // Spf | Dkim | Dmarc | Arc | Other(String)
    result: String,                // as written: pass, fail, softfail, neutral, none, temperror, permerror, policy
    domain: Option<String>,        // header.d / header.from / smtp.mailfrom / smtp.helo
    selector: Option<String>,      // header.s (DKIM)
    policy: Option<String>,        // policy.* (DMARC)
    aligned: Alignment,            // Aligned | NotAligned | Unknown, relaxed alignment with From
}
ArcSet { instance: u32, sealer: String, cv: Option<String> }   // from ARC-Seal, named only
Verification {
    provider: Option<(String /*display name*/, AuthReport)>,
    not_believed: Vec<AuthReport>,
    arc: Vec<ArcSet>,
    signature: Option<SignatureView>,   // slice B
    local_checks: Vec<LocalCheck>,      // display name (#1928); signature; "not checked here" lines
}
```

`warning()` stays the one place the header line is decided. Slice B makes it
return a `HeaderAuthenticity` in place of `Option<String>`:

```text
HeaderAuthenticity {
    warning: Option<String>,       // #1928's line, joined with the signature warnings (D4)
    mark: Option<Mark>,
}
Mark = Signed | SignedAccepted     // never drawn when the warning names a signature problem
```

`postio_client::Reading.sender_warning: Option<String>` becomes
`authenticity: HeaderAuthenticity`. Every frontend follows (no shim).

### `postio_ui::source` (slice A)

```text
SourceView {
    tab: Tab,                       // Verification | Headers | Raw
    subject: String,
    verification: VerificationView, // sections 1 to 5 of the spec, as rows of text
    headers: Vec<HeaderRow>,        // name, value (decoded, unfolded), raw (as written), boundary_above: bool
    hops: Vec<Hop>,                 // from Received: by, from, at, delay_since_previous
    raw: RawView,                   // lines, with folds
    state: SourceState,             // Local | Fetching { account } | Offline { account } | Partial | SentByYou
}
RawView { segments: Vec<RawSegment> }
RawSegment = Text(String) | Fold { part_label: String, encoding: String, bytes: u64, index: usize }
```

A fold covers a part body over 64 KiB (FR-006). `index` names the part so that
"Show" unfolds it. Copy all and Save act on the stored bytes, never on
segments.

### `postio-signature` (slice B, a new pure leaf)

```text
verify(raw: &[u8], keys: &KeySet, policy: &Policy, now: Timestamp) -> Verdict
signed_scope(raw: &[u8]) -> Option<SignedScope>   // where the signature is, before any key is needed
SignedScope = WholeMessage { format, part: Range<usize>, signature: Range<usize> }
            | Part { format, path: Vec<usize>, part: Range<usize>, signature: Range<usize> }
            | Inline { text_part: Range<usize> }
            | Encrypted { recipients: Vec<KeyId> }
Format = PgpMime | InlinePgp | Smime
Verdict {
    status: Status,                // Good | Bad | CantCheck | NotHereYet | Revoked
    reason: Reason,                // NoKey | WeakAlgorithm(name) | Expired | NotYetValid | Unsupported | Malformed | Tampered | UntrustedIssuer | AddressMismatch
    scope: SignedScope,
    signer: Option<Fingerprint>,   // OpenPGP fingerprint, or the certificate's SHA-256
    signed_at: Option<Timestamp>,
}
KeySet  // the public keys for this message's From address, already parsed
Policy  // R2's rules: no SHA-1 or MD5; RSA >= 2048; validity times; held revocations
autocrypt(header_value: &str, from: &str) -> Option<KeyBlock>
keyblocks_from_pubring_gpg(bytes: &[u8]) -> KeyRead       // tag-12 trust packets skipped
keyblocks_from_keybox(bytes: &[u8]) -> KeyRead            // type-2 blobs; type-3 skipped
KeyRead { blocks: Vec<KeyBlock>, skipped: u32 }
```

`NotHereYet` is never produced by `verify`; the host sets it when the raw
bytes are absent (D5). `verify` never reads a file or a socket.

## Tables (slice B)

```sql
CREATE TABLE public_keys (
    fingerprint  TEXT    PRIMARY KEY,           -- hex, upper case; S/MIME: 'x509:' || sha256
    kind         TEXT    NOT NULL CHECK (kind IN ('openpgp', 'x509')),
    keyblock     BLOB    NOT NULL,              -- the transferable key or the DER certificate
    primary_uid  TEXT,
    revoked      INTEGER NOT NULL DEFAULT 0,
    first_seen   INTEGER NOT NULL,
    last_seen    INTEGER NOT NULL
);
CREATE TABLE public_key_sources (
    fingerprint  TEXT    NOT NULL REFERENCES public_keys(fingerprint) ON DELETE CASCADE,
    source       TEXT    NOT NULL CHECK (source IN ('autocrypt', 'attachment', 'file', 'gnupg')),
    message_id   INTEGER REFERENCES messages(id) ON DELETE SET NULL,  -- for autocrypt and attachment
    seen_at      INTEGER NOT NULL,
    PRIMARY KEY (fingerprint, source)
);
CREATE TABLE public_key_addresses (
    fingerprint  TEXT    NOT NULL REFERENCES public_keys(fingerprint) ON DELETE CASCADE,
    address_id   INTEGER NOT NULL REFERENCES addresses(id),
    PRIMARY KEY (fingerprint, address_id)
);
CREATE INDEX idx_public_key_addresses_address ON public_key_addresses (address_id);
CREATE TABLE contact_accepted_keys (               -- ADR 0048: on the contact
    contact_id   INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    address_id   INTEGER NOT NULL REFERENCES addresses(id),
    fingerprint  TEXT    NOT NULL,
    accepted_at  INTEGER NOT NULL,
    PRIMARY KEY (contact_id, address_id)
);
CREATE TABLE signature_verdicts (
    message_id   INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    format       TEXT    NOT NULL CHECK (format IN ('pgp_mime', 'inline_pgp', 'smime', 'encrypted')),
    scope        TEXT    NOT NULL CHECK (scope IN ('whole', 'part', 'inline', 'encrypted')),
    status       TEXT    NOT NULL CHECK (status IN ('good', 'bad', 'cant_check', 'not_here_yet', 'revoked')),
    reason       TEXT,
    fingerprint  TEXT,
    checked_at   INTEGER NOT NULL
);
```

- `signature_verdicts` is written by the verdict job and read with the
  reading (one row by primary key, FR-041's count). A key added, accepted or
  forgotten for an address re-runs the job for that address's signed messages
  (bounded by `public_key_addresses`), so no read path computes a verdict.
- `public_key_sources` lets "switch the GnuPG keyring off" remove keys whose
  only source is `gnupg`, keeping accepted ones (plan R7).
- `messages.content_type` (existing) is what D5's keep rule reads.

## Raw source and eviction (slice B; ADR 0017 rewritten in place)

| Message's top-level type | Body stage fetches | `raw_blob_id` |
|---|---|---|
| `multipart/signed`, `multipart/encrypted`, `application/pkcs7-mime`, `application/pkcs7-signature` | the whole message (`BODY[]`), so the signed bytes are exact | kept |
| anything else | the text axis, as today | none |

Eviction order in `postio_storage::blob::evictable`: raw source of
unsigned mail, then attachment payloads, then raw source of signed mail. Text
is never evicted.

## States

```text
Verdict job:  (body arrives) ──▶ scope? ──none──▶ no row
                                   │
                                   ├─ raw here ──▶ verify ──▶ good | bad | cant_check | revoked
                                   └─ raw gone ──▶ not_here_yet ──(raw fetched on open)──▶ verify
Key change for an address ──▶ re-verify that address's rows
```

```text
Header (D4):  bad | different-key ──▶ warning line, no mark
              good + whole + accepted key ──▶ "Signed · accepted"
              good + whole ──▶ "Signed"
              good + part/inline-not-whole | cant_check | not_here_yet | none ──▶ no mark
              (#1928's domain warning joins independently)
```
