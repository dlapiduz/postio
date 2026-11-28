# Contract: `postio-signature`

A pure leaf. It has no store engine, toolkit, async runtime or network crate,
and `check-crate-boundaries.py` gains its entry beside `postio-calendar`'s.
Bytes in, verdicts out. The types are in [data-model.md](../data-model.md).

## Functions

| Function | Input | Output | Never |
|---|---|---|---|
| `signed_scope` | raw RFC 5322 bytes | `Option<SignedScope>` | needs a key |
| `verify` | raw bytes, `KeySet`, `Policy`, now | `Verdict` | reads a file or socket; panics on any input |
| `autocrypt` | an `Autocrypt:` value, the From address | `Option<KeyBlock>` | accepts a key whose `addr` is not From |
| `keyblocks_from_pubring_gpg` | file bytes | `KeyRead` | fails the whole read on one bad keyblock |
| `keyblocks_from_keybox` | file bytes | `KeyRead` | reads past a blob's stated length |

## Exact bytes (S1 decides the parser)

- PGP/MIME: the first body part of `multipart/signed`, from the byte after
  the boundary line's CRLF to the CRLF before the next boundary, exclusive.
  Bare LF is normalised to CRLF, and nothing else changes.
- S/MIME `multipart/signed`: the same slice. `application/pkcs7-mime`
  signed-data: the encapsulated content.
- Inline: the text part's decoded text, with the armour's dash-escaping
  undone (RFC 4880 §7).

## Policy (unit cases, one per rule)

| Rule | Verdict |
|---|---|
| hash SHA-1 or MD5 | `CantCheck(WeakAlgorithm)` |
| RSA key under 2048 bits | `CantCheck(WeakAlgorithm)` |
| signature time before the key's creation, or after its expiry | `Bad(NotYetValid \| Expired)` |
| a revocation signature on the key already held | `Revoked` |
| no key in `KeySet` with the issuer fingerprint | `CantCheck(NoKey)` |
| a packet rPGP refuses | `CantCheck(Malformed)`, never `Bad` |
| a cryptographic mismatch | `Bad(Tampered)` |

## Sweeps

- **SC-003**: for every signature fixture and every subset of its keys,
  `verify` never returns `Good` unless the signer's key is in the set and
  the bytes are untouched.
- **SC-004**: for every tampering fixture, `Bad(Tampered)`.
- **Fuzzing**: `cargo fuzz` targets for `verify`, `keyblocks_from_keybox` and
  `keyblocks_from_pubring_gpg`, run nightly (the `POSTIO-MEASUREMENT` tier),
  never on the merge path.
