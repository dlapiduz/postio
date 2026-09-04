-- Whether the sender asked for a read receipt (`Disposition-Notification-To`
-- or the older `Return-Receipt-To`), denormalized at ingest the same way
-- `list_id` is (#970). Postio never sends one automatically -- CLAUDE.md's
-- privacy section is explicit that this is fixed policy, not a setting --
-- so this exists only to let the privacy pane count how often it was asked.
ALTER TABLE messages ADD COLUMN read_receipt_requested INTEGER NOT NULL DEFAULT 0;
