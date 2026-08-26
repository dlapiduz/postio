-- Remember which `BODYSTRUCTURE` sections hold a message's own text.
--
-- The header sync already parses these out (`BodyStructure::text_part` and
-- `html_part`) and has always thrown them away, because until ADR 0017 the
-- backfill did not need them: it fetched `BODY.PEEK[]`, the whole message,
-- and dug the text out of the result. That is how ~90% of a mailbox's bytes
-- -- attachment payloads FTS5 cannot index -- ended up being downloaded to
-- find the ~10% that is words.
--
-- The text axis fetches `BODY.PEEK[<section>]` instead, which means it has to
-- be able to name the section. These two columns are that name.
--
-- NULL rather than a guess. A row synced before this column existed has an
-- honestly unknown structure, and defaulting to `1` would be wrong for every
-- multipart message; such a row falls back to the whole-message fetch until
-- the next resync re-reads its `BODYSTRUCTURE`. Same convention as
-- `content_type` (0004) and `list_id` (0007).
--
-- The `_headers` columns are the part's MIME header block, rebuilt from what
-- `BODYSTRUCTURE` said about it: content type, charset, transfer encoding.
-- They exist because `BODY[1.1]` returns a part's *encoded* bytes with no
-- headers of their own, and base64 with no `Content-Transfer-Encoding` to
-- explain it is not text. Prepending these turns the fetched section back into
-- a self-contained entity `mime::parse` can decode, at the cost of about
-- twenty bytes a message and without a second round trip for `[1.1.MIME]`.
--
-- No index: these are only ever read by id, on the row the backfill has
-- already selected.
ALTER TABLE messages ADD COLUMN text_part_id TEXT;
ALTER TABLE messages ADD COLUMN text_part_headers TEXT;
ALTER TABLE messages ADD COLUMN html_part_id TEXT;
ALTER TABLE messages ADD COLUMN html_part_headers TEXT;
