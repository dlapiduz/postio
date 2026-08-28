-- Whether a message's text/plain part declared `format=flowed` (RFC 3676):
-- its lines are soft-wrapped prose, not breaks the sender typed on purpose.
-- Read at reply/forward time so a message's own wrapped sentence can be
-- unwrapped rather than shown as line breaks nobody typed (#456).
ALTER TABLE messages ADD COLUMN text_is_flowed INTEGER NOT NULL DEFAULT 0;
