//! Rust owns every decision; the address book is only bytes moving. `plan` is a pure function of
//! what
//! Postio has, what the other side has and what was synced last; it never says delete and never
//! overwrites.
//! Nothing writes unless two keys are turned: `CONTACT_SYNC_WRITES` and the person's confirmation
//! of this very plan.

use std::collections::BTreeSet;

use postio_model::ContactKind;

/// Off until the maintainer has reviewed a dry run and flips it in a commit of its own.
pub const CONTACT_SYNC_WRITES: bool = false;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub given: String,
    pub family: String,
    pub emails: BTreeSet<String>,
    pub phones: Vec<String>,
    pub organization: String,
    pub title: String,
}

impl Fields {
    fn full_name(&self) -> String {
        format!("{} {}", self.given.trim(), self.family.trim())
            .trim()
            .to_lowercase()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRecord {
    pub id: i64,
    pub label: String,
    pub fields: Fields,
    pub saved: bool,
    pub kind: ContactKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRecord {
    pub id: String,
    pub label: String,
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub local: i64,
    pub external: String,
    pub base: Fields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    People,
    PeopleAndCompanies,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    TwoWay,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub local: i64,
    pub external: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub external: String,
    pub label: String,
    pub kind: ContactKind,
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    pub local: i64,
    pub label: String,
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub local: i64,
    pub external: String,
    pub label: String,
    pub side: Side,
    pub field: String,
    pub to: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub local: i64,
    pub external: String,
    pub label: String,
    pub field: String,
    pub mine: String,
    pub theirs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    pub local: i64,
    pub label: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gone {
    pub local: i64,
    pub label: String,
    pub external: String,
}

/// Nothing in this crate ever makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delete;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BroughtIn {
    pub people: usize,
    pub fields: usize,
    /// Records Postio could not keep (an address someone else has, a refusal); they stay where they
    /// were.
    pub skipped: usize,
    pub made: Vec<i64>,
    pub touched: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub mode: Mode,
    pub matches: Vec<Match>,
    pub creates: Vec<Create>,
    /// Both modes plan them: they change Postio and nothing else.
    pub imports: Vec<Import>,
    pub updates: Vec<Change>,
    pub conflicts: Vec<Conflict>,
    pub ambiguous: Vec<Ambiguous>,
    pub gone: Vec<Gone>,
    /// Always empty: a delete is never automatic.
    pub deletes: Vec<Delete>,
    /// Records on the other side that stay there: out of scope, without an address or name, or a
    /// second record for an address another already brings in.
    pub left_alone: usize,
}

impl Plan {
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.creates.len(),
            self.updates.len(),
            self.conflicts.len() + self.ambiguous.len() + self.gone.len(),
            self.deletes.len(),
        )
    }

    /// Changes to the other side are not in it.
    pub fn bring_in_counts(&self) -> (usize, usize) {
        let local = self
            .updates
            .iter()
            .filter(|u| u.side == Side::Local)
            .count();
        (self.imports.len(), local)
    }

    /// Counts only, so it may be logged. "Bring in" is into Postio; "add" is into the other side.
    pub fn summary(&self) -> String {
        let (add, update, ask, delete) = self.counts();
        let bring = self.imports.len();
        format!(
            "Would bring in {bring}, add {add}, update {update}, ask about {ask}, delete {delete}"
        )
    }

    /// Names are the person's to read; never logged.
    pub fn report(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for import in &self.imports {
            lines.push(format!("Bring {} into Postio", import.label));
        }
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
                change.label, change.field, change.to
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
                "{} in Contacts stay there: out of scope, or without a name and an address Postio could keep them by",
                self.left_alone
            ));
        }
        lines
    }

    /// The person's yes is bound to it, so a plan that changed since is not the one confirmed.
    pub fn fingerprint(&self) -> u64 {
        let text = format!("{self:?}");
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

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
        imports: Vec::new(),
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

    // By any shared address, then by name and phone. One candidate on both sides is a match; more
    // than one on either is a question.
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

    for local in &free_locals {
        if !settled_local.contains(&local.id) && mode == Mode::TwoWay {
            plan.creates.push(Create {
                local: local.id,
                label: local.label.clone(),
                fields: local.fields.clone(),
            });
        }
    }
    // A record only they have comes in if Postio can keep it: an address to be found by, a name or
    // organisation to be called by, and the scope must take its kind.
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for external in externals {
        if used_external.contains(external.id.as_str())
            || ambiguous_externals.contains(external.id.as_str())
        {
            continue;
        }
        let fields = &external.fields;
        let named = !fields.given.trim().is_empty() || !fields.family.trim().is_empty();
        let kind = if named || fields.organization.trim().is_empty() {
            ContactKind::Person
        } else {
            ContactKind::Company
        };
        let usable = !fields.emails.is_empty()
            && (named || !fields.organization.trim().is_empty())
            && scope.includes(kind);
        let addresses: Vec<String> = fields.emails.iter().map(|e| e.to_lowercase()).collect();
        if !usable || addresses.iter().any(|a| claimed.contains(a)) {
            plan.left_alone += 1;
            continue;
        }
        claimed.extend(addresses);
        plan.imports.push(Import {
            external: external.id.clone(),
            label: external.label.clone(),
            kind,
            fields: fields.clone(),
        });
    }
    plan
}

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
    // Addresses and phones merge as sets: nothing is ever taken away.
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
        plan.updates.push(change_set(
            local,
            external,
            Side::Local,
            "addresses",
            &lacking_here,
        ));
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
        plan.updates.push(change_set(
            local,
            external,
            Side::External,
            "addresses",
            &lacking_there,
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
        plan.updates.push(change_set(
            local,
            external,
            Side::Local,
            "phones",
            &new_here,
        ));
    }
    let new_there: Vec<&String> = local
        .fields
        .phones
        .iter()
        .filter(|p| phone_key(p).is_some_and(|k| !theirs.contains(&k)))
        .collect();
    if !new_there.is_empty() && plan.mode == Mode::TwoWay {
        plan.updates.push(change_set(
            local,
            external,
            Side::External,
            "phones",
            &new_there,
        ));
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
        label: local.label.clone(),
        side,
        field: field.to_owned(),
        to: to.to_owned(),
        values: vec![to.to_owned()],
    }
}

fn change_set(
    local: &LocalRecord,
    external: &ExternalRecord,
    side: Side,
    field: &str,
    values: &[&String],
) -> Change {
    let joined = values
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Change {
        values: values.iter().map(|s| (*s).clone()).collect(),
        ..change(local, external, side, field, &joined)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authorization {
    NotDetermined,
    Denied,
    Authorized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    NotAuthorized(Authorization),
    WritesOff,
    NotConfirmed,
    ReadOnly,
    Book(String),
}

/// A trait so every test, the demo and every storyboard use a fake; the real store (an adapter over
/// `CNContactStore`) is one implementation nothing here constructs.
pub trait ExternalAddressBook {
    /// Never prompts.
    fn authorization(&self) -> Authorization;
    /// Called only by `enable`.
    fn request_access(&mut self) -> Authorization;
    fn snapshot(&mut self) -> Result<Vec<ExternalRecord>, SyncError>;
    /// Called only by `write`, behind two keys.
    fn apply(&mut self, plan: &Plan) -> Result<(), SyncError>;
}

/// The one place access is asked for, and only when the system has not already decided.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirmation(u64);

impl Confirmation {
    pub fn of(plan: &Plan) -> Self {
        Self(plan.fingerprint())
    }
}

/// Either key off, nothing reaches the book.
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

    #[must_use]
    pub fn granting(mut self, answer: Authorization) -> Self {
        self.grant = answer;
        self
    }

    pub fn prompts(&self) -> usize {
        self.prompts
    }

    pub fn snapshots(&self) -> usize {
        self.snapshots
    }

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
