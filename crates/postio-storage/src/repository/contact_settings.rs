//! What the user decided about a correspondent (specs/011-focus-contacts S3,
//! ADR 0048): where their mail arrives, which markers are off, how their HTML
//! mail is drawn.
//!
//! The engine reads every live person's decisions in one statement
//! ([`ContactSettingsRepository::derived`]); a reader that looks a sender up
//! by address uses [`ContactSettingsRepository::treatment_for`].

use postio_model::{ContactArrival, ContactId, ContactSettings, DigestSetting};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// One person's decisions with the addresses they apply to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSettings {
    /// Who.
    pub contact: ContactId,
    /// What the person is called: the name set, else the mail's, else their
    /// address.
    pub name: String,
    /// Every address they own, lowercased.
    pub addresses: Vec<String>,
    /// What they decided.
    pub settings: ContactSettings,
}

/// Reads and writes a correspondent's decisions.
#[derive(Debug)]
pub struct ContactSettingsRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "arrival, digest_name, digest_cadence, digest_day, digest_at, \
     stop_markers, notify, html_treatment, remind_days, archive_after_days, ignored";

impl<'a> ContactSettingsRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// What `contact` decided; the default when they decided nothing.
    pub async fn get(&self, contact: ContactId) -> Result<ContactSettings> {
        Ok(sql::first(
            self.connection,
            &format!("SELECT {COLUMNS} FROM contact_settings WHERE contact_id = ?1"),
            [contact.get()],
            |row| read(row, 0),
        )
        .await?
        .unwrap_or_default())
    }

    /// Records what `contact` decided. A person who decided nothing keeps no
    /// row.
    pub async fn put(&self, contact: ContactId, settings: &ContactSettings) -> Result<()> {
        if settings.is_default() {
            sql::execute(
                self.connection,
                "DELETE FROM contact_settings WHERE contact_id = ?1",
                [contact.get()],
            )
            .await?;
            return Ok(());
        }
        let digest = settings.digest.as_ref();
        sql::execute(
            self.connection,
            "INSERT INTO contact_settings
                 (contact_id, arrival, digest_name, digest_cadence, digest_day, digest_at,
                  stop_markers, notify, html_treatment, remind_days, archive_after_days, ignored)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (contact_id) DO UPDATE SET
                 arrival = excluded.arrival, digest_name = excluded.digest_name,
                 digest_cadence = excluded.digest_cadence, digest_day = excluded.digest_day,
                 digest_at = excluded.digest_at, stop_markers = excluded.stop_markers,
                 notify = excluded.notify, html_treatment = excluded.html_treatment,
                 remind_days = excluded.remind_days,
                 archive_after_days = excluded.archive_after_days,
                 ignored = excluded.ignored",
            bind![
                contact.get(),
                settings.arrival.map(ContactArrival::as_str),
                digest.map(|d| d.name.clone()),
                digest.map(|d| d.cadence.clone()),
                digest.and_then(|d| d.day.clone()),
                digest.map(|d| d.at.clone()),
                i64::from(settings.stop_markers),
                i64::from(settings.notify),
                settings.html_treatment.clone(),
                settings.remind_days.map(i64::from),
                settings.archive_after_days.map(i64::from),
                i64::from(settings.ignored),
            ],
        )
        .await?;
        Ok(())
    }

    /// Every live person's rules (a pin, a digest, a marker turned off) with
    /// the addresses they apply to: what the engine reads. One statement.
    /// A treatment alone is not a rule and is not here.
    pub async fn derived(&self) -> Result<Vec<DerivedSettings>> {
        let rows = sql::all_unbounded(self.connection, Self::explain_derived(), (), |row| {
            Ok((
                row.col::<i64>(0)?,
                row.col::<Option<String>>(1)?,
                row.col::<Option<String>>(2)?,
                row.col::<Option<String>>(3)?,
                row.col::<String>(4)?,
                read(row, 5)?,
            ))
        })
        .await?;
        let mut people: Vec<DerivedSettings> = Vec::new();
        for (contact, name, seen, preferred, address, settings) in rows {
            let contact = ContactId::new(contact);
            match people.last_mut().filter(|last| last.contact == contact) {
                Some(last) => last.addresses.push(address),
                None => people.push(DerivedSettings {
                    contact,
                    name: [name, seen, preferred]
                        .into_iter()
                        .flatten()
                        .map(|n| n.trim().to_owned())
                        .find(|n| !n.is_empty())
                        .unwrap_or_else(|| address.clone()),
                    addresses: vec![address],
                    settings,
                }),
            }
        }
        Ok(people)
    }

    /// The SQL [`Self::derived`] runs.
    pub fn explain_derived() -> &'static str {
        "SELECT c.id, c.name, c.seen_name, p.address, a.address_normalized,
                s.arrival, s.digest_name, s.digest_cadence, s.digest_day, s.digest_at,
                s.stop_markers, s.notify, s.html_treatment, s.remind_days, s.archive_after_days
           FROM contact_settings s
           CROSS JOIN contacts c ON c.id = s.contact_id AND c.state = 'live'
           CROSS JOIN addresses a ON a.contact_id = c.id
           LEFT JOIN addresses p ON p.id = c.preferred_address
          WHERE s.arrival IS NOT NULL OR s.stop_markers <> 0
          ORDER BY c.id, a.id"
    }

    /// How `address`'s mail is always drawn, if the person it belongs to
    /// decided: `app` or `paper`.
    pub async fn treatment_for(&self, address: &str) -> Result<Option<String>> {
        sql::first(
            self.connection,
            "SELECT s.html_treatment FROM addresses a
               JOIN contact_settings s ON s.contact_id = a.contact_id
              WHERE a.address_normalized = ?1 AND s.html_treatment IS NOT NULL",
            [address.trim().to_lowercase()],
            |row| row.col::<String>(0),
        )
        .await
    }

    /// Every address with a treatment, for a reader that looks senders up
    /// from memory. One statement.
    pub async fn treatments(&self) -> Result<Vec<(String, String)>> {
        sql::all_unbounded(
            self.connection,
            "SELECT a.address_normalized, s.html_treatment FROM contact_settings s
               JOIN addresses a ON a.contact_id = s.contact_id
              WHERE s.html_treatment IS NOT NULL
              ORDER BY a.address_normalized",
            (),
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await
    }
}

/// The settings columns, starting at `at`.
fn read(row: &turso::Row, at: usize) -> Result<ContactSettings> {
    let arrival: Option<String> = row.col(at)?;
    let arrival = arrival
        .map(|name| {
            ContactArrival::from_name(&name).ok_or(Error::UnknownEnum {
                column: "contact_settings.arrival",
                value: name,
            })
        })
        .transpose()?;
    let name: Option<String> = row.col(at + 1)?;
    let cadence: Option<String> = row.col(at + 2)?;
    let day: Option<String> = row.col(at + 3)?;
    let time: Option<String> = row.col(at + 4)?;
    let digest = match (name, cadence, time) {
        (Some(name), Some(cadence), Some(time)) => Some(DigestSetting {
            name,
            cadence,
            day,
            at: time,
        }),
        _ => None,
    };
    let count = |n: Option<i64>| n.map(|n| u32::try_from(n.max(0)).unwrap_or(u32::MAX));
    Ok(ContactSettings {
        arrival,
        digest,
        stop_markers: u8::try_from(row.col::<i64>(at + 5)?).unwrap_or(0),
        notify: row.col::<i64>(at + 6)? != 0,
        html_treatment: row.col(at + 7)?,
        remind_days: count(row.col(at + 8)?),
        archive_after_days: count(row.col(at + 9)?),
        // Not every read of the row asks for it.
        ignored: row.col::<i64>(at + 10).is_ok_and(|n| n != 0),
    })
}
