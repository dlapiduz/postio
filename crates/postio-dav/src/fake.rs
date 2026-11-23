use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{CalendarBackend, Changes, DavCalendar, DavError, DavObject, DavResult, Listed};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    Unreachable,
    Forbidden,
    Unauthorized,
    TokenExpired,
}

#[derive(Debug, Clone)]
struct Held {
    etag: String,
    ics: Vec<u8>,
    start: Option<DateTime<Utc>>,
    version: u64,
}

#[derive(Debug, Default)]
struct State {
    calendars: Vec<DavCalendar>,
    objects: BTreeMap<String, BTreeMap<String, Held>>,
    removed: BTreeMap<String, Vec<(String, u64)>>,
    version: u64,
    faults: Vec<Fault>,
    requests: Vec<String>,
}

/// A server in memory: every call is logged, and a queued [`Fault`] answers the next one.
#[derive(Debug, Default)]
pub struct FakeCalendars {
    state: Mutex<State>,
}

impl FakeCalendars {
    pub fn new() -> Self {
        FakeCalendars::default()
    }

    pub fn calendar(&self, href: &str, name: &str, writable: bool) -> &Self {
        let mut state = self.state.lock().expect("the fake's lock");
        state.calendars.push(DavCalendar {
            href: href.to_owned(),
            name: name.to_owned(),
            colour: None,
            writable,
        });
        state.objects.entry(href.to_owned()).or_default();
        drop(state);
        self
    }

    /// As another client would: the next `changes` reports it.
    pub fn store(&self, calendar: &str, href: &str, ics: &[u8], start: Option<DateTime<Utc>>) {
        let mut state = self.state.lock().expect("the fake's lock");
        state.version += 1;
        let version = state.version;
        state
            .objects
            .entry(calendar.to_owned())
            .or_default()
            .insert(
                href.to_owned(),
                Held {
                    etag: format!("\"v{version}\""),
                    ics: ics.to_vec(),
                    start,
                    version,
                },
            );
    }

    pub fn remove(&self, calendar: &str, href: &str) {
        let mut state = self.state.lock().expect("the fake's lock");
        state.version += 1;
        let version = state.version;
        if let Some(objects) = state.objects.get_mut(calendar) {
            objects.remove(href);
        }
        state
            .removed
            .entry(calendar.to_owned())
            .or_default()
            .push((href.to_owned(), version));
    }

    pub fn fail_next(&self, fault: Fault) {
        self.state
            .lock()
            .expect("the fake's lock")
            .faults
            .push(fault);
    }

    pub fn object(&self, calendar: &str, href: &str) -> Option<(String, Vec<u8>)> {
        let state = self.state.lock().expect("the fake's lock");
        let held = state.objects.get(calendar)?.get(href)?;
        Some((held.etag.clone(), held.ics.clone()))
    }

    pub fn requests(&self) -> Vec<String> {
        self.state.lock().expect("the fake's lock").requests.clone()
    }

    fn enter(&self, request: String) -> DavResult<std::sync::MutexGuard<'_, State>> {
        let mut state = self.state.lock().expect("the fake's lock");
        state.requests.push(request);
        match state.faults.pop() {
            None => Ok(state),
            Some(Fault::Unreachable) => Err(DavError::Unreachable("the fake is down".to_owned())),
            Some(Fault::Forbidden) => Err(DavError::Forbidden),
            Some(Fault::Unauthorized) => Err(DavError::Unauthorized),
            Some(Fault::TokenExpired) => Err(DavError::TokenExpired),
        }
    }
}

fn listed(href: &str, held: &Held) -> Listed {
    Listed {
        href: href.to_owned(),
        etag: held.etag.clone(),
    }
}

#[async_trait]
impl CalendarBackend for FakeCalendars {
    async fn calendars(&self) -> DavResult<Vec<DavCalendar>> {
        Ok(self.enter("calendars".to_owned())?.calendars.clone())
    }

    async fn listing(
        &self,
        calendar: &str,
        window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    ) -> DavResult<Vec<Listed>> {
        let state = self.enter(format!("listing {calendar} windowed={}", window.is_some()))?;
        let objects = state.objects.get(calendar).ok_or(DavError::Forbidden)?;
        Ok(objects
            .iter()
            .filter(|(_, held)| match (window, held.start) {
                (Some((from, until)), Some(start)) => from <= start && start < until,
                (Some(_), None) => false,
                (None, _) => true,
            })
            .map(|(href, held)| listed(href, held))
            .collect())
    }

    async fn fetch(&self, calendar: &str, hrefs: &[String]) -> DavResult<Vec<DavObject>> {
        assert!(
            hrefs.len() <= crate::FETCH_BATCH,
            "a fetch of {} hrefs",
            hrefs.len()
        );
        let state = self.enter(format!("fetch {calendar} {}", hrefs.len()))?;
        let objects = state.objects.get(calendar).ok_or(DavError::Forbidden)?;
        Ok(hrefs
            .iter()
            .filter_map(|href| {
                objects.get(href).map(|held| DavObject {
                    href: href.clone(),
                    etag: held.etag.clone(),
                    ics: held.ics.clone(),
                })
            })
            .collect())
    }

    async fn changes(&self, calendar: &str, token: Option<&str>) -> DavResult<Changes> {
        let state = self.enter(format!("changes {calendar} {}", token.unwrap_or("-")))?;
        let since = match token {
            None => 0,
            Some(token) => token.parse::<u64>().map_err(|_| DavError::TokenExpired)?,
        };
        let objects = state.objects.get(calendar).ok_or(DavError::Forbidden)?;
        let changed = objects
            .iter()
            .filter(|(_, held)| held.version > since)
            .map(|(href, held)| listed(href, held))
            .collect();
        let removed = state
            .removed
            .get(calendar)
            .into_iter()
            .flatten()
            .filter(|(_, version)| *version > since && since > 0)
            .map(|(href, _)| href.clone())
            .collect();
        Ok(Changes {
            changed,
            removed,
            token: Some(state.version.to_string()),
        })
    }

    async fn put(
        &self,
        calendar: &str,
        href: &str,
        ics: &[u8],
        etag: Option<&str>,
    ) -> DavResult<String> {
        let mut state = self.enter(format!("put {calendar} {href} {}", etag.unwrap_or("new")))?;
        let writable = state
            .calendars
            .iter()
            .any(|known| known.href == calendar && known.writable);
        if !writable {
            return Err(DavError::Forbidden);
        }
        let held = state
            .objects
            .get(calendar)
            .and_then(|objects| objects.get(href));
        match (held, etag) {
            (Some(_), None) => return Err(DavError::Changed),
            (Some(held), Some(etag)) if held.etag != etag => return Err(DavError::Changed),
            (None, Some(_)) => return Err(DavError::Changed),
            _ => {}
        }
        let start = held.and_then(|held| held.start);
        state.version += 1;
        let version = state.version;
        let etag = format!("\"v{version}\"");
        state
            .objects
            .entry(calendar.to_owned())
            .or_default()
            .insert(
                href.to_owned(),
                Held {
                    etag: etag.clone(),
                    ics: ics.to_vec(),
                    start,
                    version,
                },
            );
        Ok(etag)
    }

    async fn delete(&self, calendar: &str, href: &str, etag: Option<&str>) -> DavResult<()> {
        let mut state = self.enter(format!("delete {calendar} {href}"))?;
        let held = state
            .objects
            .get(calendar)
            .and_then(|objects| objects.get(href));
        match (held, etag) {
            (None, _) => return Ok(()),
            (Some(held), Some(etag)) if held.etag != etag => return Err(DavError::Changed),
            _ => {}
        }
        state.version += 1;
        let version = state.version;
        if let Some(objects) = state.objects.get_mut(calendar) {
            objects.remove(href);
        }
        state
            .removed
            .entry(calendar.to_owned())
            .or_default()
            .push((href.to_owned(), version));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, day, 9, 0, 0).unwrap()
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        futures_lite_block_on(future)
    }

    fn futures_lite_block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
        }
    }

    fn server() -> FakeCalendars {
        let fake = FakeCalendars::new();
        fake.calendar("work", "Work", true)
            .calendar("holidays", "Holidays", false);
        fake.store("work", "a.ics", b"A", Some(at(1)));
        fake.store("work", "b.ics", b"B", Some(at(20)));
        fake
    }

    #[test]
    fn a_window_lists_only_what_starts_in_it_and_no_window_lists_all() {
        let fake = server();
        let windowed = block(fake.listing("work", Some((at(1), at(10))))).expect("listed");
        assert_eq!(
            windowed.iter().map(|l| l.href.as_str()).collect::<Vec<_>>(),
            ["a.ics"]
        );
        assert_eq!(block(fake.listing("work", None)).expect("listed").len(), 2);
    }

    #[test]
    fn changes_since_a_token_name_only_what_moved_and_what_went() {
        let fake = server();
        let first = block(fake.changes("work", None)).expect("changes");
        assert_eq!(first.changed.len(), 2);
        assert!(first.removed.is_empty());

        fake.store("work", "a.ics", b"A2", Some(at(1)));
        fake.remove("work", "b.ics");
        let next = block(fake.changes("work", first.token.as_deref())).expect("changes");
        assert_eq!(
            next.changed
                .iter()
                .map(|l| l.href.as_str())
                .collect::<Vec<_>>(),
            ["a.ics"]
        );
        assert_eq!(next.removed, ["b.ics"]);
    }

    #[test]
    fn a_stale_etag_or_a_read_only_calendar_refuses_a_put() {
        let fake = server();
        let (etag, _) = fake.object("work", "a.ics").expect("held");
        let fresh = block(fake.put("work", "a.ics", b"A3", Some(&etag))).expect("put");
        assert_ne!(fresh, etag);
        assert_eq!(
            block(fake.put("work", "a.ics", b"A4", Some(&etag))),
            Err(DavError::Changed)
        );
        assert_eq!(
            block(fake.put("work", "c.ics", b"C", Some("\"x\""))),
            Err(DavError::Changed)
        );
        assert_eq!(
            block(fake.put("holidays", "h.ics", b"H", None)),
            Err(DavError::Forbidden)
        );
        assert_eq!(
            block(fake.delete("work", "gone.ics", Some("\"x\""))),
            Ok(())
        );
    }

    #[test]
    fn a_queued_fault_answers_the_next_call_and_every_call_is_logged() {
        let fake = server();
        fake.fail_next(Fault::TokenExpired);
        assert_eq!(
            block(fake.changes("work", Some("1"))),
            Err(DavError::TokenExpired)
        );
        assert!(block(fake.changes("work", Some("1"))).is_ok());
        assert_eq!(fake.requests(), ["changes work 1", "changes work 1"]);
    }
}
