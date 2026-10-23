# Contract: contact sync (the last step)

The boundary between the Rust engine and an external address book, and the rules
that keep Postio from writing to one before the maintainer has reviewed it.
Shapes, not code; written test-first. Spec: "Sync". Plan: section 8.

## The foreign trait (UniFFI, implemented in Swift for Apple Contacts)

```text
trait ExternalAddressBook {
  fn authorization() -> Authorization          // notDetermined | denied | restricted | authorized
  fn request_access() -> bool                  // called ONLY when the person enables the account
  fn containers() -> [Container { id, name, kind }]
  fn snapshot(container_id, since: Option<Token>) -> Snapshot { records: [ExternalRecord], deleted: [ExternalId], token }
  fn apply(plan: ApprovedPlan) -> ApplyResult  // refuses unless the gate is open (below)
}
ExternalRecord { id, etag_or_hash, given, family, emails[], phones[{label,value}], organization, title, photo: Option<bytes>, groups[] }
```

- `snapshot` returns only the synced fields (spec, Field ownership); notes are
  never read. Photos cross as bytes and are written to the local blob store.
- CardDAV implements the same shape in Rust over its transport, with `Token` as the
  `sync-token` or ctag.
- The runner (Rust) is the only caller. Swift never decides what to write.

## The plan (pure, `postio-contact-sync`)

```text
Plan { imports[ExternalId → Contact],    // Contacts-only people who would come INTO Postio
       creates[Contact → Account], updates[Link, side, fields], deletes[Link],
       conflicts[Conflict { contact, field, mine, theirs }], ambiguous[Contact, [ExternalId]],
       adoptions[Link] }   // matches that create a link and write nothing
```

Inputs: the Postio contacts in scope (saved, of the account's scope kinds), the
external snapshot, the stored links with their `base`. Outputs are deterministic.

**Rules** (each a red test, both directions): match by any shared normalised email;
else name plus phone (digits, trailing nine); more than one candidate is
`ambiguous`, never linked. Per synced field: only one side changed since `base` →
take it; both changed to different values → `conflict`; both to the same value →
no change. Address lists merge as sets. A record deleted externally → `unsave` the
contact and mark the link gone. A Postio delete of a linked contact → a `delete`
**only after the person answered the question**. Kinds outside the scope never
appear in a plan.

## Read only brings Contacts into Postio

Read only means *nothing goes out*, not *nothing comes in*. A record only the
address book has (it matches no saved contact and is not ambiguous) is an
`import`, in both modes: a saved contact in Postio with its addresses, phones,
organisation and title. A record with no usable address, no name or
organisation, a kind outside the scope, or an address another import already
claims stays in Contacts and is counted. A record with an organisation and no
name is a Company; otherwise a Person.

Nothing is applied from the preview itself. The dry-run sheet offers one button,
"Bring N into Postio", which applies **only the Postio side** of the plan the
person was shown:

- creates the saved contacts (a sender Postio already knows from mail, and has
  not saved, is saved and takes the record's details; a saved owner of any of
  the addresses leaves the record where it is);
- applies the `Side::Local` updates (what Contacts changed, taken field by
  field, addresses and phones as sets);
- stores a link per pair with `base` = the record's fields, so the next sync is
  a three-way comparison (a pair with an open conflict is not linked).

The click is bound to the plan by its fingerprint. The engine plans again from
the snapshot it is handed and refuses ("Contacts changed since the preview.
Preview again.") if the fingerprint differs. This path writes Postio's own
database. It never calls `apply` or `write`, so the gate below is untouched:
`CONTACT_SYNC_WRITES` stays `false` and the adapter's `apply` still throws.

## The gate

1. `Account.writes` is 0 for every account; the runner calls `apply` only when it
   is 1 **and** the build constant `CONTACT_SYNC_WRITES` is true.
2. `CONTACT_SYNC_WRITES` is `false`. It becomes `true` in a commit the maintainer
   makes after reading a dry-run report; no setting and no environment variable
   flips it.
3. The Swift adapter's `apply` throws `notEnabled` regardless of Rust's state,
   until the same commit flips its own constant.
4. While the gate is closed, a sync run is: snapshot, plan, store adoptions and
   `base`, **never** `apply`. Adoptions (links) are Postio-side writes.

## The dry-run report

`Plan` rendered for a person: counts and, per entry, the contact, the field and
both values. It is shown in a Settings sheet or written to a file the person
chose. **It is never written to a log**; logs carry the counts and the account id
only.

## Permissions and consent

- `request_access` runs when the person turns "Contacts on this Mac" on and not
  before; `denied` is a shown final state until they turn it off and on.
- A CardDAV account is added by the person with its credentials in the OS keyring;
  each connection writes an `egress_log` row. Nothing runs speculatively.
- No account, no call: a store with sync off makes no permission request and no
  network request (SC-006).

## Tests never touch the real address book

- `postio-contact-sync` is pure and tested over fake snapshots.
- The runner and the Swift adapter are tested against in-memory fakes of the trait.
- The one test that reads a real store is `#[ignore]` ("this machine may lack what
  the test needs"), refuses to start unless `POSTIO_SCRATCH_CONTACTS_CONTAINER`
  names a local container made for it, enumerates only that container, and writes
  only in a session the maintainer starts after the gate has been lifted.
