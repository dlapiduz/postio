-- A message can be hidden from ordinary lists until an instant, then
-- reappear on its own (#493).
--
-- `messages.snoozed_until` is local preference, never server state -- the
-- same shape as `signature_id` and `backfill_excluded` (#394, #350): no wire
-- format carries a snooze, so no sync pass ever reads or writes this column,
-- and unlike those two, no code in the message repository's own write path
-- (`insert`/`write_update`) touches it either -- a fresh row is never born
-- snoozed, and `MessageRepository::snooze` writes it through a dedicated
-- statement, so there is nothing for a resync to clobber in the first place.
-- NULL means "not snoozed", not a sentinel timestamp.
ALTER TABLE messages ADD COLUMN snoozed_until INTEGER;

-- `mailboxes.snoozed_count` extends migration 0003's cached counts with a
-- fourth, and folds snooze into the same "counts as visible" predicate
-- `deleted_locally` already gates: a snoozed message is off the list exactly
-- the way a pending-delete one is, so it must be off `total_count`,
-- `unread_count` and `flagged_count` too, or the sidebar would show a folder
-- as non-empty while opening it shows nothing.
--
-- No backfill statement is needed here the way 0003's was: the column this
-- migration just added cannot hold a value yet, so every row is unsnoozed
-- the instant this runs.
ALTER TABLE mailboxes ADD COLUMN snoozed_count INTEGER NOT NULL DEFAULT 0;

-- Triggers cannot be altered, only replaced -- migration 0003's three are
-- dropped and recreated here with the same "old contribution off, new
-- contribution on" shape, now weighing a fourth column and a fourth count.
--
-- "Now" is SQLite's own clock (`strftime('%s','now') * 1000`, matching
-- `received_at`'s millisecond epoch), read at write time -- the same
-- two-tier arrangement `deleted_locally` already has. The cached counts can
-- lag a snooze's expiry by as much as one write to the row; every list
-- query (`where_clause`, `MEMBER`, `VISIBLE`) reads `snoozed_until` fresh
-- against wall-clock time on every page and is therefore always exactly
-- correct regardless. `postio-runtime`'s `POLL_INTERVAL` sweep is what
-- keeps the cached counts' lag small while the app is open.
DROP TRIGGER messages_count_insert;
DROP TRIGGER messages_count_delete;
DROP TRIGGER messages_count_update;

CREATE TRIGGER messages_count_insert AFTER INSERT ON messages
WHEN NEW.deleted_locally = 0
BEGIN
    UPDATE mailboxes
       SET total_count = total_count +
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000)),
           unread_count = unread_count +
               ((NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
                AND NEW.seen = 0),
           flagged_count = flagged_count +
               ((NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
                AND NEW.flagged = 1),
           snoozed_count = snoozed_count +
               (NEW.snoozed_until IS NOT NULL AND NEW.snoozed_until > (strftime('%s','now') * 1000))
     WHERE id = NEW.mailbox_id;
END;

CREATE TRIGGER messages_count_delete AFTER DELETE ON messages
WHEN OLD.deleted_locally = 0
BEGIN
    -- Clamped at zero, the same reason 0003's version was: a count that has
    -- drifted should degrade to a wrong number, never to a negative one
    -- that reads as a mailbox of length 4294967295 once it crosses the
    -- crate boundary as a u32.
    UPDATE mailboxes
       SET total_count = max(total_count -
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000)), 0),
           unread_count = max(unread_count -
               ((OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
                AND OLD.seen = 0), 0),
           flagged_count = max(flagged_count -
               ((OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
                AND OLD.flagged = 1), 0),
           snoozed_count = max(snoozed_count -
               (OLD.snoozed_until IS NOT NULL AND OLD.snoozed_until > (strftime('%s','now') * 1000)), 0)
     WHERE id = OLD.mailbox_id;
END;

-- One trigger for every column that can move a message in or out of a count,
-- written as "take the old contribution off, put the new one on" -- 0003's
-- own description of its own shape, now also true of `snoozed_until`.
CREATE TRIGGER messages_count_update
AFTER UPDATE OF mailbox_id, seen, flagged, deleted_locally, snoozed_until ON messages
BEGIN
    UPDATE mailboxes
       SET total_count = max(total_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))), 0),
           unread_count = max(unread_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
               AND OLD.seen = 0), 0),
           flagged_count = max(flagged_count - (OLD.deleted_locally = 0 AND
               (OLD.snoozed_until IS NULL OR OLD.snoozed_until <= (strftime('%s','now') * 1000))
               AND OLD.flagged = 1), 0),
           snoozed_count = max(snoozed_count - (OLD.deleted_locally = 0 AND
               OLD.snoozed_until IS NOT NULL AND OLD.snoozed_until > (strftime('%s','now') * 1000)), 0)
     WHERE id = OLD.mailbox_id;
    UPDATE mailboxes
       SET total_count = total_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))),
           unread_count = unread_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
               AND NEW.seen = 0),
           flagged_count = flagged_count + (NEW.deleted_locally = 0 AND
               (NEW.snoozed_until IS NULL OR NEW.snoozed_until <= (strftime('%s','now') * 1000))
               AND NEW.flagged = 1),
           snoozed_count = snoozed_count + (NEW.deleted_locally = 0 AND
               NEW.snoozed_until IS NOT NULL AND NEW.snoozed_until > (strftime('%s','now') * 1000))
     WHERE id = NEW.mailbox_id;
END;
