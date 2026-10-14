# Postio domain language

The words the code, the design and the people building Postio share. One
word, one meaning; where two words were in use, the one named here wins and
the other is listed under _Avoid_.

## Search

Defined by `specs/010-focus-search` (spec, data model, and the design in
`Design/focus-macos-search/SPEC.md`). The vocabulary lives in `postio-search`
(pure), `postio-focus` (the controller) and `postio-ui` (the words shown).

**Query**:
The one string a search is, held by `postio-focus` (FR-001). Everything
else (chips, the plain-English bar, the dropdown) is a view of it, and
editing any of them rewrites the string.
_Avoid_: search text, filter state.

**Term**:
One token of a parsed query: an operator clause, a partial one still being
typed, or a free word, with its span in the string. The design's word for what
the code calls a token.
_Avoid_: token (in prose about the design), keyword.

**Chip**:
A term drawn as a pill in the query field: an operator with its value, or
the same struck through when excluded. A chip is a way of showing and editing a
term, never a second copy of it.
_Avoid_: tag, token field.

**Operator**:
The name before the colon that says which field a term constrains: `from:`,
`to:`, `in:`, `label:`, `has:`, and the date operators. Providers are not
operators; operators are the same for every account.

**Value set**:
Two or more values of one name-valued operator that match if any does, written
`from:{ada,priya}` (D26). Never mixed across operators, never nested.
_Avoid_: OR group, list.

**Relaxation**:
One term dropped or loosened in a query that found nothing, with the count the
loosened query would find ("Remove “before March”"). It is offered, never
applied (the no-results page, US6).
_Avoid_: suggestion (that is a completion), fallback.

**Passage**:
The window of text around a match, with the matched words marked, shown under a
result and read aloud in full: highlights are never the only signal.

**Source tag**:
The quiet word saying where a match was found: subject, body, quoted text, file
name, or a file's own location (page, sheet and row, slide). It decides how a
screen reader says a result: "matched in body: …".

**Facet**:
A count by one property of the results (sender, recipient, label, folder,
attachment, action, unread) or the twelve monthly counts, taken over the same
capped match as the results. Facets feed the filter pills and the timeline.

**Timeline**:
The row of monthly bars above the results: how many conversations match in each
of the last twelve months. Dragging across it narrows the query to those
months.

**Results view**:
A mode of the main window, not a new window, that replaces the inbox with the
matches for a query: toolbar with the query field, tabs (Conversations, Files,
People), filter bar, timeline, grouped rows, footer. Esc returns to the inbox;
history holds its entries like the inbox's (D17).
_Avoid_: search window, search page.

**Top hits**:
The first group of the results in Best match order: at most three
conversations that ranked clearly above the rest, each saying why. Absent when
sorted by Newest.

**Rank reason**:
The short phrase under a top hit saying why it ranked: you replied, you flagged
it, a frequent sender, in the subject, in a file name, or "N matches". At most
two, joined by " · " (D20). The words are `postio-ui`'s; the executor supplies
only the reasons.

**Saved search**:
A named query kept in `config.toml`, optionally pinned to the sidebar and
optionally notifying, with the instant it was last seen so "3 new" can be
counted. Saving takes a name and two toggles (the Save popover).

**Recent search**:
A query that was *run* (the results view opened, a hit opened, a saved search
run), never each keystroke. Stored in the encrypted store, never logged (D16).

**Quick Look in search**:
Space over a result or file card: a panel over the main window showing the
message or the attachment without opening it. j/k and ]/[ move through the
results with the panel up; Esc closes it. Only files already on this Mac are
previewed, and nothing is fetched to do it.

**Dropdown**:
The panel under the toolbar field while it is focused: recents and saved
searches when empty, top hits, people, labels and completions while typing.
Its highlight belongs to the toolkit; what a row runs belongs to the
controller. To a screen reader the field is a combobox and the dropdown its
listbox.
