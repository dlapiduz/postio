ALTER TABLE markers ADD COLUMN moved_from INTEGER;

-- The calendar (spec 017): "On this computer" and each account's CalDAV
-- calendars, their events as the iCalendar objects the server or Postio
-- wrote, and the occurrences views page over.

CREATE TABLE calendars (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL for "On this computer", which never syncs.
    account_id            INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    href                  TEXT,
    name                  TEXT    NOT NULL,
    colour                TEXT,
    writable              INTEGER NOT NULL DEFAULT 1,
    shown                 INTEGER NOT NULL DEFAULT 1,
    -- The server answers invitations itself (RFC 6638), so an answer is a
    -- PUT and never a mail.
    schedules             INTEGER NOT NULL DEFAULT 0,
    default_alarm_minutes INTEGER,
    sync_token            TEXT,
    synced_at             INTEGER,
    -- A fixed vocabulary, never a server's body.
    error                 TEXT,
    UNIQUE (account_id, href)
);

CREATE TABLE events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    calendar_id       INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    uid               TEXT    NOT NULL,
    href              TEXT,
    etag              TEXT,
    -- Kept whole, so an edit changes only what Postio models.
    object            BLOB    NOT NULL,
    title             TEXT    NOT NULL,
    location          TEXT    NOT NULL DEFAULT '',
    -- The master's first instant, UTC ms, for ordering only.
    starts_at         INTEGER NOT NULL,
    recurs            INTEGER NOT NULL DEFAULT 0,
    status            TEXT    NOT NULL CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
    my_partstat       TEXT,
    source_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    UNIQUE (calendar_id, uid)
);

-- Expanded from a year back to two years ahead; paging past either end
-- expands more. A floating time is stored as its wall clock read as UTC.
CREATE TABLE event_occurrences (
    event_id      INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    start_utc     INTEGER NOT NULL,
    end_utc       INTEGER NOT NULL,
    all_day       INTEGER NOT NULL,
    floating      INTEGER NOT NULL,
    recurrence_id TEXT,
    PRIMARY KEY (event_id, start_utc)
);

CREATE INDEX idx_event_occurrences_start ON event_occurrences (start_utc, event_id);

CREATE TABLE event_changes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id    INTEGER REFERENCES events(id) ON DELETE SET NULL,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('create', 'update', 'delete')),
    -- What the change was made against, for the three-way merge.
    base_object BLOB,
    base_etag   TEXT,
    state       TEXT    NOT NULL CHECK (state IN ('queued', 'sent', 'conflict', 'failed')),
    created_at  INTEGER NOT NULL
);

CREATE INDEX idx_event_changes_state ON event_changes (state, id);
CREATE INDEX idx_event_changes_event ON event_changes (event_id, state);

-- An alarm's due instant for one occurrence. Not `reminders`, which is
-- mail's remind-if-no-reply.
CREATE TABLE event_reminders (
    event_id             INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    occurrence_start_utc INTEGER NOT NULL,
    due_at               INTEGER NOT NULL,
    state                TEXT    NOT NULL CHECK (state IN ('pending', 'fired', 'snoozed', 'missed')),
    PRIMARY KEY (event_id, occurrence_start_utc, due_at)
);

CREATE INDEX idx_event_reminders_due ON event_reminders (state, due_at);

INSERT INTO calendars (account_id, href, name) VALUES (NULL, NULL, 'On this computer');

-- The egress log gains 'caldav'; a CHECK cannot be altered, so the table
-- is rebuilt with its rows.
CREATE TABLE egress_log_rows AS SELECT * FROM egress_log;
DROP INDEX idx_egress_log_at;
DROP TABLE egress_log;
CREATE TABLE egress_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          INTEGER NOT NULL,
    -- 'model' is the person's own model runtime, on this computer (spec 007
    -- FR-168): every call Focus makes to it, by where and outcome.
    subsystem   TEXT    NOT NULL CHECK (subsystem IN ('imap', 'smtp', 'discovery', 'model', 'caldav')),
    -- NULL before an account exists: discovery during onboarding probes
    -- servers for an account not yet created.
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    host        TEXT    NOT NULL,
    port        INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('connected', 'failed'))
);
INSERT INTO egress_log (id, at, subsystem, account_id, host, port, outcome)
     SELECT id, at, subsystem, account_id, host, port, outcome FROM egress_log_rows;
DROP TABLE egress_log_rows;
CREATE INDEX idx_egress_log_at ON egress_log (at DESC);
