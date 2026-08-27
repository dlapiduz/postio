-- The OAuth composition data an account signed in with (#534, ADR 0006).
--
-- What the engine needs, every launch and offline too, to rebuild the
-- account's OwnClientTokenSource: the resolved token endpoint and the
-- client id. Persisted at sign-in rather than re-discovered at startup --
-- RFC 8414 metadata is a network fetch, and a mail client that cannot
-- start without one has traded the wrong thing.
--
-- Never a secret: the client id is public by definition on a native app,
-- and the endpoint is a URL. The client *secret* (rare, and still not a
-- secret on a public client, but treated as one anyway) and the refresh
-- token live in the keyring under derived keys -- see
-- `postio_imap::oauth::token_source`.
--
-- Both NULL on every non-OAuth account, and on OAuth accounts fed by an
-- external broker: broker accounts carry auth_method = 'xoauth2' with no
-- client of their own, and the engine reads exactly that shape.
ALTER TABLE accounts ADD COLUMN oauth_client_id TEXT;
ALTER TABLE accounts ADD COLUMN oauth_token_url TEXT;
ALTER TABLE accounts ADD COLUMN oauth_authorize_url TEXT;
ALTER TABLE accounts ADD COLUMN oauth_scopes TEXT;
