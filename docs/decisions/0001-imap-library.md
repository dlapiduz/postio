# ADR 0001 — IMAP library: `io-imap`

- **Status:** Accepted (2026-08-22)
- **Decision:** `postio-account` builds its IMAP backend on Pimalaya `io-imap`, pinned exactly (`=0.6.0`). No `io-imap` type crosses the `MailBackend` boundary.

## Rules
- Pin `io-imap` with `=`, `default-features = false`, `features = ["client"]`. A bump is a reviewed change, never a drive-by `cargo update`. The same discipline applies to `io-smtp`.
- Take `imap-codec` and `imap-types` from `io_imap::codec` and `io_imap::types`. Never depend on them directly: io-imap version-locks the `2.0.0-alpha.*` pair.
- Capabilities come from `ImapSessionOpen` / `ImapClientStd::connect` only. A hand-built auth coroutine sets `ensure_capabilities: true`. An empty post-auth capability list is an error, never a silent downgrade.
- Gate QRESYNC on the post-auth CAPABILITY list, never on the `* ENABLED` echo. iCloud omits the echo; treat a `None` or empty echo as success.
- Expunges come from `SELECT`/`EXAMINE (QRESYNC ...)`.`vanished_earlier`, never from `FETCH ... (VANISHED)`. A real-time `* VANISHED` arrives through the IDLE stream.
- Do not use `watch::ImapMailboxWatch`. Build the loop from ENABLE / SELECT / IDLE / SELECT(QRESYNC) primitives.
- io-imap silently skips an undecodable untagged line (iCloud has sent sequence number `-1` under QRESYNC). Log `io_imap` at `debug` in dev builds. A skip during a resync forces a full resync.
- STORE has no `UNCHANGEDSINCE` in io-imap; use `raw()` or an upstream change.

## Why
- Every extension incremental sync needs (ENABLE, CONDSTORE, QRESYNC, `VANISHED (EARLIER)`, `FETCH CHANGEDSINCE`, IDLE, UIDPLUS, MOVE) is reachable through the public API, verified in source.
- The sans-I/O design lets Postio own the socket and the runtime.
- The risk is release velocity (six minors in eleven weeks, 0.5.0 broke nearly every signature), not capability. The `MailBackend` boundary and the exact pin contain it. `async-imap` was not evaluated because the spike passed.

## Revisit when
- io-imap reaches 1.0, or a bump breaks the sync path.
