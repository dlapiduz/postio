# Contract: `config.toml` for Focus

**The user's decisions live in `config.toml`, not the store.** A schema change
makes the store resync, and anything held only there is lost (research R0,
R14).

**Every write Focus makes to the file uses the path the settings panel
already uses.** That is `toml_edit`, which keeps what Focus does not own,
followed by `postio_config::save::write_atomically`, the save an editor makes
(`crates/postio-config/src/save.rs`). A restore, a "stop digesting" and a new
rule each write this way.

**The change reaches every running surface through `ConfigWatcher`,** exactly
as `$EDITOR`'s would. `ConfigChanged` gains a `focus` flag
(`crates/postio-config/src/change.rs:41-62`).

## `[focus]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `filtering` | bool | `true` | FR-119. `false` stops filing new mail into Filtered. What is already filtered stays where it is |

Keys this version does not know are kept, as `[tui]` keeps them (`extras`).

## `[focus.filter]`

| Key | Type | Meaning |
|---|---|---|
| `never` | array of strings | Senders that are never filtered: pinned (FR-111), or restored from Filtered (FR-116). An entry is an address (`ada@example.org`) or a whole domain (`@example.org`) |
| `stop_markers` | array of `{ sender, kind }` | Marker kinds the user stopped for a sender, by repeated dismissal (FR-108). `kind` is `question` or `todo`. As built (T118) it is its own array of tables, `[[focus.stop_markers]]`, and a third dismissal writes one entry, which undo takes back |

Validation reports an entry that is not an address or `@domain`. It reports
the entry by position, never by content, in the log.

## `[[focus.digests]]`

The file's order is the order the rules are listed and matched in, as
`[[rules]]` is in ADR 0008 (Q4). The first rule that matches holds the
message.

| Key | Type | Meaning |
|---|---|---|
| `name` | string, unique | What the digest row and the rules list call it ("Newsletters"). It defaults to the sender's display name |
| `match` | array of strings | Queries in the one query language. The rule holds a message when **any** of them matches (FR-127). Milestone 1 writes only `from:<address>`, and milestone 2 adds `list:` and free queries |
| `cadence` | `"daily"` \| `"weekly"` \| `"monthly"` | |
| `day` | weekday name (weekly) \| `1`–`28` (monthly) | Absent for daily |
| `at` | `"HH:MM"` | Local time |

**When a rule is due.** The next delivery is the next occurrence of
(`cadence`, `day`, `at`) after the previous delivery, or after the rule was
created, in the local zone.

- `postio_ui::schedule::next_due` computes it, generic over the zone. It
  steps calendar days, and resolves the time with
  `postio_search::date::resolve_local`, as `parse_when` does. A time the clocks
  skip is pushed forward by the gap. A time they repeat comes due once, at its
  first occurrence, not twice.
- Monthly days stop at 28, so every month has one.

**Validation** reports:

- an unparsable query, by the rule's name and the query's index. The check
  is split, because `postio-config` does not parse queries
  (docs/ARCHITECTURE.md). `postio-config` reports a blank query, and
  `postio_ui::digest::unreadable_queries` reports an operator the language
  cannot read;
- an unknown cadence;
- a day that does not fit the cadence;
- a duplicate name.

A rule that fails validation is not applied, and the others still are
(ADR 0008, Q6).

## `[focus.model]` (milestone 2; absent means off)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `endpoint` | URL or socket path | none | `http://127.0.0.1:<port>/v1`, `http://[::1]:<port>/v1`, `http://localhost:<port>/v1`, or a `unix:` socket path. **Anything else is refused**, with a message naming why: "the model must run on this computer" (FR-168) |
| `model` | string | none | The model name the runtime serves |
| `needs_action` | bool | `true` | Use the model for questions and to-dos, in place of the built-in detector (FR-107) |
| `digest_summary` | bool | `true` | Write digest summaries (FR-172) |
| `like_this` | bool | `true` | Offer "Digest mail like this" (FR-171) |

**How the model is used:**

- **Nothing connects to a runtime unless this section is present**
  (FR-166). Postio never probes a port.
- **The runtime is spoken to through the OpenAI-compatible
  `/chat/completions`** that Ollama and llama.cpp's server both serve. Output
  is constrained by `response_format: json_schema` and validated on the
  client (research R16).
- **No runtime or model is named in code** (FR-169).

## `[focus.vault]` (milestone 3)

| Key | Type | Meaning |
|---|---|---|
| `path` | path | The Obsidian vault folder |
| `tasks_note` | path relative to `path` | Where tasks go when no project is chosen. Default: `Tasks.md` |
| `projects` | path relative to `path` | A folder of project notes, in addition to `type: project` frontmatter |

## `[keys]`

The format is unchanged: overrides only, by command id
(`crates/postio-config/src/keys.rs`).

- **What changes is the defaults under it,** which become the one keymap for
  every app ([keymap.md](./keymap.md)).
- **An override applies in every app that has the command.** There is no
  per-app table and no `keys.toml` (FR-081).
- **Renamed ids are simply new names.** `mark_unread` becomes `toggle_read`,
  and `focus_sidebar` becomes `go_to_folders`. The validator reports an
  override naming an id that no longer exists
  (`crates/postio-config/src/validate.rs:402`), and there is no alias (no
  backwards compatibility, constitution).
