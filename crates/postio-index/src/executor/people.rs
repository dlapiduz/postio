//! Who the matched mail is from and to, for the People tab: the conversation search's walk folded
//! per address.
//! The person's own addresses are left out: a row for yourself is not someone to search for.

use chrono::DateTime;
use postio_search::suggest::Person;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use crate::error::Result;

pub const OWN_ADDRESSES: &str =
    "SELECT lower(address) FROM accounts UNION SELECT lower(address) FROM identities";

pub const PEOPLE_CAP: u32 = 1_000;

pub async fn people(
    connection: &Connection,
    request: &super::ConversationRequest<'_>,
) -> Result<Vec<Person>> {
    let found = super::conversations::correspondents(connection, request).await?;
    if found.is_empty() {
        return Ok(Vec::new());
    }
    let ids = format!(
        "[{}]",
        found
            .iter()
            .map(|person| person.address.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let named: Vec<(i64, String, Option<String>)> = sql::all_unbounded(
        connection,
        &format!(
            "SELECT a.id, a.address,
                    (SELECT r.name FROM recipients r
                      WHERE r.address_id = a.id AND r.name IS NOT NULL AND r.name <> ''
                      LIMIT 1)
               FROM json_each(?1) j JOIN addresses a ON a.id = j.value
              WHERE a.address_normalized NOT IN ({OWN_ADDRESSES})"
        ),
        [ids.as_str()],
        |row| Ok((row.int(0)?, row.text(1)?, row.opt_text(2)?)),
    )
    .await?;

    let mut people: Vec<(i64, Person)> = named
        .into_iter()
        .filter_map(|(id, address, name)| {
            let counted = found.iter().find(|person| person.address == id)?;
            Some((
                counted.last,
                Person {
                    name,
                    address,
                    received: counted.from,
                    sent: counted.to,
                    last: DateTime::from_timestamp_millis(counted.last),
                },
            ))
        })
        .collect();
    people.sort_by(|(a_last, a), (b_last, b)| {
        (b.received + b.sent)
            .cmp(&(a.received + a.sent))
            .then(b_last.cmp(a_last))
            .then(a.address.cmp(&b.address))
    });
    let offset = (request.offset as usize).min(people.len());
    let end = offset
        .saturating_add(request.limit.min(PEOPLE_CAP) as usize)
        .min(people.len());
    Ok(people
        .drain(offset..end)
        .map(|(_, person)| person)
        .collect())
}
