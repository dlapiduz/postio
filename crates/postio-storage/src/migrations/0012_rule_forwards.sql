-- What a rule has forwarded, and the mark on a draft that says a rule made it
-- (#1142, ADR 0008 Q5, ADR 0028).
--
-- `forward:` is the one action that leaves the machine, and it carries three
-- guards. Two of them need something remembered:
--
-- * The **rate cap** is per rule per hour, so there has to be a record of what
--   a rule has already sent. Append-only, like the unsubscribe and egress
--   logs beside it, and for the same reason: it is evidence, and evidence
--   that is edited is not evidence. It also answers "what has this rule
--   actually done", which is the question a settings panel asks.
-- * The **loop guard** is a header on the message that goes out, and the
--   bytes are built from the draft row when the queue drains -- long after
--   the rule that decided to forward has gone. `drafts.forwarded_by` is what
--   survives in between: the rule's name, kept locally, so the builder knows
--   to mark the message and a reader can see which rule sent it. The name
--   itself never leaves the machine; the header it causes is a bare marker.
--
-- `message_id` is not a foreign key on purpose. The record is of a send that
-- happened, and it has to stay true after the message it forwarded is
-- deleted -- a `move:` to Trash by a later rule, an expunge, a UIDVALIDITY
-- reset. `ON DELETE CASCADE` would quietly make the rate cap forget.
CREATE TABLE rule_forwards (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    rule         TEXT    NOT NULL,
    message_id   INTEGER NOT NULL,
    forwarded_at INTEGER NOT NULL
);

-- The rate cap's own query: this rule, this account, since an hour ago.
CREATE INDEX idx_rule_forwards_rate
    ON rule_forwards (account_id, rule, forwarded_at DESC);

ALTER TABLE drafts ADD COLUMN forwarded_by TEXT;
