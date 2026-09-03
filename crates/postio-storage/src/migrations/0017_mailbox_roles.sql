-- Each account's own map from role to server folder (ADR 0025, #962).
--
-- Which of an account's folders plays which part is a fact about that
-- account's server, so it lives beside the account (ADR 0005 Q6b: an account
-- is state, not preference) rather than in `[mailboxes]`, which is one table
-- for every account and stays as the default beneath this one.
--
-- Keyed by path, not by mailbox id: the map is what the user said about the
-- server, and it has to survive the folder's row being retired and re-created
-- when the folder vanishes from a listing and comes back. A path the server
-- no longer lists is a dangling entry, which settings shows rather than
-- anything silently dropping it.
--
-- One row per (account, role) by construction -- the primary key -- because
-- `by_role` answers with one mailbox, and two folders wearing one role is a
-- state nothing can act on (#943). `inbox` is not in the CHECK: RFC 3501
-- names that folder itself. `regular` and `flagged` are not roles a folder
-- is mapped to.
CREATE TABLE mailbox_roles (
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL
                        CHECK (role IN ('archive', 'sent', 'drafts', 'trash', 'junk')),
    path        TEXT    NOT NULL CHECK (length(path) > 0),
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (account_id, role)
) WITHOUT ROWID;
