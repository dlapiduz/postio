-- The search walk's columns on `messages` (spec 010 D30): its people, its
-- labels and how many attachments it carries, kept here so the walk reads
-- three columns rather than three correlated lookups per matched message.
-- ADR 0040's rule: an aggregate on a hot path is a maintained answer.
--
-- An insert appends, which is one row write whatever the message already
-- holds; a delete or a move recounts the message from its rows, which is a
-- seek on the child table's `(message_id, ...)` key. Every lookup here is
-- by key, and nothing here touches a full-text index.

CREATE TRIGGER trg_walk_people_recipients_ai AFTER INSERT ON recipients
WHEN new.message_id IS NOT NULL AND new.kind IN ('from', 'to', 'cc', 'bcc')
BEGIN
    UPDATE messages
       SET people_ids = coalesce(people_ids || ',', '')
                        || CASE new.kind WHEN 'from' THEN -new.address_id
                                         ELSE new.address_id END
     WHERE id = new.message_id;
END;

CREATE TRIGGER trg_walk_people_recipients_ad AFTER DELETE ON recipients
WHEN old.message_id IS NOT NULL AND old.kind IN ('from', 'to', 'cc', 'bcc')
BEGIN
    UPDATE messages
       SET people_ids = (SELECT group_concat(CASE r.kind WHEN 'from' THEN -r.address_id
                                                         ELSE r.address_id END)
                           FROM recipients r
                          WHERE r.message_id = old.message_id
                            AND r.kind IN ('from', 'to', 'cc', 'bcc'))
     WHERE id = old.message_id;
END;

CREATE TRIGGER trg_walk_people_recipients_au
AFTER UPDATE OF message_id, kind, address_id ON recipients
BEGIN
    UPDATE messages
       SET people_ids = (SELECT group_concat(CASE r.kind WHEN 'from' THEN -r.address_id
                                                         ELSE r.address_id END)
                           FROM recipients r
                          WHERE r.message_id = messages.id
                            AND r.kind IN ('from', 'to', 'cc', 'bcc'))
     WHERE id IN (old.message_id, new.message_id);
END;

CREATE TRIGGER trg_walk_labels_message_labels_ai AFTER INSERT ON message_labels
BEGIN
    UPDATE messages
       SET label_ids = coalesce(label_ids || ',', '') || new.label_id
     WHERE id = new.message_id;
END;

CREATE TRIGGER trg_walk_labels_message_labels_ad AFTER DELETE ON message_labels
BEGIN
    UPDATE messages
       SET label_ids = (SELECT group_concat(ml.label_id) FROM message_labels ml
                         WHERE ml.message_id = old.message_id)
     WHERE id = old.message_id;
END;

CREATE TRIGGER trg_walk_labels_message_labels_au
AFTER UPDATE OF message_id, label_id ON message_labels
BEGIN
    UPDATE messages
       SET label_ids = (SELECT group_concat(ml.label_id) FROM message_labels ml
                         WHERE ml.message_id = messages.id)
     WHERE id IN (old.message_id, new.message_id);
END;

CREATE TRIGGER trg_walk_attachments_ai AFTER INSERT ON attachments
WHEN new.message_id IS NOT NULL
BEGIN
    UPDATE messages SET attachment_count = attachment_count + 1 WHERE id = new.message_id;
END;

CREATE TRIGGER trg_walk_attachments_ad AFTER DELETE ON attachments
WHEN old.message_id IS NOT NULL
BEGIN
    UPDATE messages SET attachment_count = max(attachment_count - 1, 0)
     WHERE id = old.message_id;
END;

CREATE TRIGGER trg_walk_attachments_au AFTER UPDATE OF message_id ON attachments
WHEN old.message_id IS NOT new.message_id
BEGIN
    UPDATE messages SET attachment_count = max(attachment_count - 1, 0)
     WHERE id = old.message_id;
    UPDATE messages SET attachment_count = attachment_count + 1 WHERE id = new.message_id;
END;
