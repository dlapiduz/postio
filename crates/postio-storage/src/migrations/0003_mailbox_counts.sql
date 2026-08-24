-- Give the cached mailbox counts a writer.
--
-- `mailboxes.total_count`, `unread_count` and `flagged_count` exist so the
-- sidebar never counts rows, and so the message list can ask "how long are
-- you" with every page without paying a linear scan for the answer. They were
-- derived data with no owner: `MailboxRepository::recount` maintained them and
-- had one production caller in the whole workspace.
--
-- The consequence was not a wrong number, it was an empty application. The
-- list is a GListModel whose n_items is that total, and a GtkListView over a
-- model of length zero asks for no pages at all. On a live account with 81,716
-- messages every count was 0 and every folder drew nothing, while the store
-- and the query were both perfectly correct (postio-qhz.7).
--
-- Triggers rather than a call at each write site, because the call sites are
-- the problem: sync, the operation drainer, undo, threading and every future
-- writer would each have to remember, and the one that forgets produces a
-- blank mailbox rather than a visibly wrong number. Here the invariant is the
-- table's, and nothing above has to know the column exists.
--
-- `deleted_locally = 0` is the same predicate the list query uses. A message
-- hidden pending a remote delete is not in the folder as far as anything the
-- user can see is concerned, so it must not be in the count either.

-- Whatever the previous builds left behind. Runs once, on open, so a store
-- that was already full of mail lists it immediately and offline, rather than
-- waiting for a sync to repair the numbers.
UPDATE mailboxes SET
    total_count = coalesce((SELECT count(*) FROM messages
                             WHERE mailbox_id = mailboxes.id AND deleted_locally = 0), 0),
    unread_count = coalesce((SELECT count(*) FROM messages
                              WHERE mailbox_id = mailboxes.id AND deleted_locally = 0
                                AND seen = 0), 0),
    flagged_count = coalesce((SELECT count(*) FROM messages
                               WHERE mailbox_id = mailboxes.id AND deleted_locally = 0
                                 AND flagged = 1), 0);

CREATE TRIGGER messages_count_insert AFTER INSERT ON messages
WHEN NEW.deleted_locally = 0
BEGIN
    UPDATE mailboxes
       SET total_count = total_count + 1,
           unread_count = unread_count + (NEW.seen = 0),
           flagged_count = flagged_count + (NEW.flagged = 1)
     WHERE id = NEW.mailbox_id;
END;

CREATE TRIGGER messages_count_delete AFTER DELETE ON messages
WHEN OLD.deleted_locally = 0
BEGIN
    -- Clamped at zero: a count that has drifted should degrade to a wrong
    -- number, never to a negative one that reads as a mailbox of length
    -- 4294967295 once it crosses the crate boundary as a u32.
    UPDATE mailboxes
       SET total_count = max(total_count - 1, 0),
           unread_count = max(unread_count - (OLD.seen = 0), 0),
           flagged_count = max(flagged_count - (OLD.flagged = 1), 0)
     WHERE id = OLD.mailbox_id;
END;

-- One trigger for every column that can move a message in or out of a count,
-- written as "take the old contribution off, put the new one on". The two
-- statements land on the same row when only a flag changed and on two rows
-- when the message moved folder, which is the whole of what has to happen in
-- either case -- and it stays correct when a writer sets a column to the value
-- it already held, which the repositories do on every upsert.
CREATE TRIGGER messages_count_update
AFTER UPDATE OF mailbox_id, seen, flagged, deleted_locally ON messages
BEGIN
    UPDATE mailboxes
       SET total_count = max(total_count - (OLD.deleted_locally = 0), 0),
           unread_count = max(unread_count - (OLD.deleted_locally = 0 AND OLD.seen = 0), 0),
           flagged_count = max(flagged_count - (OLD.deleted_locally = 0 AND OLD.flagged = 1), 0)
     WHERE id = OLD.mailbox_id;
    UPDATE mailboxes
       SET total_count = total_count + (NEW.deleted_locally = 0),
           unread_count = unread_count + (NEW.deleted_locally = 0 AND NEW.seen = 0),
           flagged_count = flagged_count + (NEW.deleted_locally = 0 AND NEW.flagged = 1)
     WHERE id = NEW.mailbox_id;
END;
