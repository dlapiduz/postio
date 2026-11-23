//! CalDAV for Postio: the one trait sync calls, a fake for its tests, and `io-webdav` behind it.
//! No library type crosses this crate's edge, and no store or toolkit comes in.

mod fake;
mod webdav;

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub use fake::{FakeCalendars, Fault};
pub use webdav::WebdavBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavCalendar {
    /// What every other call names the calendar by.
    pub href: String,
    pub name: String,
    pub colour: Option<String>,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub href: String,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavObject {
    pub href: String,
    pub etag: String,
    pub ics: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    pub changed: Vec<Listed>,
    pub removed: Vec<String>,
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DavError {
    #[error("the object changed on the server")]
    Changed,
    #[error("the server refused the change")]
    Forbidden,
    #[error("the server no longer knows that sync token")]
    TokenExpired,
    #[error("the server refused the credentials")]
    Unauthorized,
    #[error("the server could not be reached: {0}")]
    Unreachable(String),
    #[error("the server answered in a way Postio does not understand: {0}")]
    Protocol(String),
}

impl DavError {
    pub fn is_transient(&self) -> bool {
        matches!(self, DavError::Unreachable(_))
    }
}

pub type DavResult<T> = Result<T, DavError>;

pub const FETCH_BATCH: usize = 100;

/// `put` with `etag: None` creates and fails with [`DavError::Changed`] if the object exists; with
/// `Some`, it fails so when the server's copy is not that version. `delete` of an object already
/// gone succeeds.
#[async_trait]
pub trait CalendarBackend: Send + Sync + fmt::Debug {
    async fn calendars(&self) -> DavResult<Vec<DavCalendar>>;

    async fn listing(
        &self,
        calendar: &str,
        window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    ) -> DavResult<Vec<Listed>>;

    /// At most [`FETCH_BATCH`] hrefs; an href the server no longer has is left out.
    async fn fetch(&self, calendar: &str, hrefs: &[String]) -> DavResult<Vec<DavObject>>;

    async fn changes(&self, calendar: &str, token: Option<&str>) -> DavResult<Changes>;

    /// Returns the new ETag.
    async fn put(
        &self,
        calendar: &str,
        href: &str,
        ics: &[u8],
        etag: Option<&str>,
    ) -> DavResult<String>;

    async fn delete(&self, calendar: &str, href: &str, etag: Option<&str>) -> DavResult<()>;

    /// Each connection opened since the last call, reached or not, once.
    fn connections(&self) -> Vec<Connection>;
}
