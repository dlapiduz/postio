use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use io_webdav::client::{WebdavClientStd, WebdavClientStdConnectOptions, WebdavClientStdError};
use io_webdav::rfc4918::WebdavAuth;
use io_webdav::rfc4918::report::WebdavReport;
use io_webdav::rfc4918::send::WebdavSendError;
use io_webdav::rfc6578::sync_collection::{WebdavSyncCollectionError, WebdavSyncCollectionOptions};

use crate::{
    CalendarBackend, Changes, Connection, DavCalendar, DavError, DavObject, DavResult, Listed,
};

pub struct WebdavBackend {
    url: url::Url,
    user: String,
    secret: String,
    client: Arc<Mutex<Option<WebdavClientStd>>>,
    opened: Arc<Mutex<Vec<Connection>>>,
}

impl std::fmt::Debug for WebdavBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebdavBackend")
            .field("host", &self.url.host_str())
            .finish_non_exhaustive()
    }
}

impl WebdavBackend {
    pub fn new(url: &str, user: &str, secret: &str) -> DavResult<Self> {
        let url = url::Url::parse(url).map_err(|error| DavError::Protocol(error.to_string()))?;
        Ok(WebdavBackend {
            url,
            user: user.to_owned(),
            secret: secret.to_owned(),
            client: Arc::new(Mutex::new(None)),
            opened: Arc::default(),
        })
    }

    async fn with<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut WebdavClientStd) -> Result<T, WebdavClientStdError> + Send + 'static,
    ) -> DavResult<T> {
        let (client, url, opened_log) = (
            Arc::clone(&self.client),
            self.url.clone(),
            Arc::clone(&self.opened),
        );
        let auth = WebdavAuth::Basic(io_http::rfc7617::basic::HttpAuthBasic::new(
            &self.user,
            &self.secret,
        ));
        tokio::task::spawn_blocking(move || {
            let mut held = client
                .lock()
                .map_err(|_| DavError::Protocol("a poisoned client".to_owned()))?;
            if held.is_none() {
                let connection = Connection {
                    host: url.host_str().unwrap_or_default().to_owned(),
                    port: url.port_or_known_default().unwrap_or(443),
                    reached: false,
                };
                let opened =
                    WebdavClientStd::connect(&url, auth, WebdavClientStdConnectOptions::default())
                        .and_then(|mut opened| {
                            opened.current_user_principal()?;
                            opened.calendar_home_set()?;
                            Ok(opened)
                        });
                if let Ok(mut log) = opened_log.lock() {
                    log.push(Connection {
                        reached: opened.is_ok(),
                        ..connection
                    });
                }
                *held = Some(opened.map_err(|error| match failure(error) {
                    DavError::Protocol(why) => DavError::Unreachable(why),
                    known => known,
                })?);
            }
            let outcome = work(held.as_mut().expect("opened above"));
            if matches!(&outcome, Err(WebdavClientStdError::Io(_))) {
                *held = None;
            }
            outcome.map_err(failure)
        })
        .await
        .map_err(|error| DavError::Protocol(error.to_string()))?
    }
}

fn failure(error: WebdavClientStdError) -> DavError {
    let status = |error: &WebdavSendError| match error {
        WebdavSendError::HttpStatus { status, .. } => Some(*status),
        _ => None,
    };
    match &error {
        WebdavClientStdError::WebdavSyncCollection(WebdavSyncCollectionError::InvalidSyncToken) => {
            DavError::TokenExpired
        }
        WebdavClientStdError::Send(send) => match status(send) {
            Some(412) => DavError::Changed,
            Some(401) => DavError::Unauthorized,
            Some(403) => DavError::Forbidden,
            _ => DavError::Protocol(error.to_string()),
        },
        WebdavClientStdError::Io(io) => DavError::Unreachable(io.kind().to_string()),
        _ => DavError::Protocol(error.to_string()),
    }
}

fn last_segment(href: &str) -> String {
    href.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href)
        .to_owned()
}

fn unquoted(etag: Option<String>) -> String {
    etag.unwrap_or_default().trim_matches('"').to_owned()
}

fn query(window: Option<(DateTime<Utc>, DateTime<Utc>)>) -> String {
    let range = window.map_or(String::new(), |(from, until)| {
        format!(
            "<c:time-range start=\"{}\" end=\"{}\"/>",
            from.format("%Y%m%dT%H%M%SZ"),
            until.format("%Y%m%dT%H%M%SZ")
        )
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-query xmlns:d=\"DAV:\" \
         xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag/></d:prop><c:filter>\
         <c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\">{range}\
         </c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"
    )
}

#[async_trait]
impl CalendarBackend for WebdavBackend {
    async fn calendars(&self) -> DavResult<Vec<DavCalendar>> {
        self.with(|client| {
            Ok(client
                .list_calendars()?
                .into_iter()
                .filter(|calendar| {
                    calendar.components.is_empty() || calendar.components.contains("VEVENT")
                })
                .map(|calendar| DavCalendar {
                    name: calendar
                        .display_name
                        .clone()
                        .unwrap_or_else(|| calendar.id.clone()),
                    href: calendar.id,
                    colour: calendar.color,
                    writable: true,
                })
                .collect())
        })
        .await
    }

    async fn listing(
        &self,
        calendar: &str,
        window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    ) -> DavResult<Vec<Listed>> {
        let calendar = calendar.to_owned();
        self.with(move |client| {
            let home = client.calendar_home_set()?;
            let path = format!(
                "{}{calendar}/",
                home.path().trim_end_matches('/').to_owned() + "/"
            );
            let report: io_webdav::rfc4918::WebdavMultistatus = client.run(WebdavReport::new(
                &home,
                client.auth(),
                "postio",
                &path,
                1,
                query(window).into_bytes(),
            ))?;
            Ok(report
                .responses
                .into_iter()
                .filter(|entry| !entry.href.ends_with('/'))
                .map(|entry| Listed {
                    etag: entry
                        .props
                        .iter()
                        .find(|prop| prop.local == "getetag")
                        .map(|prop| prop.text.trim().trim_matches('"').to_owned())
                        .unwrap_or_default(),
                    href: last_segment(&entry.href),
                })
                .collect())
        })
        .await
    }

    async fn fetch(&self, calendar: &str, hrefs: &[String]) -> DavResult<Vec<DavObject>> {
        let (calendar, hrefs) = (calendar.to_owned(), hrefs.to_vec());
        self.with(move |client| {
            let ids: Vec<&str> = hrefs.iter().map(String::as_str).collect();
            Ok(client
                .multiget_items(&calendar, &ids)?
                .items
                .into_iter()
                .map(|item| DavObject {
                    href: last_segment(&item.href),
                    etag: unquoted(item.etag),
                    ics: item.data,
                })
                .collect())
        })
        .await
    }

    async fn changes(&self, calendar: &str, token: Option<&str>) -> DavResult<Changes> {
        let (calendar, token) = (calendar.to_owned(), token.map(str::to_owned));
        self.with(move |client| {
            let delta = client.sync_items(
                &calendar,
                token.as_deref(),
                WebdavSyncCollectionOptions::default(),
            )?;
            Ok(Changes {
                changed: delta
                    .changed
                    .into_iter()
                    .map(|change| Listed {
                        href: last_segment(&change.href),
                        etag: unquoted(change.etag),
                    })
                    .collect(),
                removed: delta
                    .vanished
                    .iter()
                    .map(|href| last_segment(href))
                    .collect(),
                token: delta.sync_token,
            })
        })
        .await
    }

    async fn put(
        &self,
        calendar: &str,
        href: &str,
        ics: &[u8],
        etag: Option<&str>,
    ) -> DavResult<String> {
        let (calendar, href, ics) = (calendar.to_owned(), href.to_owned(), ics.to_vec());
        let etag = etag.map(|etag| format!("\"{}\"", etag.trim_matches('"')));
        self.with(move |client| {
            Ok(match etag {
                None => unquoted(client.create_item(&calendar, &href, ics)?.etag),
                Some(etag) => {
                    unquoted(client.update_item(&calendar, &href, ics, Some(&etag))?.etag)
                }
            })
        })
        .await
    }

    fn connections(&self) -> Vec<Connection> {
        self.opened
            .lock()
            .map(|mut log| std::mem::take(&mut *log))
            .unwrap_or_default()
    }

    async fn delete(&self, calendar: &str, href: &str, etag: Option<&str>) -> DavResult<()> {
        let (calendar, href) = (calendar.to_owned(), href.to_owned());
        let etag = etag.map(|etag| format!("\"{}\"", etag.trim_matches('"')));
        self.with(
            move |client| match client.delete_item(&calendar, &href, etag.as_deref()) {
                Err(WebdavClientStdError::Send(WebdavSendError::HttpStatus {
                    status: 404,
                    ..
                })) => Ok(()),
                other => other,
            },
        )
        .await
    }
}
