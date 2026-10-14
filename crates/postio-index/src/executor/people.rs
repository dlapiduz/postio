//! Who the matched mail is from and to, as the People tab lists them
//! (spec 010 step 10, US9, design §3.11).
//!
//! The same walk the conversation search makes, capped as it is
//! ([`super::conversations`]), folded into one row per address, then one
//! read for the addresses and a name their mail gave them, the person's
//! own addresses left out: you are in every message you sent, and a row
//! for yourself is not someone to search for.

use chrono::DateTime;
use postio_search::suggest::Person;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use crate::error::Result;

/// The person's own addresses, folded as `addresses.address_normalized`
/// is: every account's and every identity's. Who "you" are wherever a
/// list of people leaves you out (the People tab here, the facets' names).
pub const OWN_ADDRESSES: &str =
    "SELECT lower(address) FROM accounts UNION SELECT lower(address) FROM identities";

/// How many people one answer carries: the walk is capped as a
/// conversation search's is, and this caps what one answer holds.
pub const PEOPLE_CAP: u32 = 1_000;

/// The People tab: everyone the request's match is from or to but the
/// person themselves, each with how many matched messages are from them
/// (`received`) and to them (`sent`) and the newest of those, most
/// messages first, then the most recent, then by address.
///
/// Two statements whatever the match: the walk and the addresses.
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
    // A person's name is one their mail carried; the address row keeps
    // only the address. The person's own addresses are every account's and
    // identity's, folded as `address_normalized` is.
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
