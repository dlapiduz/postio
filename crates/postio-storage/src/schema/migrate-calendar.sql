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

-- Markers gain the tentative answers and where an update moved an event; a
-- CHECK cannot be altered, so the table is rebuilt with its rows.
CREATE TABLE markers_rows AS SELECT * FROM markers;
DROP INDEX idx_markers_answer_until;
DROP INDEX idx_markers_invite;
DROP INDEX idx_markers_dismissed;
DROP TABLE markers;
CREATE TABLE markers (
    message_id       INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    -- What the row calls out. `no_reply` is a fired reminder's.
    kind             TEXT    NOT NULL
                             CHECK (kind IN ('invite', 'question', 'todo', 'no_reply')),
    -- What made it: a calendar part, the built-in detector, the person's
    -- own model, or a reminder.
    source           TEXT    NOT NULL
                             CHECK (source IN ('calendar', 'detector', 'model', 'reminder')),
    -- Character offsets of the sentence in the message's own text; NULL for
    -- an invitation or a reminder.
    span_start       INTEGER,
    span_end         INTEGER,
    -- The sentence, verbatim, at most 200 characters; NULL for an invitation.
    excerpt          TEXT,
    -- An invitation's event, UTC milliseconds.
    starts_at        INTEGER,
    ends_at          INTEGER,
    -- A to-do's due date; for `no_reply`, the day the reminder was set.
    due_at           INTEGER,
    -- The invitation's iTIP identity: which later REQUEST or CANCEL
    -- replaces this marker (research R9).
    invite_uid       TEXT,
    invite_sequence  INTEGER,
    invite_stamp     INTEGER,
    invite_state     TEXT    CHECK (invite_state IN ('open', 'cancelled', 'past')),
    -- How the person answered; the `-ing` two last while the reply's window
    -- is open.
    answer           TEXT    CHECK (answer IN ('accepting', 'accepted',
                                               'declining', 'declined',
                                               'tentative_pending', 'tentative')),
    -- While an answer's reply waits out its window in the outbox, when the
    -- window closes (research R9): Focus's due timer then makes the answer
    -- final. NULL otherwise.
    answer_until     INTEGER,
    -- When the person dismissed it. It never returns on this message.
    dismissed_at     INTEGER,
    -- An update moved the event the person had answered: where it was.
    moved_from       INTEGER
);
INSERT INTO markers (message_id, kind, source, span_start, span_end, excerpt, starts_at, ends_at,
                     due_at, invite_uid, invite_sequence, invite_stamp, invite_state, answer,
                     answer_until, dismissed_at)
     SELECT message_id, kind, source, span_start, span_end, excerpt, starts_at, ends_at, due_at,
            invite_uid, invite_sequence, invite_stamp, invite_state, answer, answer_until,
            dismissed_at
       FROM markers_rows;
DROP TABLE markers_rows;
CREATE INDEX idx_markers_answer_until ON markers (answer_until);
CREATE INDEX idx_markers_invite ON markers (invite_uid);
CREATE INDEX idx_markers_dismissed ON markers (dismissed_at, message_id);
