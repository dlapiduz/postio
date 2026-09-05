-- Whether the stored body is what was sent, or the best guess available.

-- Three degradations are correct-but-lossy, and every one of them is
-- invisible without this: base64 outside its alphabet comes back as the raw
-- base64 text, an unknown Content-Transfer-Encoding is shown verbatim per RFC
-- 2045 §6.4, and a charset that decoded lossily leaves U+FFFD where octets
-- were. Each is the right degradation -- a body beats no body -- and each
-- reads exactly like a message that simply said that.
--
-- On the row rather than beside the text for the same reason
-- `body_headers_truncated` is: the body and the fact that it is a guess are
-- one piece of information, and a writer able to set them apart eventually
-- sets one. `StoredBody` carries both, so there is no way to store a body
-- without answering this.
--
-- Rows written before this column take the default and are not reparsed; a
-- resync answers them.
ALTER TABLE messages ADD COLUMN body_encoding_problems INTEGER NOT NULL DEFAULT 0;
