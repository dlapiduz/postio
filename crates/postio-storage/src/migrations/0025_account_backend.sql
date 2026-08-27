-- The backend choice (#545, ADR 0018 Q5): which protocol adapter reaches
-- this account's server, chosen at add-account time from the preset row's
-- preference and read back by the engine every launch. 'imap' for every
-- account that predates the column — exactly what they have always used.
-- The JMAP session URL is composition data like the oauth columns above
-- it: resolved once at add time so startup needs no discovery, and never
-- a secret.

ALTER TABLE accounts ADD COLUMN backend TEXT NOT NULL DEFAULT 'imap';
ALTER TABLE accounts ADD COLUMN jmap_session_url TEXT;
