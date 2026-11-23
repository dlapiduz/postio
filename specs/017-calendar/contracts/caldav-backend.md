# Contract: CalDAV sync and presets

The spec's D11 and FR-030 to FR-035 and FR-052; plan R3 and R5.

## The preset row (`crates/postio-account/data/providers.toml`)

```toml
[provider.icloud.caldav]
url = "https://caldav.icloud.com/"
schedules = true

[provider.fastmail.caldav]
url = "https://caldav.fastmail.com/"
schedules = true

# Second slice (Q3 = A):
[provider.gmail.caldav]
url = "https://apidata.googleusercontent.com/caldav/v2/"
schedules = false                       # until spike S3 says otherwise
oauth_scopes = ["https://www.googleapis.com/auth/calendar"]
```

- The schema is in `crates/postio-account/src/discovery/providers_toml.rs`,
  and `build.rs` validates it. `url` is absent for RFC 6764 discovery.
- `oauth_scopes` are requested only when the person turns that account's
  calendars on.
- A provider with no `[caldav]` table and no discovery result offers "Enter a
  calendar address" in Settings.

## Discovery and sync (`postio-dav`, tested on an in-process wire server)

| Step | Request | Asserted |
|---|---|---|
| context root | the preset's `url`, else `io-pim-discovery` RFC 6764 (SRV, TXT, `/.well-known/caldav`) | a redirect is surfaced, and is followed only to the same registrable domain |
| principal | `PROPFIND current-user-principal` | — |
| home | `PROPFIND calendar-home-set` | — |
| calendars | `PROPFIND` with depth 1: display name, colour, `supported-calendar-component-set`, `current-user-privilege-set` | non-VEVENT collections skipped; read-only ones have `writable = false` |
| first sync | `REPORT calendar-query` over the window, then `calendar-multiget` in batches of 100 | the window first, then the rest in the background |
| incremental | `REPORT sync-collection` with the token; without one, an ETag listing | only changed objects are fetched |
| write | `PUT` with `If-Match`, or `If-None-Match: *` | 412 leads to a fetch and a merge |
| delete | `DELETE` with `If-Match` | 404 counts as done |

## Egress and credentials

- Every connection is an `egress_log` row with `subsystem = 'caldav'`, its
  account, host, port and outcome.
- Credentials are the account's own, from the keyring: the app password, or
  the OAuth token with the calendar scope. Neither is written to the store or
  a log.

## Tests

- **Fake backend**: every `postio-sync` calendar case.
- **The wire server** (`crates/postio-dav/tests/dav_suite/`): discovery,
  sync, 412, 403, and token expiry (`valid-sync-token` precondition failure
  leads to a full resync of that calendar).
- **Live servers**: `#[ignore]` only, with credentials from the environment,
  and never in CI's default tier.
