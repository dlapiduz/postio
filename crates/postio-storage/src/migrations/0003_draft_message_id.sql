-- The `Message-ID` reserved for a draft's send attempt series (ADR 0021).
--
-- NULL while the draft is being edited. `DraftRepository::queue_send` mints
-- one in the same write that enqueues `Operation::Send`, and every build of
-- the message uses it — so a retried send is the *same* message rather than a
-- second, distinct one that happens to say the same thing, which is what a
-- fresh id per build made it. #461.
ALTER TABLE drafts ADD COLUMN rfc_message_id TEXT;
