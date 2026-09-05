# ADR 0006 — OAuth 2, and what "providers are data" has to mean

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#2 OAuth 2 + extensible provider presets](https://github.com/dlapiduz/postio/issues/2)
- **Related:** [ADR 0001](0001-imap-library.md) (the `MailBackend` seam),
  `docs/ARCHITECTURE.md` §7 (providers are data) and §10 (generated, never
  retyped), [#1](https://github.com/dlapiduz/postio/issues/1) (multi-account),
  [#64](https://github.com/dlapiduz/postio/issues/64) (add-account flow)
- **Decision:** **the token source is a strategy, not a build.** Postio ships
  three ways to obtain an OAuth token — an external broker, the user's own
  client credentials, and eventually Postio's own verified client — behind one
  trait, chosen by a preset row. The preset table becomes a **TOML asset
  compiled by `build.rs`**, layered with a user file at runtime. Postio does
  not block on a CASA assessment to ship OAuth.

---

## What is already built

| Piece | State |
|---|---|
| Keyring-backed `SecretStore`, with `Password` zeroized | Built (`imap/src/secret.rs`) |
| **`CommandSecretStore`** — read a secret by shelling out | **Built** (`secret.rs:449`) |
| `MemorySecretStore` for tests | Built |
| Preset table, one row per provider, no vendor identifiers | Built (`imap/src/discovery/builtin.rs`) |
| Autoconfig probe chain: preset → Thunderbird autoconfig → SRV → guess | Built (`discovery/mod.rs`) |
| `accounts.auth_method CHECK (… 'oauth2', 'xoauth2')` | **Built** — the schema is already ready |
| `AccountSettings.requires_app_password` | Built, with a doc comment that already anticipates "no OAuth path" |
| `postio-sync` reaching the keyring without the protocol crate | Built — `default-features = false` |
| Any OAuth code at all | **Absent** |

The finding that shapes this ADR: **`CommandSecretStore` is already an OAuth
delegation mechanism.** `oama access ada@example.com` prints an access token to
stdout; so does `mutt_oauth2.py`; so does `ortie`. Postio can consume a
delegated token today with no new subsystem — the trait, the escape hatch and
the documentation for it already exist.

---

## Q1 — Build the OAuth client, or delegate it?

The issue asks for this decision explicitly, and cites Himalaya v2 dropping its
built-in flow in favour of `pimalaya/ortie`.

The reason it is a hard question is that an OAuth client for mail is not code —
it is a **relationship with a provider**. Restricted-scope Gmail access needs a
verified consent screen and a CASA security assessment, both with months of
lead time and both recurring. And a desktop application is a *public* client:
its `client_secret` ships in a public repository, where it is not a secret, and
its quota is shared by everyone who ever installs it.

**Decision: neither exclusively. The token source is a strategy, and which one
a provider uses is a field in its preset row.**

```rust
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// A currently-valid access token, refreshing if the cached one is stale.
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError>;
    /// Called when the server rejected the token we just presented.
    async fn invalidate(&self, account: &AccountKey);
}
```

Three implementations, in the order they ship:

| Strategy | What it is | Verification burden | Ships |
|---|---|---|---|
| `BrokerTokenSource` | `CommandSecretStore` plus expiry semantics — `oama`, `ortie`, `mutt_oauth2.py` | none | first |
| `OwnClientTokenSource` | the user's own `client_id`/`client_secret` from their own cloud project, full flow in Postio | none (it is the user's own project) | first |
| `PostioClientTokenSource` | Postio's verified client, credentials baked in | CASA + consent review | when it lands |

The third is the same code as the second with different credentials. That is
the point: **the verification process gates a preset row, not a feature.** Work
on it starts now, in parallel, because the lead time is the risk — but nothing
waits for it.

This also answers "who does Postio become". A mail client that only works
through its own verified client is a client whose users are hostage to its
Google standing. One where the token source is data is one a user can run
against their own project, or against a broker they already trust, forever.

**`CommandSecretStore` needs exactly one change.** It is "read-only by
construction", which is right, and it caches nothing — but an access token
expires, and nothing today re-invokes the command when a server says
`AUTHENTICATE` failed. `invalidate` on the trait above is that: the session
layer that got the rejection calls it, and the next `access_token` re-runs the
command. Without this, delegation works for exactly one token lifetime and then
looks like a broken account.

---

## Q2 — Where does the OAuth code live?

`postio-imap` with `default-features = false` is already the crate that means
*reach an account*: the `MailBackend` seam, its mock, autoconfig discovery and
the keyring, with `io-imap` and its TLS stack excluded. `postio-sync` depends on
it exactly that way, and `postio-smtp` takes the credential as a parameter
rather than fetching it.

**OAuth goes there, in `postio-imap::auth`, outside the `imap` feature.** That
gives SMTP the same token through the same path it already gets a password,
gives `postio-sync` refresh without acquiring the protocol crate, and keeps the
token machinery testable against `MemorySecretStore` with no network — which
the repository's no-network rule requires.

The loopback listener and the token exchange are HTTP, and
`discovery/transport.rs` already does HTTP in this crate for autoconfig, so no
new dependency class arrives.

**Recorded wart:** `postio-imap` now holds three things that are not IMAP —
discovery, the keyring, and OAuth. The name is wrong and `postio-account` would
be right. A rename touches every crate that names it and buys clarity only, so
it is not a blocker; it is the sort of thing to do while the crate is open for
other reasons rather than as its own change.

> **Resolved 2026-09-03 (#153):** renamed to `postio-account`, keeping the
> `imap` feature and module names as they were — the maintainer's call was
> the rename alone, not the split #153 also sketched. Every reference in this
> ADR below is left as `postio-imap`, the name that was true when each of
> these decisions was made; read it as history, not as the crate's current
> name.

---

## Q3 — The flow: system browser, loopback redirect, PKCE

- **The consent screen opens in the user's own browser.** Never in a
  `WebKitWebView` inside Postio. An embedded browser can read the password
  being typed into it, which defeats the reason OAuth exists; major providers
  block embedded user agents outright; and Postio's hardened `WebView` has
  JavaScript off, which a consent screen requires.
- **Redirect to `http://127.0.0.1:<ephemeral>/`**, a listener bound for the
  duration of the flow and closed the moment the code arrives or the user
  cancels. Not a custom URI scheme: that needs a desktop-file registration and
  hands the callback to whatever else claimed the scheme.
- **PKCE (S256) always**, including where a client secret exists. It is what
  makes the interception of a loopback redirect useless.
- **`state` is a fresh random value per attempt**, and a callback whose state
  does not match is dropped without a token exchange.
- **Cancellation is first-class.** `postio-imap::cancel::CancelToken` already
  exists for the discovery probe, and
  [#57](https://github.com/dlapiduz/postio/issues/57) is an open bug about the
  probe not honouring it. The OAuth flow is the same shape — a long
  user-initiated wait that must be abandonable — so it uses the same token, and
  #57 should be fixed before this is written rather than reproduced in a second
  place.

> **Amended 2026-08-27 (#537).** The flow's RFC 6749/7636 wire core —
> grant bodies, token response schemas, PKCE, the `state` value — is
> `io-oauth` (Pimalaya) rather than hand-rolled, adopted at the
> maintainer's direction once the crate (published in the same weeks this
> ADR was first written, and missed by its survey) reached 0.3. Two
> deliberate deviations stay in `exchange.rs`, each a candidate upstream
> change: the HTTP pump remains this crate's cancellable
> `pimalaya-stream` transport because io-oauth's optional client consumes
> the HTTP status that the 200-with-error-body shim needs; and a missing
> `token_type` still defaults to `Bearer`, which §5.1 forbids and real
> refresh responses do anyway. The transmitted `state` is base64url over
> io-oauth's entropy: RFC 6749 allows any VSCHAR in `state`, and a value
> with `&` or spaces has to survive a browser redirect and a query parse
> to be compared at all.

**Privacy posture.** Every network request in this flow is one the user asked
for by clicking *Sign in*, which is the test `ARCHITECTURE.md` §11 sets. The
listener binds only on demand and only on loopback. The refresh token goes
straight to the keyring; the access token is a `Password` (zeroizing) and is
never written to disk. `scripts/checks/check-no-silent-tracking.py` should learn about
this flow so that "Postio opened a connection" stays an enumerable list — and
the flow's requests belong in the egress log (#151) that turns that list from
an assertion into a record the user can read.

---

## Q4 — Presets: from a Rust `static` to a compiled TOML asset

`builtin.rs` today is a `static PRESETS: &[Preset]` with one row, and its doc
comment already states the rule correctly: every provider is one row, no named
constant, no vendor identifier. That satisfies §7's *spirit*. It does not
satisfy the issue's criterion — "presets are data, not code" — in the sense
that matters to a user, because adding a provider still requires a Rust
toolchain.

**Decision: the table is a TOML file, parsed at build time into the same
static, and layered with a user file at runtime.**

```
  crates/postio-imap/data/providers.toml     ← the shipped table
        │  build.rs, using the same parser as the runtime
        ▼
  static PRESETS: &[Preset]                  ← unchanged shape, zero runtime cost
        ▲
        │  layered at startup, user rows win
  $XDG_CONFIG_HOME/postio/providers.toml     ← the user's own additions
```

This is `ARCHITECTURE.md` §10's tokens pipeline applied to a second table, for
the same reason and with the same guarantee: `build.rs` compiles the parser
module directly (`#[path = …]`), so the build script and the test suite run
*exactly* the same code and drift is caught by a test rather than by eye.

A preset row grows the fields OAuth needs:

```toml
[[provider]]
display_name = "…"
domains      = ["example.net"]
imap         = { host = "…", port = 993, security = "tls" }
smtp         = { host = "…", port = 465, security = "tls" }
auth         = ["oauth2", "app-password"]      # in preference order

[provider.oauth]
authorize = "https://…/authorize"
token     = "https://…/token"
scopes    = ["https://…/mail"]
# How a token is obtained. `broker` and `own-client` need nothing from Postio;
# `builtin` is only present for providers Postio has a verified client for.
sources   = ["builtin", "broker", "own-client"]
```

> **Amended 2026-08-24 (#152), before the schema was implemented.** As
> written above, `authorize`, `token` and `scopes` are hand-carried in every
> row. [ADR 0005](0005-multiple-accounts.md) Q7's survey found that
> unnecessary: `io-pim-discovery` — already in the graph for autoconfig —
> ships `rfc8414` (OAuth 2.0 authorization-server metadata), so for any
> provider that publishes metadata those endpoints can be **discovered
> instead of maintained**. Three hand-kept fields per provider become zero,
> and an endpoint a provider rotates keeps working without a Postio release.
>
> The amended shape: the `[provider.oauth]` endpoint fields are **one source
> of two, and metadata wins when both exist**. A row may carry an `issuer`
> from which RFC 8414 metadata is fetched — during the add-account flow the
> user initiated, the same consent footing as the autoconfig probe, so §11's
> "did the user ask for it" test passes for the same reason. Explicit
> `authorize`/`token`/`scopes` remain the offline and no-metadata path, and
> the row still owns what metadata cannot supply: the `sources` list, the
> client id reference, and the preference order in `auth`. The user-overlay
> semantics are unchanged.
>
> Consequence for the implementation (#191): the parser accepts `issuer`,
> endpoint fields, or both — but a row with an OAuth entry in `auth` and
> *neither* is a validation error, said at load time rather than discovered
> at sign-in.

**The user file is what makes this real.** A self-hosted provider, a corporate
IdP, a provider Postio has never heard of: one row in a file the user owns, no
rebuild, no issue filed, no wait for a release. It also means the shipped table
can stay small and honest rather than becoming a directory Postio has to
maintain — Thunderbird's ISPDB is a full-time job, and Postio's probe chain
already falls back to autoconfig and SRV for everything not in the table.

**`providers.toml` holds no secrets and the existing check enforces it.**
`postio-config::secrets::is_secret_key` already strips `client_secret` from any
TOML Postio reads (`clientsecret` contains `secret`). A user's own client
credentials therefore go to the keyring like everything else, and the preset
file references the entry. This falls out of what is built rather than needing
new enforcement — but it needs a test, because it is exactly the kind of
property that is true by accident until someone adds a bypass.

---

## Q5 — Refresh, and N accounts hitting expiry at once

With multi-account (ADR 0005) there are N engines, each on its own thread, each
presenting a token. Access tokens for one provider expire together, so a
stampede is the normal case rather than the edge one.

- **One `TokenSource` instance per account**, held by the composition root and
  shared with both that account's IMAP and SMTP paths. Not one per connection:
  the IMAP pool (`imap/pool.rs`) opens several sessions, and each refreshing
  independently would multiply the refresh calls by the pool size and can
  invalidate the others' tokens on providers that rotate refresh tokens.
- **Refresh is single-flight per account.** Concurrent callers await one
  in-flight refresh rather than each starting one.
- **A rejected token is `invalidate` then one retry, then the account goes to
  `Attention`.** `SyncStatus`/`Attention` already exist in `postio-sync`, and a
  reauthorisation prompt is a UI state, not a retry loop. Retrying a genuinely
  revoked grant is how an account gets rate-limited or locked.
- **Backoff on the token endpoint is the existing `RetryPolicy`**, not a new
  one.

---

## Alternatives

**Register Postio's own client and ship only that.** The straightforward
reading of the issue, and it makes Gmail work with no user setup. Rejected as
the *only* path: it blocks every OAuth user on a review with months of lead
time, ties Postio's users to Postio's standing with a provider, and shares one
quota across all installs. Kept as one strategy among three.

**Delegate only, as Himalaya v2 does.** Cheapest, and defensible — but it makes
the first-run experience for a mainstream provider "install this other program
first", which is precisely the impression `builtin.rs`'s doc comment says the
preset table exists to avoid.

**Keep presets as a Rust `static`.** Already good, already vendor-neutral, and
still requires a compiler to add a provider. The TOML asset costs one build
script and closes the criterion properly.

**Fetch the preset table over the network, ISPDB-style.** A speculative network
request at startup, for a file that changes a few times a year. Against §11 and
unnecessary: the probe chain already queries autoconfig per-domain, on demand,
when the user types an address.

---

## Consequences

- `postio-imap` gains `auth`, outside the `imap` feature, plus `build.rs` and
  `data/providers.toml`. `postio-sync` and `postio-smtp` are unchanged except
  that the credential they are handed may now come from a `TokenSource`.
- `CommandSecretStore` gains `invalidate`; delegated tokens start surviving
  their first expiry.
- The onboarding screen (#64) gains an auth-method branch driven by the preset
  row rather than by a condition on the provider.
- Start the consent-screen and CASA process **now**, tracked as its own issue,
  because it is a calendar dependency and not an engineering one.
