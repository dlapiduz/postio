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
