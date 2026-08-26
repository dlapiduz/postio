-- Give a message's `List-Id` a place to live.
--
-- `list:` already parses and composes with every other search operator
-- (postio-search's `Filter::List`), but `postio-index`'s executor has always
-- approximated it by matching the list's address among recipients, because
-- nothing stored the header itself -- see #9. This is that column: the
-- bracketed identifier out of RFC 2919's `"Display Name" <list-id>`, so a
-- mailing list is detected from the header a sender's own list software
-- sets, with nothing for the user to configure.
--
-- NULL rather than empty string: a message with no `List-Id` is not "in a
-- list with no name", and a row synced before this column existed is
-- honestly "not known" until the next resync, the same convention
-- `content_type` (0004) already set for this table.
ALTER TABLE messages ADD COLUMN list_id TEXT;

-- The list scope and `list:` both need to find list mail fast without a
-- full-table scan; NULL rows (the common case, most mail is not list mail)
-- are excluded, so this index is only as big as the list traffic actually is.
CREATE INDEX idx_messages_list_id ON messages (account_id, list_id) WHERE list_id IS NOT NULL;
