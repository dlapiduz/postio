-- A draft's send state, on the message row that stands for it.
--
-- Spec 003. #166 gives every draft a `messages` row in the account's Drafts
-- folder, written in the same transaction as the draft and therefore present
-- offline. That row is what the list draws, so the Outbox and Drafts are not
-- two mailboxes — they are two predicates over one, and this column is what
-- either predicate can be written against.
--
-- # Why it is duplicated from `drafts.state` rather than joined
--
-- One of the two predicates lands on `ListScope::Mailbox`, which is the query
-- every folder in the application reads through. A correlated subquery there
-- would be paid by every list in Postio, on every open, to make a distinction
-- that matters to one folder. A column is an indexed comparison instead.
--
-- It is the same trade the schema already makes a few tables up: `draft`,
-- `seen`, `flagged` and `answered` are denormalised off the message's flag set
-- for exactly this reason. A list query must not join to answer what a row is.
--
-- `DraftRepository` is the only writer, in the same transaction that writes
-- `drafts.state`. Not a trigger: two writers with different invariants is how
-- a denormalised value drifts from the thing it denormalises.
ALTER TABLE messages ADD COLUMN send_state TEXT
    CHECK (send_state IS NULL
           OR send_state IN ('editing', 'queued', 'sending', 'sent', 'failed', 'unconfirmed'));

-- `sent` is here, though it is the shortest-lived value the column holds. The
-- drainer marks a draft `Sent` the instant the submission server accepts it
-- and deletes the row moments later, once the copy is filed -- so the state
-- exists, briefly, and a CHECK that forbade it would turn a successful send
-- into a constraint violation.
--
-- It is excluded from *both* list predicates rather than left to fall into
-- Drafts: for that instant the message is neither being written nor on its
-- way. It is in Sent, which is a real folder holding a real copy.

-- Partial, because almost every row in this table is NULL here: an account's
-- drafts are a rounding error against its mail. The index exists to answer
-- "what is this account sending" and "what needs a person", which is the
-- sidebar's two counts, without touching the other 80,000 rows.
CREATE INDEX idx_messages_send_state
    ON messages (account_id, send_state)
 WHERE send_state IS NOT NULL;

-- Backfill. The mirror rows already exist; they simply have not carried this
-- before now. No compatibility shim beyond it: there are no deployed installs,
-- and a draft written after this migration gets the column from the repository.
UPDATE messages
   SET send_state = (SELECT drafts.state FROM drafts WHERE drafts.message_id = messages.id)
 WHERE id IN (SELECT message_id FROM drafts WHERE message_id IS NOT NULL);

-- The counting triggers are deliberately untouched.
--
-- `mailboxes.total_count` keeps meaning "message rows filed here", which is
-- what it means for every other folder, and what every other reader of it
-- expects. The Drafts badge stops asking that column instead: it needs "what
-- the user will see in Drafts", which now excludes what is on its way, and the
-- Outbox needs a number no mailbox row could hold anyway because the Outbox is
-- not a mailbox. Both come from one query over the index above.
--
-- The alternative -- teaching three intricate triggers a fourth condition --
-- would have made one column mean something different for one folder, and put
-- the difference in the hardest place in this schema to read.
