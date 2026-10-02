//! The search box's prefix modes for the macOS frontend: `#` folders, `@`
//! correspondents, `+` labels (`postio_ui::finder::MODES`; `>` is the
//! palette's, `paletteEntries`).
//!
//! What crosses is a mode's matches, already scored and ordered by the shared
//! matcher, and the sentence for when there are none -- the wording GTK's box
//! uses, so the two boxes answer an empty result the same way.

/// One row a prefix mode offers.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FinderHitFfi {
    /// The folder or label to act on. Zero for a correspondent, whose row is
    /// acted on through [`query`](Self::query).
    pub id: i64,
    /// What the row says: the folder's name, the label's, the person's.
    pub title: String,
    /// The second thing the row says: a folder's unread count, a
    /// correspondent's address.
    pub detail: Option<String>,
    /// Byte offsets in `title` the query matched, for emphasis.
    pub positions: Vec<u32>,
    /// What picking a correspondent writes into the box: a `from:` query,
    /// so the search that follows is one the user can go on building.
    pub query: Option<String>,
}

/// A mode's answer: its rows, and what to say when there are none.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FinderAnswerFfi {
    /// The matches, best first.
    pub hits: Vec<FinderHitFfi>,
    /// Never a shrug: names what was looked in.
    pub empty: String,
}

pub(crate) fn positions(positions: &[usize]) -> Vec<u32> {
    positions.iter().map(|&at| at as u32).collect()
}

/// `#`: the folders matching `query`, best first.
///
/// The palette's matcher over the names the sidebar draws, so `wd` finds
/// `wayland-devel` here exactly as `cp` finds "Command palette" in `>`.
/// GTK's `postio_gtk::finder::folders` is the same rule over its own rows.
/// Views (Flagged, Snoozed) are not folders to go to, and a container that
/// holds no mail is not either.
pub(crate) fn folders(mailboxes: &[crate::MailboxFfi], query: &str) -> FinderAnswerFfi {
    let query = query.trim();
    let mut found: Vec<(i32, FinderHitFfi)> = mailboxes
        .iter()
        .filter(|mailbox| mailbox.id != 0 && mailbox.selectable)
        .filter_map(|mailbox| {
            let matched = postio_ui::palette::score(query, &mailbox.name)?;
            Some((
                matched.score,
                FinderHitFfi {
                    id: mailbox.id,
                    title: mailbox.name.clone(),
                    detail: (mailbox.unread > 0).then(|| mailbox.unread.to_string()),
                    positions: positions(&matched.positions),
                    query: None,
                },
            ))
        })
        .collect();
    // Stable, so an empty query leaves the sidebar's own order alone.
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found.truncate(postio_ui::palette::MAX_ROWS);
    FinderAnswerFfi {
        hits: found.into_iter().map(|(_, hit)| hit).collect(),
        empty: format!("No folder matches “{query}”"),
    }
}

/// `@`: the correspondents matching `query`, best first.
pub(crate) fn contacts(known: &[postio_model::Contact], query: &str) -> FinderAnswerFfi {
    let hits = postio_ui::finder::contacts(known, query)
        .into_iter()
        .map(|hit| FinderHitFfi {
            id: 0,
            query: Some(postio_ui::finder::contact_query(&hit)),
            positions: positions(&hit.positions),
            detail: Some(hit.address.clone()),
            title: hit.name,
        })
        .collect();
    // Two different empties. Postio has no address book: correspondents are
    // learned from the mail that syncs, so none at all is a mailbox that has
    // not synced rather than a query that missed.
    let empty = if known.is_empty() {
        "No correspondents yet — Postio learns them from the mail it syncs.".to_owned()
    } else {
        format!("No correspondent matches “{}”", query.trim())
    };
    FinderAnswerFfi { hits, empty }
}

/// `+`: the labels matching `query`, best first.
pub(crate) fn labels(known: &[postio_model::Label], query: &str) -> FinderAnswerFfi {
    FinderAnswerFfi {
        hits: postio_ui::finder::labels(known, query)
            .into_iter()
            .map(|hit| FinderHitFfi {
                id: hit.id.get(),
                title: hit.name,
                detail: None,
                positions: positions(&hit.positions),
                query: None,
            })
            .collect(),
        empty: format!("No label matches “{}”", query.trim()),
    }
}
