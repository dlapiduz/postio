# Contract: the `with:` search field

One new field in the one query language (Principle III; research R6).

## Syntax

```text
with:<address>[,<address>…]
```

- Keyword `with`. No aliases.
- The value is one or more addresses separated by commas, no spaces
  (`with:ada@work.example,ada@home.example`). Quoted values follow the
  existing quoting rules.
- Negatable like every field: `-with:ada@example.org`.
- A half-typed value (`with:`, `with:ada@`, a trailing comma) is an ordinary
  intermediate state and never an error; it matches as far as it can be read,
  like the rest of the parser.

## Meaning

A message matches when **any** listed address, compared by normalised form,
appears in its `From`, `To`, `Cc` or `Bcc`. Exact address, not full text —
`with:ada@example.org` does not match `ada@example.org.invalid` or a display
name containing the string. An address the store has never seen matches
nothing (never everything — the rule `group:` and `account:` follow).

Several `with:` tokens are ANDed like any two filters: mail involving both
people.

## Who writes it

- `@` finder, picking a person: `with:` + every address of the person at that
  moment (replacing today's `from:<address>`, `finder.rs:392`).
- Contacts screen, `contact_show_mail`: the same string.

## Where it is implemented

`postio-search`: `Field::With`, `Filter::With(Vec<String>)`, parser arm, parse
tests. `postio-index/src/executor.rs::filter_condition`: resolved through
`recipients.address_id` against `addresses.address_normalized IN (…)`,
`kind IN ('from','to','cc','bcc')`. The shortcut and config references
regenerate (`docs/keybindings.md` search operators, `docs/config.md`).
