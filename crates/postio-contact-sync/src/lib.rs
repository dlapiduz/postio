//! Contacts sync as a plan (specs/011-focus-contacts S12).
//!
//! Rust owns every decision here and the address book is only bytes moving:
//! [`plan`] is a pure function of what Postio has, what the other side has and
//! what was synced last. It matches records, decides create, update or leave,
//! and turns a field both sides changed into a question; it never says delete
//! and never overwrites. [`dry_run`] reads a snapshot and returns the plan;
//! that is all this crate does with an address book unless two keys are turned,
//! and [`write`] says which: the build's [`CONTACT_SYNC_WRITES`] and the
//! person's confirmation of this very plan.
//!
//! No I/O, no database, no toolkit: the boundary check holds it there, and
//! every test runs over a [`FakeAddressBook`] that counts what reaches it.

use std::collections::BTreeSet;

use postio_model::ContactKind;

/// The build-time key. Off until the maintainer has reviewed what a dry run
/// printed and flips it in a commit of its own; until then nothing in this
/// crate asks an address book to change.
pub const CONTACT_SYNC_WRITES: bool = false;

/// The fields that sync. Kind, settings, notes, stats and Postio's own groups
/// stay in Postio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    /// Given name.
    pub given: String,
    /// Family name.
    pub family: String,
    /// Every address.
    pub emails: BTreeSet<String>,
    /// Every phone number, as written.
    pub phones: Vec<String>,
    /// Organisation.
    pub organization: String,
    /// Job title.
    pub title: String,
}

impl Fields {
    fn full_name(&self) -> String {
        format!("{} {}", self.given.trim(), self.family.trim())
            .trim()
            .to_lowercase()
    }
}

/// A person Postio has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRecord {
    /// Postio's id.
    pub id: i64,
    /// How to say them in a report.
    pub label: String,
    /// What would sync.
    pub fields: Fields,
    /// Only saved contacts sync.
    pub saved: bool,
    /// Which kinds sync is decided by [`Scope`].
    pub kind: ContactKind,
}

/// A record the other side has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRecord {
    /// The other side's identifier for it.
    pub id: String,
    /// How to say it in a report.
    pub label: String,
    /// What would sync.
    pub fields: Fields,
}

/// A match kept: the stable link, and the fields as of the last sync, which is
/// what makes a three-way comparison possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Postio's id.
    pub local: i64,
    /// The other side's id.
    pub external: String,
    /// The synced fields as of the last sync.
    pub base: Fields,
}

/// What syncs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// People only.
    People,
    /// People and companies (the default).
    PeopleAndCompanies,
    /// Everything saved.
    Everything,
}

impl Scope {
    fn includes(self, kind: ContactKind) -> bool {
        match self {
            Self::People => kind == ContactKind::Person,
            Self::PeopleAndCompanies => matches!(kind, ContactKind::Person | ContactKind::Company),
            Self::Everything => true,
        }
    }
}

/// Whether a plan may ask the other side to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Both ways.
    TwoWay,
    /// Read only: what the other side has may come in; nothing goes out.
    ReadOnly,
}

/// Which side a change is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A change to Postio's record.
    Local,
    /// A change to the other side's record.
    External,
}

/// A pair that is the same person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Postio's id.
    pub local: i64,
    /// The other side's id.
    pub external: String,
}

/// A saved contact the other side lacks: what would be created there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    /// Postio's id.
    pub local: i64,
    /// For the report.
    pub label: String,
    /// What would be written.
    pub fields: Fields,
}

/// One field of one record that would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Postio's id.
    pub local: i64,
    /// The other side's id.
    pub external: String,
    /// Which side changes.
    pub side: Side,
    /// Which field.
    pub field: String,
    /// What it becomes.
    pub to: String,
}

/// A field both sides changed, to different values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Postio's id.
    pub local: i64,
    /// The other side's id.
    pub external: String,
    /// For the report.
    pub label: String,
    /// Which field.
    pub field: String,
    /// Postio's value.
    pub mine: String,
    /// The other side's.
    pub theirs: String,
}

/// A contact that could be more than one record, or the reverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    /// Postio's id.
    pub local: i64,
    /// For the report.
    pub label: String,
    /// The other side's records it could be.
    pub candidates: Vec<String>,
}

/// A linked record the other side no longer has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gone {
    /// Postio's id.
    pub local: i64,
    /// For the report.
    pub label: String,
    /// The id that vanished.
    pub external: String,
}

/// A delete. Nothing in this crate ever makes one; the plan carries the list
/// so a report can say there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete;

/// What a sync would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Whether the plan may change the other side.
    pub mode: Mode,
    /// Records found to be the same person.
    pub matches: Vec<Match>,
    /// Saved contacts the other side lacks.
    pub creates: Vec<Create>,
    /// Fields that would change, one side or the other.
    pub updates: Vec<Change>,
    /// Fields both sides changed.
    pub conflicts: Vec<Conflict>,
    /// Contacts that could be more than one record.
    pub ambiguous: Vec<Ambiguous>,
    /// Linked records the other side no longer has.
    pub gone: Vec<Gone>,
    /// Always empty: a delete is never automatic.
    pub deletes: Vec<Delete>,
    /// Records on the other side nothing in Postio matches, which stay there.
    pub left_alone: usize,
}

impl Plan {
    /// `(add, update, ask, delete)`.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.creates.len(),
            self.updates.len(),
            self.conflicts.len() + self.ambiguous.len() + self.gone.len(),
            self.deletes.len(),
        )
    }

    /// "Would add 12, update 3, ask about 2, delete 0": counts only, so it
    /// may be logged.
    pub fn summary(&self) -> String {
        let (add, update, ask, delete) = self.counts();
        format!("Would add {add}, update {update}, ask about {ask}, delete {delete}")
    }

    /// What the plan says in words, a line each, with names. It is the
    /// person's to read and is never logged.
    pub fn report(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for create in &self.creates {
            lines.push(format!("Add {} to Contacts", create.label));
        }
        for change in &self.updates {
            let who = match change.side {
                Side::Local => "in Postio",
                Side::External => "in Contacts",
            };
            lines.push(format!(
                "Update {} {who}: {} becomes {}",
                change.local, change.field, change.to
            ));
        }
        for conflict in &self.conflicts {
            lines.push(format!(
                "Ask about {}: {} is \u{201c}{}\u{201d} here and \u{201c}{}\u{201d} there",
                conflict.label, conflict.field, conflict.mine, conflict.theirs
            ));
        }
        for ambiguous in &self.ambiguous {
            lines.push(format!(
                "Ask about {}: could be {}",
                ambiguous.label,
                ambiguous.candidates.join(" or ")
            ));
        }
        for gone in &self.gone {
            lines.push(format!(
                "Ask about {}: no longer in Contacts. Postio keeps them either way",
                gone.label
            ));
        }
        if self.left_alone > 0 {
            lines.push(format!(
                "{} in Contacts match no one in Postio and stay there",
                self.left_alone
            ));
        }
        lines
    }

    fn fingerprint(&self) -> u64 {
        // FNV-1a over the plan's own text: stable, and different whenever
        // what would be written is.
        let text = format!("{self:?}");
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// The last four digits' worth of a number, for matching: digits only,
/// compared on the trailing nine.
fn phone_key(phone: &str) -> Option<String> {
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 7).then(|| digits[digits.len().saturating_sub(9)..].to_owned())
}

fn shares_email(a: &Fields, b: &Fields) -> bool {
    a.emails
        .iter()
        .any(|x| b.emails.iter().any(|y| x.eq_ignore_ascii_case(y)))
}

fn shares_name_and_phone(a: &Fields, b: &Fields) -> bool {
    let name = a.full_name();
    if name.is_empty() || name != b.full_name() {
        return false;
    }
    a.phones
        .iter()
        .filter_map(|p| phone_key(p))
        .any(|x| b.phones.iter().filter_map(|p| phone_key(p)).any(|y| x == y))
}

/// The plan for syncing `locals` with `externals`, given what is already
/// `links`.
pub fn plan(
    locals: &[LocalRecord],
    externals: &[ExternalRecord],
    links: &[Link],
    scope: Scope,
    mode: Mode,
) -> Plan {
    let mut plan = Plan {
        mode,
        matches: Vec::new(),
        creates: Vec::new(),
        updates: Vec::new(),
        conflicts: Vec::new(),
        ambiguous: Vec::new(),
        gone: Vec::new(),
        deletes: Vec::new(),
        left_alone: 0,
    };
    let in_scope = |l: &&LocalRecord| l.saved && scope.includes(l.kind);
    let mut used_external: BTreeSet<&str> = BTreeSet::new();
    let mut settled_local: BTreeSet<i64> = BTreeSet::new();

    // Links already made: compare field by field, or ask about a record that
    // went away. Deleting anything is never the answer.
    for link in links {
        let Some(local) = locals.iter().filter(in_scope).find(|l| l.id == link.local) else {
            continue;
        };
        settled_local.insert(local.id);
        match externals.iter().find(|e| e.id == link.external) {
            Some(external) => {
                used_external.insert(external.id.as_str());
                compare(&mut plan, local, external, &link.base);
            }
            None => plan.gone.push(Gone {
                local: local.id,
                label: local.label.clone(),
                external: link.external.clone(),
            }),
        }
    }

    // New matches, by any shared address, then by name and phone. One
    // candidate on both sides is a match; more than one on either is a
    // question.
    let free_locals: Vec<&LocalRecord> = locals
        .iter()
        .filter(in_scope)
        .filter(|l| !settled_local.contains(&l.id))
        .collect();
    let free_externals: Vec<&ExternalRecord> = externals
        .iter()
        .filter(|e| !used_external.contains(e.id.as_str()))
        .collect();
    let mut ambiguous_externals: BTreeSet<&str> = BTreeSet::new();
    for local in &free_locals {
        let mut candidates: Vec<&&ExternalRecord> = free_externals
            .iter()
            .filter(|e| shares_email(&local.fields, &e.fields))
            .collect();
        if candidates.is_empty() {
            candidates = free_externals
                .iter()
                .filter(|e| shares_name_and_phone(&local.fields, &e.fields))
                .collect();
        }
        match candidates.as_slice() {
            [] => {}
            [only] => {
                let rivals = free_locals
                    .iter()
                    .filter(|l| {
                        shares_email(&l.fields, &only.fields)
                            || shares_name_and_phone(&l.fields, &only.fields)
                    })
                    .count();
                if rivals == 1 {
                    plan.matches.push(Match {
                        local: local.id,
                        external: only.id.clone(),
                    });
                    settled_local.insert(local.id);
                    used_external.insert(only.id.as_str());
                    compare(&mut plan, local, only, &Fields::default());
                } else {
                    settled_local.insert(local.id);
                    ambiguous_externals.insert(only.id.as_str());
                    plan.ambiguous.push(Ambiguous {
                        local: local.id,
                        label: local.label.clone(),
                        candidates: vec![only.label.clone()],
                    });
                }
            }
            many => {
                settled_local.insert(local.id);
                for e in many {
                    ambiguous_externals.insert(e.id.as_str());
                }
                plan.ambiguous.push(Ambiguous {
                    local: local.id,
                    label: local.label.clone(),
                    candidates: many.iter().map(|e| e.label.clone()).collect(),
                });
            }
        }
    }

    // A saved contact nobody matched is a create on the other side, in two
    // way mode only.
    for local in &free_locals {
        if !settled_local.contains(&local.id) && mode == Mode::TwoWay {
            plan.creates.push(Create {
                local: local.id,
                label: local.label.clone(),
                fields: local.fields.clone(),
            });
        }
    }
    plan.left_alone = externals
        .iter()
        .filter(|e| {
            !used_external.contains(e.id.as_str()) && !ambiguous_externals.contains(e.id.as_str())
        })
        .count();
    plan
}

/// One pair, field by field against what was synced last.
fn compare(plan: &mut Plan, local: &LocalRecord, external: &ExternalRecord, base: &Fields) {
    let strings: [(&str, &str, &str, &str); 4] = [
        (
            "given name",
            &local.fields.given,
            &external.fields.given,
            &base.given,
        ),
        (
            "family name",
            &local.fields.family,
            &external.fields.family,
            &base.family,
        ),
        (
            "organization",
            &local.fields.organization,
            &external.fields.organization,
            &base.organization,
        ),
        (
            "title",
            &local.fields.title,
            &external.fields.title,
            &base.title,
        ),
    ];
    for (field, mine, theirs, was) in strings {
        if mine == theirs {
            continue;
        }
        if mine == was {
            plan.updates
                .push(change(local, external, Side::Local, field, theirs));
        } else if theirs == was {
            if plan.mode == Mode::TwoWay {
                plan.updates
                    .push(change(local, external, Side::External, field, mine));
            }
        } else {
            plan.conflicts.push(Conflict {
                local: local.id,
                external: external.id.clone(),
                label: local.label.clone(),
                field: field.to_owned(),
                mine: mine.to_owned(),
                theirs: theirs.to_owned(),
            });
        }
    }
    // Addresses and phones merge as sets: what one side has and the other
    // lacks goes across, and nothing is ever taken away.
    let lacking_here: Vec<&String> = external
        .fields
        .emails
        .iter()
        .filter(|e| {
            !local
                .fields
                .emails
                .iter()
                .any(|m| m.eq_ignore_ascii_case(e))
        })
        .collect();
    if !lacking_here.is_empty() {
        let joined = lacking_here
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        plan.updates
            .push(change(local, external, Side::Local, "addresses", &joined));
    }
    let lacking_there: Vec<&String> = local
        .fields
        .emails
        .iter()
        .filter(|m| {
            !external
                .fields
                .emails
                .iter()
                .any(|e| e.eq_ignore_ascii_case(m))
        })
        .collect();
    if !lacking_there.is_empty() && plan.mode == Mode::TwoWay {
        let joined = lacking_there
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        plan.updates.push(change(
            local,
            external,
            Side::External,
            "addresses",
            &joined,
        ));
    }
    let keys =
        |f: &Fields| -> BTreeSet<String> { f.phones.iter().filter_map(|p| phone_key(p)).collect() };
    let (mine, theirs) = (keys(&local.fields), keys(&external.fields));
    let new_here: Vec<&String> = external
        .fields
        .phones
        .iter()
        .filter(|p| phone_key(p).is_some_and(|k| !mine.contains(&k)))
        .collect();
    if !new_here.is_empty() {
        let joined = new_here
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        plan.updates
            .push(change(local, external, Side::Local, "phones", &joined));
    }
    let new_there: Vec<&String> = local
        .fields
        .phones
        .iter()
        .filter(|p| phone_key(p).is_some_and(|k| !theirs.contains(&k)))
        .collect();
    if !new_there.is_empty() && plan.mode == Mode::TwoWay {
        let joined = new_there
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        plan.updates
            .push(change(local, external, Side::External, "phones", &joined));
    }
}

fn change(
    local: &LocalRecord,
    external: &ExternalRecord,
    side: Side,
    field: &str,
    to: &str,
) -> Change {
    Change {
        local: local.id,
        external: external.id.clone(),
        side,
        field: field.to_owned(),
        to: to.to_owned(),
    }
}

/// Whether the person has let Postio into an address book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    /// Never asked.
    NotDetermined,
    /// The person said no (or the system did). Final until they turn sync on
    /// again.
    Denied,
    /// Allowed.
    Authorized,
}

/// Why sync did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// No permission, so nothing was read.
    NotAuthorized(Authorization),
    /// The build's key is off.
    WritesOff,
    /// The person has not confirmed this plan.
    NotConfirmed,
    /// The plan is for a read-only account.
    ReadOnly,
    /// The address book said no.
    Book(String),
}

/// The address book: a trait, so every test, the demo and every storyboard
/// use a fake, and the real store (an adapter over `CNContactStore`) is one
/// implementation nothing here constructs.
pub trait ExternalAddressBook {
    /// Whether Postio may read it. Never prompts.
    fn authorization(&self) -> Authorization;
    /// Asks the system for access. Called only by [`enable`].
    fn request_access(&mut self) -> Authorization;
    /// Every record, for the sync scope.
    fn snapshot(&mut self) -> Result<Vec<ExternalRecord>, SyncError>;
    /// Writes `plan`. Called only by [`write`], behind two keys.
    fn apply(&mut self, plan: &Plan) -> Result<(), SyncError>;
}

/// The person turning sync on: the one place access is asked for, and only
/// when the system has not already decided.
pub fn enable(book: &mut dyn ExternalAddressBook) -> Authorization {
    match book.authorization() {
        Authorization::NotDetermined => book.request_access(),
        decided => decided,
    }
}

/// What a sync would do, read from a snapshot. Never writes, never prompts.
pub fn dry_run(
    book: &mut dyn ExternalAddressBook,
    locals: &[LocalRecord],
    links: &[Link],
    scope: Scope,
    mode: Mode,
) -> Result<Plan, SyncError> {
    match book.authorization() {
        Authorization::Authorized => {}
        other => return Err(SyncError::NotAuthorized(other)),
    }
    let externals = book.snapshot()?;
    Ok(plan(locals, &externals, links, scope, mode))
}

/// The person's yes to one plan: bound to what it would write, so a plan that
/// changed since is not confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirmation(u64);

impl Confirmation {
    /// The confirmation of exactly `plan`.
    pub fn of(plan: &Plan) -> Self {
        Self(plan.fingerprint())
    }
}

/// Writes `plan` to the address book, if both keys are turned: the build's
/// (`writes_enabled`, which callers pass [`CONTACT_SYNC_WRITES`]) and the
/// person's confirmation of this plan. Either off, nothing reaches the book.
pub fn write(
    book: &mut dyn ExternalAddressBook,
    plan: &Plan,
    confirmation: Option<Confirmation>,
    writes_enabled: bool,
) -> Result<(), SyncError> {
    if !writes_enabled {
        return Err(SyncError::WritesOff);
    }
    if confirmation != Some(Confirmation::of(plan)) {
        return Err(SyncError::NotConfirmed);
    }
    if plan.mode == Mode::ReadOnly {
        return Err(SyncError::ReadOnly);
    }
    book.apply(plan)
}

/// An address book in memory that counts what reaches it.
#[derive(Debug, Clone)]
pub struct FakeAddressBook {
    state: Authorization,
    grant: Authorization,
    records: Vec<ExternalRecord>,
    prompts: usize,
    snapshots: usize,
    applied: usize,
}

impl FakeAddressBook {
    /// A book in `state` holding `records`.
    pub fn new(state: Authorization, records: Vec<ExternalRecord>) -> Self {
        Self {
            state,
            grant: Authorization::Denied,
            records,
            prompts: 0,
            snapshots: 0,
            applied: 0,
        }
    }

    /// What asking for access will answer.
    #[must_use]
    pub fn granting(mut self, answer: Authorization) -> Self {
        self.grant = answer;
        self
    }

    /// How many times access was asked for.
    pub fn prompts(&self) -> usize {
        self.prompts
    }

    /// How many snapshots were read.
    pub fn snapshots(&self) -> usize {
        self.snapshots
    }

    /// How many times a plan was applied.
    pub fn applied(&self) -> usize {
        self.applied
    }
}

impl ExternalAddressBook for FakeAddressBook {
    fn authorization(&self) -> Authorization {
        self.state
    }

    fn request_access(&mut self) -> Authorization {
        self.prompts += 1;
        self.state = self.grant;
        self.state
    }

    fn snapshot(&mut self) -> Result<Vec<ExternalRecord>, SyncError> {
        self.snapshots += 1;
        Ok(self.records.clone())
    }

    fn apply(&mut self, _plan: &Plan) -> Result<(), SyncError> {
        self.applied += 1;
        Ok(())
    }
}
