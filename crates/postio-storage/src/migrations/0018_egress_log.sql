-- The egress log (#151): every outbound connection Postio opens, recorded
-- where a user can audit it.
--
-- The privacy claim -- nothing leaves this machine that the user did not ask
-- for -- is proven with this log rather than asserted (CLAUDE.md's privacy
-- section; ADR 0003 requirement 7). ADR 0009 Q6 designs the AI provider log
-- as an extension of this table's idea, with its own columns for feature,
-- provider and message ids; this one stays connection-shaped on purpose.
--
-- Ids, counts and outcomes -- never content. A row says "the IMAP engine
-- connected to imap.example.com:993 at 14:02 for account 1"; nothing in the
-- schema can hold a byte of mail.
CREATE TABLE egress_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    subsystem   TEXT    NOT NULL CHECK (subsystem IN ('imap', 'smtp', 'discovery')),
    -- NULL before an account exists: discovery during onboarding probes
    -- servers for an account not yet created.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('connected', 'failed'))
);

-- The settings surface reads newest-first; the proof test counts.
CREATE INDEX idx_egress_log_at ON egress_log (at DESC);
