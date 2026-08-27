-- The cross-account move saga (#188, ADR 0005 Q9): the state two per-account
-- queues share instead of the transaction they cannot.
--
-- Moving a message between accounts is the only operation in Postio that can
-- lose mail. There is no server-side move: the message is appended to the
-- target account and removed from the source, over two connections, in two
-- queues. This row is what orders those halves: the source drainer refuses
-- the REMOVE until the phase here says the target copy is CONFIRMED.
--
-- Phases:
--   copying     the copy may be (re-)attempted; nothing is deleted.
--   unconfirmed the append ran but arrival could not be proven — the saga
--               stops and asks, it never guesses and never deletes.
--   confirmed   the target has the message (APPENDUID, or a Message-ID
--               search); the source's REMOVE may run now.
--   done        the source copy is gone; the move is complete.
--   aborted     the saga ended without deleting anything; the source copy
--               is intact (Q13).
--
-- Foreign keys are SET NULL, not CASCADE, on purpose: the saga must outlive
-- the rows it names. Phase 3 deletes the source message; account removal
-- mid-saga must abort the saga, not silently vanish it with the account.
CREATE TABLE cross_account_moves (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    source_message_id  INTEGER REFERENCES messages(id)  ON DELETE SET NULL,
    source_account_id  INTEGER REFERENCES accounts(id)  ON DELETE SET NULL,
    source_mailbox_id  INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    target_account_id  INTEGER REFERENCES accounts(id)  ON DELETE SET NULL,
    target_mailbox_id  INTEGER REFERENCES mailboxes(id) ON DELETE SET NULL,
    -- The provisional local row in the target account: local-first means the
    -- message appears there immediately, and the saga reconciles.
    target_message_id  INTEGER REFERENCES messages(id)  ON DELETE SET NULL,
    -- The raw RFC 5322 bytes to append, content-addressed. Held here as well
    -- as on the source row because phase 3 deletes that row, and the blob
    -- sweep's reference walk includes this column so the bytes survive the
    -- saga however the race with collection falls.
    raw_blob_id        TEXT,
    -- The Message-ID: phase 1's idempotency key and phase 2's fallback
    -- confirmation, on servers without UIDPLUS.
    rfc_message_id     TEXT,
    phase              TEXT NOT NULL DEFAULT 'copying'
                       CHECK (phase IN ('copying', 'unconfirmed', 'confirmed', 'done', 'aborted')),
    confirmed_uid      INTEGER,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- The drainers ask "sagas for this account still in flight".
CREATE INDEX idx_cross_account_moves_phase ON cross_account_moves (phase);
