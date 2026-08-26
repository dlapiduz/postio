-- Per-account and per-mailbox signature defaults (#12's last item, #394).
--
-- A signature has always been per-identity, and the picker (0009) lets a
-- draft point at any of the account's named signatures instead -- but
-- nothing decides what the picker *starts on* besides the identity's own.
-- Two accounts sharing one inbox for two roles ("support@" and "sales@",
-- say) want the folder they are composing from to decide, independently of
-- which address happens to be picked that message.
--
-- Both nullable and both `ON DELETE SET NULL`: "no override here" is the
-- ordinary state for most accounts and nearly every mailbox, and a deleted
-- signature should fall the composer back to the identity's own rather than
-- leave a dangling reference or refuse the delete.
ALTER TABLE accounts ADD COLUMN default_signature_id INTEGER
    REFERENCES signatures(id) ON DELETE SET NULL;

ALTER TABLE mailboxes ADD COLUMN signature_id INTEGER
    REFERENCES signatures(id) ON DELETE SET NULL;
