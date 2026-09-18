# Two OAuth expiries, and only one of them is a failure (2026-09-04, #954)

There are two timestamps in the keyring for an OAuth account and they look
alike enough that #954 was originally written against the wrong one:

| key | what it is | how long | what its passing means |
|---|---|---|---|
| `<account>#oauth-expiry` | the **access** token (#870) | an hour | nothing: a refresh renews it silently |
| `<account>#oauth-refresh-deadline` | the **refresh** grant (#954) | days to months | the account is dead until someone signs in again |

The access expiry comes from `expires_in` in every token response, so it is
already in the past for any account nobody has synced for an hour. Routing on
it — which is what the issue first asked for — would have put every OAuth
account into "sign in again" roughly hourly. The refresh grant's lifetime is
the one whose expiry is real and unrecoverable, and **no token response ever
mentions it**: the only source is the provider's documentation, which is why
it is a `providers.toml` field.

Three things worth keeping:

- **Re-record the deadline on every successful refresh, not only on
  rotation.** Microsoft's ninety days slide — the window resets on use — so a
  deadline written once retires exactly the accounts being used most. Google's
  seven days do not slide, so sliding treatment makes a dead Google grant look
  healthy for a while longer; that costs nothing, because the reactive path
  (`invalid_grant` → `Blocker::Authentication`) still catches it.
- **A cached access token is still served past the deadline, deliberately.**
  It works until it expires, and refusing it early would interrupt a working
  session for a deadline that has not cost anything yet. The refusal happens
  where the refresh would have.
- **This is earliness, not a second failure path.** A dead grant produces the
  same `Blocker::Authentication` a refused refresh already produced, so every
  surface that learned to render one renders both. The only new code on the
  routing side is one arm in `BackendError::is_authentication_failure` —
  `SecretError::GrantExpired`, and pointedly not `Locked` or `NotFound`, which
  are fixed by unlocking and by adding a credential rather than by signing in
  again.
