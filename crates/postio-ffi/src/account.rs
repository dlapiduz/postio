//! The accounts a settings pane lists.
//!
//! Rows, not records: everything here is what the pane draws, already worded,
//! because the two settings panes have to describe the same software. The
//! facts are assembled on this side from `postio_ui::account` so a frontend
//! joins them and does not decide them.

/// One account, as the accounts pane draws it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountFfi {
    /// The account this row is about.
    pub id: i64,
    /// The address, which is the row's title.
    pub address: String,
    /// What the person called it, if they called it anything.
    pub display_name: String,
    /// The two letters the row's chip shows — `postio_ui::row::initials`, so
    /// a sender and an account abbreviate the same way.
    pub initials: String,
    /// Whether new messages come from this one.
    ///
    /// Words, never colour alone (ADR 0005): the row draws a "default" tag,
    /// and it says what the marker *does* rather than asserting a status —
    /// #960's fence is that this account is not more the user's than another.
    pub is_default: bool,
    /// The `·`-joined line under the address, in the order a person reads it:
    /// what kind of account, how it signs in, and how it stands right now.
    ///
    /// Assembled here rather than in a frontend. Two panes joining their own
    /// facts is two descriptions of one account, and the one that drifts is
    /// whichever nobody is looking at.
    pub facts: Vec<String>,
}

impl AccountFfi {
    /// How `account` reads in the pane.
    pub(crate) fn of(account: &postio_model::account::Account) -> Self {
        let mut facts = vec![postio_ui::account::badge(account)];
        // A disabled account still has a row: it is configured, it is simply
        // not being synced, and a list that hid it would make "where did my
        // account go" the next question.
        if !account.enabled {
            facts.push("disabled".to_owned());
        }
        AccountFfi {
            id: account.id.get(),
            address: account.address.address.clone(),
            display_name: account.display_name.clone(),
            initials: postio_ui::row::initials(Some(&account.address)),
            is_default: account.is_default,
            facts,
        }
    }
}

/// What a connection test found.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConnectionReportFfi {
    /// Whether the account could sign in.
    pub reachable: bool,
    /// Whether the failure was that there is **no** credential for this
    /// account rather than a rejected one — the pane's *Partial* state,
    /// which calls for a different offer.
    pub missing_credential: bool,
    /// What happened, in words somebody can act on. The wording is
    /// `postio_session::checkup`'s, so both frontends explain a rejected
    /// password the same way — including the part the error cannot know,
    /// which is that a provider refusing an ordinary account password says
    /// only "rejected".
    pub message: String,
}
