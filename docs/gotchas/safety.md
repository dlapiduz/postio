# Gotchas: logging, privacy and hostile input

Rules themselves are in CLAUDE.md (Privacy) and the constitution.

- **A `POSTIO_LOG` of only per-target directives turns every other target off.** Start with a bare level (`info,postio_sync=debug`). A run without `postio starting` means the filter hides everything.
- **Redact an error that may name an account with `postio_model::address::redact_addresses` at the log call site.** The screen keeps the address; the log does not. Never log search query text. (enforced by: `check-error-logs-name-their-cause.py`, cause text only)
- **Secrets escape through buffers around `Zeroizing`.** A failed `String::from_utf8` drops bytes unzeroed, and `SecretString::from(String)` frees the old buffer unzeroed when capacity is spare. Copy once with `str::to_owned`.
- **`is_secret_key` matches substrings.** Run it only on unknown keys from a `#[serde(flatten)]` catch-all, never a whole document with fields like `password_help_url`.
- **Never paste fuzz crash inputs or io-imap traces into issues.** They look like real mail. Describe the input and add a fixture via `/add-fixture`.
- **A fuzz panic in a dependency may be a `debug_assert!` release never hits.** Read the panic site before judging severity. `fuzz_target!` aborts even on a caught panic; use `postio_fuzz::allow_contained_panics`. A fuzz property must be a promise the function makes.
- **On the ingest path use a worklist, not recursion.** Input-controlled depth is input-controlled stack, and overflow cannot be caught.
- **`address::parse_list` parses what a person types, so it stays lenient.** Decoded header values may contain CR and LF; fold before writing one back into a header. Change `docs/rfc-compliance.md` and `crates/postio-model/tests/rfc5322.rs` together.

## Signatures

- **rPGP accepts SHA-1 and short keys.** A signature it verifies is mathematics, not a verdict: only `postio_signature::Policy` may say Good. Never return rPGP's answer straight.
- **The signed part is the wire bytes, not a parse.** Cut it from `mime-rs`'s spans; a re-encoded or re-assembled part fails. The CRLF before the boundary belongs to the boundary (RFC 3156 §5). Mail without exact bytes gets `not_here_yet`, never `bad`.
- **webpki asks every certificate on a path for the EKU.** A CA certificate rarely has one, so `KeyUsage::required` fails every real chain. Use `required_if_present(emailProtection)` and check the leaf's own EKU after.
- **`rustls-webpki` with default features off still links `ring` in this workspace**, by feature unification with the TLS stack. S/MIME must pass RustCrypto algorithms, never webpki's own.
- **S/MIME is judged at its `signingTime`, not now**, as OpenPGP is at its creation time. `postio-signature` reads no clock.
- **Never run `gpg` and never take GnuPG's lock.** Read `pubring.kbx`, `pubring.gpg` or keyboxd's `pubring.db` (read-only) whole, and read again once when the lengths do not add up: GnuPG may be writing. (enforced by: `check-no-silent-tracking.py`)
- **This machine's global gitignore drops `*.pem`, `*.crt` and `*.cer`.** The corpus's test anchors are DER (`keys/smime/*.der`); a PEM one would be lost silently.
