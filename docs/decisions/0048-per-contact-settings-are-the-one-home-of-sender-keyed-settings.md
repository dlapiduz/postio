# ADR 0048 — Per-contact settings are the one home of sender-keyed settings

- **Status:** Accepted (2026-10-20). Built for pins, markers, one-sender
  digests and the HTML treatment; notify, remind-after and archive-after are
  columns waiting for the surfaces that set them
- **Related:** [specs/011-focus-contacts](../../specs/011-focus-contacts/plan.md)
  (section 4.3 and slice S3), [specs/005-contacts](../../specs/005-contacts/spec.md)
  (a contact is a person who owns addresses), [ADR 0008](0008-filters-and-rules.md)
  (digest rules are queries)
- **Decision:** **What a person decided about one correspondent -- where
  their mail arrives, which markers are off, how their HTML mail is drawn --
  is kept on the contact (`contact_settings`), not per address in a file.
  The engine reads the file and the contacts together: the contacts'
  decisions are added to `[focus]` as the entries the file would have held,
  so every decision function answers for both homes. What is not one
  correspondent's -- a domain, a `list:` or `subject:` rule, a rule over
  several senders -- stays in `config.toml`. The remote-image grants are a
  privacy rule and stay where they are.**

---

## Context

Four settings were keyed by a sender, each in its own home and each by an
address, a domain or a query: a digest rule (`[[focus.digests]]`, keyed by a
query), a pinned sender (`[focus.filter] never`), a marker kind turned off
(`stop_markers`), and the treatment a person chose for a sender's HTML (a
`[Treatment]` section of the remote-image list). A person is not an address.
Joining two addresses into one contact left each setting behind on the
address it was written for, so the person's second address was filtered, held
or drawn differently from the first, and a settings screen for a contact had
four places to read and write.

## Decision

1. **`contact_settings`** holds one row per correspondent who decided
   something: `arrival` (`inbox`, `digest`, `filtered`), the digest's name,
   cadence, day and time, the marker kinds that are off (a bitmask),
   `notify`, `html_treatment` (`app` or `paper`), and the remind and archive
   periods. A correspondent who decided nothing has no row.
2. **The engine reads both.** `FocusConfig::with_contacts` adds what
   correspondents decided as `never` entries, `stop_markers` entries and
   `[[focus.digests]]` rules (one `from:` query per address). Focus mode
   installs the result, and `FocusSettings` republishes it whenever a verb
   changes a correspondent. Nothing downstream of the configuration knows
   there are two homes.
3. **Writers write the person.** Pinning an address, turning a marker off for
   one and saving a rule that is one `from:` query for one address land on
   the contact; taking one back clears it from whichever home holds it. A
   pin for someone who already has a digest, and a second digest for one
   person, stay in the file, because one `arrival` cannot be both and the
   file can say both.
4. **A one-shot move.** The first time Focus mode starts with a config file,
   what a correspondent can hold is given to them and taken out of the file,
   which is saved as an editor saves it, and the file as it was is kept once
   as `config.toml.pre-contacts`. The `[Treatment]` section of the
   remote-image list moves the same way. A move changes no decision: a
   fixture files a corpus under the file-only engine and under the moved one
   and compares folder, reason and digest for every message.
5. **A digest keeps its name.** The mail it holds and the deliveries it made
   are filed under the rule's name, so the contact's digest carries the name
   of the rule it came from. A page names whose digest it is.

## Consequences

- A second address joined to a contact is covered by what the contact
  already decided. (Merging does not yet carry settings from the absorbed
  contact to the survivor; see the spec's Q8.)
- A digest that is a correspondent's is theirs as a whole: stopping it for
  one of their addresses stops it for the person.
- The Filtering page lists a person's pin once, by name, with the addresses
  it covers.
- A frontend that edits `config.toml` as text cannot take a person's decision
  back by editing the file; it asks the host.

## What would falsify it

If a setting that must stay per address appears (say, a pin for one address
of a person whose other address should be filtered), `arrival` would need an
address scope and this ADR would be rewritten to say where.

## Alternatives rejected

- **Keep the file and add a contact-aware reader.** The file is keyed by
  address, so a merge would still need to rewrite it, and a setting edited in
  two places is the "two answers" this exists to end.
- **Move everything, including domains and list rules.** Those are not
  correspondents. A domain is not a contact, and a list rule is a query.
- **Move the remote-image grants too.** They are a privacy rule that fails
  closed, and a second home for them is the risk their own module header
  names.
