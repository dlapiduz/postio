use postio_model::calendar::{Calendar, NewCalendar};
use postio_model::ids::{AccountId, CalendarId};
use turso::Row;

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug)]
pub struct CalendarRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str =
    "id, account_id, href, name, colour, writable, shown, schedules, default_alarm_minutes";

impl<'a> CalendarRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn local(&self) -> Result<Calendar> {
        sql::one(
            self.connection,
            &format!(
                "SELECT {COLUMNS} FROM calendars WHERE account_id IS NULL ORDER BY id LIMIT 1"
            ),
            (),
            read_calendar,
        )
        .await
    }

    pub async fn list(&self) -> Result<Vec<Calendar>> {
        sql::all(
            self.connection,
            &format!("SELECT {COLUMNS} FROM calendars ORDER BY account_id IS NOT NULL, id"),
            (),
            read_calendar,
        )
        .await
    }

    pub async fn create(&self, calendar: &NewCalendar) -> Result<CalendarId> {
        sql::execute(
            self.connection,
            "INSERT INTO calendars (account_id, href, name, colour, writable, schedules)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            bind![
                calendar.account.map(AccountId::get),
                calendar.href.clone(),
                calendar.name.clone(),
                calendar.colour.clone(),
                calendar.writable,
                calendar.schedules,
            ],
        )
        .await?;
        Ok(CalendarId::new(self.connection.last_insert_rowid()))
    }
}

fn read_calendar(row: &Row) -> Result<Calendar> {
    let account: Option<i64> = row.col(1)?;
    let minutes: Option<i64> = row.col(8)?;
    Ok(Calendar {
        id: CalendarId::new(row.col(0)?),
        account: account.map(AccountId::new),
        href: row.col(2)?,
        name: row.col(3)?,
        colour: row.col(4)?,
        writable: row.col(5)?,
        shown: row.col(6)?,
        schedules: row.col(7)?,
        default_alarm_minutes: minutes.and_then(|minutes| u32::try_from(minutes).ok()),
    })
}
