# ADR 0006 — OAuth 2, and what "providers are data" has to mean

- **Status:** Accepted (2026-08-24); amended 2026-08-27 (io-oauth wire core)
- **Decision:** the token source is a strategy chosen by a preset row, not a build. The preset table is a TOML asset compiled by `build.rs` and layered with a user file. Postio does not block on a CASA assessment to ship OAuth.

## Rules
- **Token sources** sit behind `TokenSource { access_token, invalidate }`: `BrokerTokenSource` (`CommandSecretStore` with expiry: `oama`, `ortie`), `OwnClientTokenSource` (the user's own client id and secret), `PostioClientTokenSource` (Postio's verified client, when it lands). Verification gates a preset row, not a feature. `invalidate` re-runs a broker command after the server rejected a token.
- **Code lives in `postio-account::auth`**, outside the `imap` feature, so SMTP and `postio-sync` get tokens without the protocol crate, testable against `MemorySecretStore`.
- **Flow.** The consent screen opens in the user's own browser, never an embedded view. Redirect to `http://127.0.0.1:<ephemeral>/`, bound only for the flow and closed on code or cancel; no custom URI scheme. PKCE S256 always. `state` is fresh per attempt; a mismatch is dropped without an exchange. Cancellation is first-class (`CancelToken`).
- **Wire core** (grant bodies, token responses, PKCE, `state`) is `io-oauth`. Two deviations stay in `exchange.rs`: the cancellable `pimalaya-stream` HTTP pump, and `token_type` defaulting to `Bearer`.
- **Tokens.** The refresh token goes to the keyring. The access token is a zeroizing `Password`, never written to disk. The flow's requests belong in the egress log.
- **Presets** are `crates/postio-account/data/providers.toml`, parsed by `build.rs` with the runtime's parser into a static, layered with `$XDG_CONFIG_HOME/postio/providers.toml` (user rows win). A row carries `domains`, `imap`, `smtp`, `auth` (preference order), `[provider.oauth]` and `sources`.
- **Endpoints** come from RFC 8414 metadata via `issuer` (fetched during the add-account flow the user started), or from explicit `authorize`/`token`/`scopes` (the offline path). Metadata wins when both exist. An OAuth row with neither is a load-time error.
- **No secrets in `providers.toml`.** `postio-config::secrets::is_secret_key` strips `client_secret`; a test covers it. A user's client credentials go to the keyring.
- **Refresh.** One `TokenSource` per account, shared by its IMAP and SMTP paths. Refresh is single-flight. A rejected token is `invalidate`, one retry, then `Attention`; never retry a revoked grant. Backoff is the existing `RetryPolicy`.
- The preset table is never fetched over the network.

## Why
- Restricted Gmail scopes need a verified consent screen and a CASA assessment with months of lead time, and a desktop client's secret is public and its quota shared.
- Registering only Postio's own client blocks every user on that review. Delegating only (Himalaya v2) makes first run "install another program".
- Embedded browsers can read the password, are blocked by providers, and run with script off.
- A Rust `static` still needs a compiler to add a provider; an ISPDB-style fetch is a speculative request.

## Revisit when
- Postio's verified client is approved: add its preset row.
