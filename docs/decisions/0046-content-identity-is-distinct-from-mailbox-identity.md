# 0046 — Content identity is distinct from mailbox identity

Status: Accepted (2026-10-05), implementation in #1780; amended 2026-11-22 (#1526)

## Context

`messages.id` identifies a mailbox occurrence. It is the target of flags,
pending operations, snoozes and remote mutations. Keeping decoded bodies and
search documents on that identity repeats both when a backend exposes the
same immutable message in several folders.

JMAP exposes one Email with several mailboxIds; its body properties are
immutable ([RFC 8621 §4.1](https://www.rfc-editor.org/rfc/rfc8621.html#section-4.1)).
Gmail exposes one immutable message ID with several labelIds
([Message resource](https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages)).
Ordinary IMAP has no comparable account-wide guarantee. Its UID is scoped to
a mailbox and UIDVALIDITY; RFC Message-ID can be reused or forged and is not
evidence that two bodies are identical.

## Decision

Keep `messages` as the occurrence and the bounded mailbox list projection.
A separate `message_contents` row owns decoded text, HTML and header bytes.
Backends may supply an explicit `ContentIdentity { namespace, key }` only
when they guarantee that it names immutable content across mailboxes.
Storage scopes that identity to the local account. An absent identity
allocates independent content, even when remote coordinates or RFC IDs match.

Gmail message IDs and JMAP Email IDs opt in under different namespaces.
IMAP, Maildir and ordinary mocks do not opt in. A backend's addressing ID
alone never opts it in implicitly. Protocol adapters own this guarantee;
the sync engine consumes it through the existing MailBackend seam.

Content sharing must preserve location IDs, mailbox addresses, flags and
pending operations. A search document belongs to content, and search chooses
a qualifying occurrence after applying account, folder and flag constraints.
An Archive match must therefore return an Archive mutation target even if
an earlier Inbox occurrence shares the same content.

The list's envelope, availability and MIME metadata remain small read
projections on occurrences. Reusing downloaded content must also reuse its
MIME parts and availability; otherwise a late membership would either fetch
again or claim to be readable while its inline images were missing.
Removing a membership retains shared content; removing the last collects it.

The forward migration gives each existing occurrence independent content.
It never infers identity from old RFC IDs or mailbox coordinates. Derived
search data may be rebuilt. Unknown schema fingerprints remain refused.

### Exact copies collapse in search

> **Amended 2026-11-22, by the maintainer (#1526).** IMAP occurrences still
> never share content. Search alone also collapses an occurrence into
> another that is an **exact copy** of it: the same account, the same
> `rfc_message_id`, the same size (non-zero), the same `Date`, and the same
> people, From included (`people_ids`). A message a person filed in both
> `Sent` and iCloud's `Sent Messages` is then one hit, as a Gmail message
> with two labels is.

The collapse extends the content path rather than adding one. The earliest
qualifying occurrence stands for the copies as for content siblings, and the
counts and folder facets count it once, in every folder that holds a copy.
A copy stands only if it matched as well: it has content of its own, whose
body may not be indexed yet. Storage, flags, bodies and mutation targets are
untouched.

## Alternatives

* RFC Message-ID search collapse **alone** was rejected: it cannot establish
  byte identity, saves no body storage and can conceal distinct messages. A
  reused or forged Message-ID on a message of another size, date or sender
  still shows as its own hit; the amendment above collapses only when every
  one of those agrees.
* Sharing content between exact IMAP copies was not taken: matching
  envelopes do not prove the bodies identical, and a shared body is what the
  reader would then show for both.
* Inferring content identity from `remote_id` was rejected: IMAP coordinates
  can repeat in different folders and other backends have different scopes.
* Replacing occurrence IDs with content IDs was rejected: it would make
  flag and move operations ambiguous and cross a working local-first seam.
* Removing every list projection was rejected: sharing large payloads does
  not require adding content joins to the windowed mailbox list.

## Validation

The implementation must prove sharing and non-sharing, late membership
reuse, last-membership cleanup, scoped search and retained remote mutation
targets without a live server. Read paths must retain indexed, bounded
lookups and the existing mailbox paging guarantees.
