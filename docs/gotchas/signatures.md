# Gotchas: signatures

The rules are spec 016's; these are the traps.

- **rPGP accepts SHA-1 and short keys.** A signature it verifies is mathematics, not a verdict: only `postio_signature::Policy` may say Good. Never return rPGP's answer straight.
- **The signed part is the wire bytes, not a parse.** Cut it from `mime-rs`'s spans; a re-encoded or re-assembled part fails. The CRLF before the boundary belongs to the boundary (RFC 3156 §5). Mail without exact bytes gets `not_here_yet`, never `bad`.
- **webpki asks every certificate on a path for the EKU.** A CA certificate rarely has one, so `KeyUsage::required` fails every real chain. Use `required_if_present(emailProtection)` and check the leaf's own EKU after.
- **`rustls-webpki` with default features off still links `ring` in this workspace**, by feature unification with the TLS stack. S/MIME must pass RustCrypto algorithms, never webpki's own.
- **S/MIME is judged at its `signingTime`, not now**, as OpenPGP is at its creation time. `postio-signature` reads no clock.
- **Never run `gpg` and never take GnuPG's lock.** Read `pubring.kbx`, `pubring.gpg` or keyboxd's `pubring.db` (read-only) whole, and read again once when the lengths do not add up: GnuPG may be writing. (enforced by: `check-no-silent-tracking.py`)
- **This machine's global gitignore drops `*.pem`, `*.crt` and `*.cer`.** The corpus's test anchors are DER (`keys/smime/*.der`); a PEM one would be lost silently.
