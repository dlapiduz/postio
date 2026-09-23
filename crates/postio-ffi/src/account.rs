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
    /// Whether the account participates in sync.
    ///
    /// Separate from the `disabled` fact below, which is the same state
    /// worded for a person to read. A switch needs the flag: a pane deriving
    /// its toggle by searching the fact line for a word would be reading
    /// prose as an API, and the prose is allowed to change.
    pub enabled: bool,
    /// The `·`-joined line under the address, in the order a person reads it:
    /// what kind of account, how it signs in, and how it stands right now.
    ///
    /// Assembled here rather than in a frontend. Two panes joining their own
    /// facts is two descriptions of one account, and the one that drifts is
    /// whichever nobody is looking at.
    pub facts: Vec<String>,
    /// Whether the row draws a warning mark, because something about this
    /// account is a problem rather than a fact.
    ///
    /// Today that is exactly one state: an OAuth grant past its expiry,
    /// which stops mail arriving and which only the user can put right.
    ///
    /// **A flag, not a word to search the facts for.** The frontend used to
    /// derive this by scanning `facts` for "expired" or "reconnect" — over a
    /// boundary that produced neither, so the gate could not fire and the
    /// Reconnect button behind it was unreachable code (#1584). A row that
    /// re-derives a state the engine already knows is a row that can be
    /// wrong about it.
    pub needs_attention: bool,
    /// What putting this account's credential right would take, so the pane
    /// offers the one route that would work.
    ///
    /// Answered for every account, not only a flagged one: a credential can
    /// be missing or rejected without any expiry saying so — the pane's
    /// *Partial* state — and the offer is the same either way.
    pub repair: RepairRouteFfi,
}

/// How an account's credential is put right, when it needs to be.
///
/// `postio_ui::account::Repair`, crossing the boundary. The routes are not
/// interchangeable: an expired grant is re-consented in a browser and cannot
/// be typed, and a rotated app password is typed and cannot be re-consented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RepairRouteFfi {
    /// Nothing to repair — a local mail store signs in to nothing.
    Nothing,
    /// A new password for the keyring: `Session::repairCredential`.
    Password,
    /// Sign in again through the system browser:
    /// `Session::reconnectAccount`, which resolves the client this account
    /// registered and then runs the ordinary sign-in over it. Only an
    /// account Postio itself signed in lands here — one whose token a
    /// broker mints is [`Nothing`](Self::Nothing), because there is no
    /// client to reconnect with.
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
    /// How `account` reads in the pane, given where its token stands.
    ///
    /// `token` is handed in because this side cannot read it: the expiry
    /// lives in the keyring, the read is asynchronous, and this is not — the
    /// same split `postio-gtk`'s panel makes with `set_token_expiries`, for
    /// the same reason. [`crate::Session::accounts`] is what does the
    /// reading.
    pub(crate) fn of(
        account: &postio_model::account::Account,
        token: postio_ui::account::TokenStanding,
    ) -> Self {
        let mut facts = vec![postio_ui::account::badge(account)];
        // Between the badge and "disabled", which is the order GTK's row
        // reads in: what kind of account, how it stands, and only then
        // whether it is switched off.
        facts.extend(token.line());
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
            enabled: account.enabled,
            facts,
            needs_attention: token.is_expired(),
            repair: postio_ui::account::repair(account).into(),
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
