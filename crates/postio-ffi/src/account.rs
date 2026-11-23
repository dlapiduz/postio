//! The accounts a settings pane lists, already worded so both panes describe the same software.

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountFfi {
    pub id: i64,
    pub address: String,
    pub display_name: String,
    pub initials: String,
    /// Words, never colour alone (ADR 0005): the row draws a "default" tag.
    pub is_default: bool,
    /// A flag for the toggle; `facts` is prose and may change.
    pub enabled: bool,
    /// The `·`-joined line under the address, assembled here so both panes describe an account
    /// alike.
    pub facts: Vec<String>,
    /// A flag, not a word to search `facts` for: the Reconnect gate once scanned prose that never
    /// contained it (#1584).
    pub needs_attention: bool,
    /// Answered for every account: a credential can be missing or rejected without any expiry.
    pub repair: RepairRouteFfi,
}

/// The routes are not interchangeable: an expired grant is re-consented in a browser, a rotated
/// app password is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RepairRouteFfi {
    Nothing,
    Password,
    /// Only an account Postio signed in itself; a broker-minted token has no client, so it is
    /// `Nothing`.
    Browser,
}

impl From<postio_ui::account::Repair> for RepairRouteFfi {
    fn from(repair: postio_ui::account::Repair) -> Self {
        match repair {
            postio_ui::account::Repair::Nothing => Self::Nothing,
            postio_ui::account::Repair::Password => Self::Password,
            postio_ui::account::Repair::Browser => Self::Browser,
        }
    }
}

impl AccountFfi {
    /// `token` is passed in because the expiry lives in the keyring, behind an async read.
    pub(crate) fn of(
        account: &postio_model::account::Account,
        token: postio_ui::account::TokenStanding,
    ) -> Self {
        let mut facts = vec![postio_ui::account::badge(account)];
        facts.extend(token.line());
        if !account.enabled {
            facts.push("disabled".to_owned());
        }
        AccountFfi {
            id: account.id.get(),
            address: account.address.address.clone(),
            display_name: account.display_name.clone(),
            initials: postio_ui::row::initials(Some(&account.address)),
            is_default: account.is_default,
            enabled: account.enabled,
            facts,
            needs_attention: token.is_expired(),
            repair: postio_ui::account::repair(account).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConnectionReportFfi {
    pub reachable: bool,
    /// No credential at all, rather than a rejected one (the pane's Partial state).
    pub missing_credential: bool,
    /// The server refused the credential; like `missing_credential`, it calls for repair, not
    /// retry.
    pub rejected_credential: bool,
    /// In words somebody can act on, from `postio_session::checkup`.
    pub message: String,
}
