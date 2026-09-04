-- The one-click-unsubscribe activation log (#971).
--
-- CLAUDE.md's privacy section requires one-click unsubscribe to fire "only
-- on deliberate activation" -- this is the record of when that happened,
-- append-only, so the privacy settings pane can show it back. Logging is
-- all this issue does: whether activating also sends the real RFC 8058
-- request is #972, a separate outbound-network decision.
--
-- `list_identifier` is a sender's `List-Id` header when the message that
-- was activated had one, or the sender's domain otherwise -- whichever the
-- reader found, not re-derived here, so this table has no opinion on that
-- extraction and cannot drift from it.
CREATE TABLE unsubscribe_activations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    list_identifier TEXT    NOT NULL,
    activated_at    INTEGER NOT NULL
);

CREATE INDEX idx_unsubscribe_activations_account
    ON unsubscribe_activations (account_id, activated_at DESC);
