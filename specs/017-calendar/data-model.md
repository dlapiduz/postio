# Data model: Calendar

The spec's "Key Entities", made concrete. Tables go in
`crates/postio-storage/src/schema.rs`'s one declared `HEAD`; a store an
older schema wrote is resynced (`PRODUCT.md` §6). No compatibility shims.

## Types

### `postio-calendar` (pure leaf; grows)

```text
Event {
    uid: String,
    recurrence_id: Option<EventTime>,   // set on an exception (an overridden occurrence)
    title: String,
    start: EventTime, end: EventTime,   // EventTime and Zone are the existing types
    all_day: bool,
    rule: Option<Recurrence>,           // RRULE, RDATE, EXDATE, as the library models them
    location: String, notes: String,
    organiser: Option<Attendee>, attendees: Vec<Attendee>,   // Attendee and PartStat are existing
    alarms: Vec<Alarm>,
    sequence: u32, dtstamp: Instant,
    status: Status,                     // Confirmed | Tentative | Cancelled
}
Alarm { trigger: Trigger, action: AlarmAction }   // Display | Audio | Email (Email is never acted on)
Trigger = Before(Duration) | After(Duration) | At(Instant)
Occurrence { uid, recurrence_id: Option<EventTime>, start_utc: Instant, end_utc: Instant, all_day, floating: bool }

parse_object(ics: &[u8]) -> Result<Vec<Event>, CalendarError>     // a VCALENDAR with a master and exceptions
expand(events: &[Event], window: Range<Instant>, zone: &Zone) -> Vec<Occurrence>
patch(ics: &[u8], change: &Change) -> Vec<u8>                        // byte-faithful: only modelled fields change
merge(base: &[u8], ours: &[u8], theirs: &[u8]) -> Merge              // Clean(Vec<u8>) | Conflict { ours, theirs }
request(event: &Event, method: Method) -> Vec<u8>                    // iTIP REQUEST, CANCEL; `reply` exists
Change = Fields(FieldEdits) | ThisOccurrence(FieldEdits) | ThisAndFollowing(FieldEdits) | Delete(Scope)
Scope = This | ThisAndFollowing | All
```

`expand` is idempotent over a window. `patch` and `merge` keep every
property and component Postio does not model byte for byte (spec FR-014).
The library behind them is S1's choice. Its types never cross the crate's
edge, as today.

### `postio-dav` (new protocol crate, slice C)

```text
trait CalendarBackend {
    async fn discover(&self, account: &CalendarAccount) -> Result<Vec<RemoteCalendar>, DavError>;
    async fn sync(&self, calendar: &RemoteCalendar, token: Option<&SyncToken>) -> Result<SyncPage, DavError>;
    async fn fetch(&self, calendar: &RemoteCalendar, hrefs: &[Href]) -> Result<Vec<RemoteObject>, DavError>;
    async fn put(&self, object: &OutgoingObject, precondition: Precondition) -> Result<ETag, DavError>;
    async fn delete(&self, href: &Href, etag: &ETag) -> Result<(), DavError>;
}
RemoteCalendar { href, display_name, colour: Option<String>, writable: bool, components: Vec<String>, schedules: bool }
SyncPage { changed: Vec<(Href, ETag)>, removed: Vec<Href>, token: Option<SyncToken>, more: bool }
Precondition = IfMatch(ETag) | IfNoneMatch
DavError = Auth | NotFound | PreconditionFailed | Forbidden | Server(u16) | Network(String)
```

`postio-sync` sees only the trait. A fake implements it for every sync test,
and an in-process wire server tests the real one.

### `postio-ui::calendar` (view models)

```text
AgendaRow = DayHeading { date, label } | Occurrence { id, time_label, title, calendar_name, colour_mark, location, chip: Option<Chip>, queued: bool }
Chip = Invite | Tentative | Declined | Clashes(String) | Moved(String) | Cancelled
EventView { title, when_label, zone_label: Option<String>, repeat_label: Option<String>, location, people: Vec<PersonRow>, notes, calendar_name, reminders: Vec<String>, verbs: Vec<CommandId> }
CalendarStrip = Synced { last } | Syncing { calendar, done, total } | Offline { waiting } | Failing { calendar, error, retry_key }
```

### `postio-focus` (behaviour)

```text
CalendarMode { view: View, date: Date, cursor: Option<OccurrenceId>, history: History }
View = Agenda | Day | Week | Month        // Day, Week and Month are held for the designer handoff
SurfaceHost = MainWindow | OwnWindow(WindowId)   // pop out moves CalendarMode between hosts, unchanged
```

## Tables

```sql
CREATE TABLE calendars (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER REFERENCES accounts(id) ON DELETE CASCADE,   -- NULL: "On this computer"
    href          TEXT,                                                -- NULL for the local calendar
    name          TEXT    NOT NULL,
    colour        TEXT,
    writable      INTEGER NOT NULL DEFAULT 1,
    shown         INTEGER NOT NULL DEFAULT 1,
    schedules     INTEGER NOT NULL DEFAULT 0,                          -- from the preset row, then discovery
    default_alarm_minutes INTEGER,
    sync_token    TEXT,
    synced_at     INTEGER,
    error         TEXT,                                                -- a fixed vocabulary, never a server's body
    UNIQUE (account_id, href)
);
CREATE TABLE events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    calendar_id   INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    uid           TEXT    NOT NULL,
    href          TEXT,
    etag          TEXT,
    object        BLOB    NOT NULL,     -- the iCalendar object as the server or Postio wrote it (zstd when smaller)
    title         TEXT    NOT NULL,
    starts_at     INTEGER NOT NULL,     -- the master's first instant, for ordering only
    recurs        INTEGER NOT NULL DEFAULT 0,
    status        TEXT    NOT NULL CHECK (status IN ('confirmed', 'tentative', 'cancelled')),
    my_partstat   TEXT,                 -- the person's own answer, when invited
    source_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,   -- the invitation it came from
    UNIQUE (calendar_id, uid)
);
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
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id      INTEGER REFERENCES events(id) ON DELETE SET NULL,
    calendar_id   INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL CHECK (kind IN ('create', 'update', 'delete')),
    base_object   BLOB,                 -- what the change was made against, for the three-way merge
    base_etag     TEXT,
    state         TEXT    NOT NULL CHECK (state IN ('queued', 'sent', 'conflict', 'failed')),
    created_at    INTEGER NOT NULL
);
CREATE TABLE reminders (
    event_id      INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    occurrence_start_utc INTEGER NOT NULL,
    due_at        INTEGER NOT NULL,
    state         TEXT    NOT NULL CHECK (state IN ('pending', 'fired', 'snoozed', 'missed')),
    PRIMARY KEY (event_id, occurrence_start_utc, due_at)
);
CREATE INDEX idx_reminders_due ON reminders (state, due_at);
CREATE TABLE event_search (rowid_event INTEGER PRIMARY KEY, body TEXT NOT NULL);   -- folded title, location, notes, names
CREATE INDEX event_search_fts ON event_search USING fts (body);
```

- `egress_log.subsystem` gains `'caldav'`.
- `settings` gains the `calendar.window` row `{ popped, width, height }`
  (pop out, plan R9). No position is stored.
- The `operation_queue` carries `calendar_put` and `calendar_delete`
  operations naming an `event_changes` row. The queue drain in `postio-sync`
  calls `CalendarBackend`.

## Windows and paging

- **Occurrences** are materialised from a year back to two years ahead.
  Paging past either end expands more. A changed event re-expands only its
  own rows.
- **Agenda** pages by `(start_utc, event_id)` keyset over
  `idx_event_occurrences_start`, joined to `events` and `calendars` by
  primary key. That is a fixed number of statements per page, gated as
  counts.

## States

```text
Event change:  verb ─▶ store write + event_changes(queued) ─▶ emit ─▶ repaint
               drain ─▶ put/delete ─▶ 2xx: etag stored, change removed
                                   ─▶ 412: fetch theirs ─▶ merge ─▶ Clean: put again
                                                                  └▶ Conflict: state = conflict (both versions kept)
                                   ─▶ 403/4xx: state = failed ("Retry" / "Discard my change")
Reminder:      pending ─(due, app holds the store)─▶ fired
               pending ─(due while closed, < 12 h)─▶ missed ─(shown once at start)─▶ fired
               fired ─(Snooze 5 min)─▶ snoozed(due + 5 min) ─▶ fired
RSVP:          y/Y/alt+y ─▶ my_partstat written ─▶ calendar.schedules ? put(updated object) : outbox iTIP REPLY
Pop out:       mode in MainWindow ─pop_out─▶ OwnWindow ─pop_in or close─▶ MainWindow (CalendarMode unchanged)
```
