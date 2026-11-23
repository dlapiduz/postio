# Calendar fixtures

Synthetic iCalendar objects for `postio-calendar`'s tests. Every address is
under a reserved domain and every name is invented. Lines end in CRLF, as RFC
5545 asks; keep them so when editing.

The instants are the reference the expansion is held to: both calcard and
ical-rs gave exactly these.

| File | What it exercises | Occurrences (UTC) |
|---|---|---|
| `weekly-dst-exception.ics` | A weekly rule in New York across the end of daylight time, two `EXDATE`s, and one occurrence moved by `RECURRENCE-ID` | 10-06 13:30, 10-13 13:30, 10-27 18:00 (moved), 11-03 14:30, 11-17 14:30, 11-24 14:30 |
| `all-day-multi-day.ics` | An all-day event over three days | 12-14 to 12-17, as dates |
| `floating.ics` | A daily rule at a floating 07:00 | 12-03, 12-04, 12-05 at 07:00 on the reader's clock |
| `windows-zone.ics` | A Windows zone name with no `VTIMEZONE`, across the end of US daylight time | 10-30 16:00, 11-06 17:00 |
| `zone-without-vtimezone.ics` | An IANA `TZID` the object does not define, across the end of European summer time | 10-24 09:00, 10-31 10:00 |
| `unknown-properties.ics` | `X-` properties and parameters, a folded line, `CATEGORIES`, and a `VTODO`: all kept byte for byte through an edit | 11-07 10:00 |
| `email-alarm.ics` | An `EMAIL` alarm (never acted on), a `DISPLAY` alarm 10 minutes before, and an `AUDIO` alarm at an absolute time | 11-12 15:00 |
| `hostile-urls.ics` | URLs in `SUMMARY`, `DESCRIPTION`, `LOCATION`, `URL`, `ATTACH`, `CONFERENCE` and `IMAGE`: none may be fetched | 11-18 16:00 |
