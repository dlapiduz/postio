# Gotchas: sync, IMAP and accounts

- **A pending local change outranks the server until it settles.** Resync hides messages with an undrained move or delete and replays queued flags; only `pending` and `in_flight` count. A new operation type must decide what an unacknowledged one does. Test with a second client changing a different flag, or the pass fetches nothing.
- **A bulk verb that depends on row state enqueues only rows that disagree.** The queue is the undo set; an agreeing row would let `u` undo what never happened.
- **io-imap keeps only the last untagged `* SEARCH` line.** `existing_uids` checks the listing against `EXISTS`; `SORT` and `THREAD` share the bug. When `crates/postio-account/tests/account_suite/io_imap_search_defect.rs` fails, upstream fixed it: delete the workarounds.
- **QRESYNC: decide from `CAPABILITY` after login, not the `* ENABLED` echo.** Take expunges from `SELECT`'s `vanished_earlier`, not `FETCH (VANISHED)`. An empty capability list after login is an error. Do not use `watch::ImapMailboxWatch`.
- **io-imap completes a command `Ok` with parts missing.** Dropped lines and undecodable `FETCH` items appear only in `ImapOutput` (`skipped`, `partial`). Read via `postio_account::imap`'s `undecoded` and `whole_messages`, not `.data` alone.
- **A Unix socket path caps at 104 bytes (macOS) or 108 (Linux), and the agent socket sits beside the store.** A deep `POSTIO_STORE` (scratchpad, `target/tmp`) makes `agents/mcp.sock` unbindable and the app only logs it. Use a short `mktemp -d`.
- **`UnixStream::try_clone` shares the non-blocking flag.** Converting a clone to tokio makes the original non-blocking; `postio_mcp::socket::peer_of` restores it.
- **Nothing under `postio-host` may print to stdout.** `postio-mcp` runs the host in-process and stdout is its JSON-RPC channel. Log through `tracing` (stderr).
- **Build one `TokenSource` per account, at the composition root.** A second compiles but splits rejections and refresh rotation between IMAP and SMTP. Only `postio_account::auth::with_credential` decides what a refused credential means.
- **An OAuth account has two expiries.** `#oauth-expiry` is the hourly access token; only `#oauth-refresh-deadline` means "sign in again". Re-record it after every refresh.
- **Write the credential before the account row.** `postio_host::startup::route` counts an account only if a row's secret is in the keyring.
