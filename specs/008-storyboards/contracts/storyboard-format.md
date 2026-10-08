# Contract: The Storyboard File

Every runner reads this file, now and later. Entities and their rules are in
[../data-model.md](../data-model.md). Why it is TOML, and why sentinels are
tables, is in research R2.

When this lands, `storyboards/README.md` becomes the living reference, written
from this contract. This file is the design record, and the README is what
people read. If the two disagree, the README is wrong only if this contract is
amended in the same commit.

## Layout

```text
storyboards/
├── README.md              # the living reference (from this contract)
├── list/                  # one directory per surface
├── search/
├── reader/
├── conversation/
├── compose/
├── sidebar/
├── settings/
├── onboarding/
├── flows/                 # end-to-end flows (FR-026)
├── screens/               # zero-step storyboards: the old screens.sh table (FR-030)
├── calibration/           # the reviewer's known-true and known-false set (R10)
└── gaps/
    └── focus.toml         # the app's, for the generated pass
```

The directory is the surface, for batching reviews (R9). It never changes the
meaning of a storyboard.

## Grammar

```toml
# Top level. Only `source` is required; `name` defaults to the file stem.
name    = "archive-walks-down"
source  = { kind = "issue", ref = "#1687" }
proof   = "pinned"              # base | open | pinned, required when kind = issue
seed    = "thirty-threads"      # default "small"
preset  = "settings/account-form"   # optional, app-declared
apps    = "auto"                # or ["focus"], ["focus", "terminal"]
vary    = { scheme = ["light", "dark"], width = ["wide", "narrow"] }
design  = "01-inbox-reading"
routing = "chain"               # or "real": the whole storyboard needs real routing

[[step]]
id      = "down"                # optional; defaults to the 1-based index
command = "select_next"         # a CommandId string id, as in [keys]
check   = { keyboard.region = "list", cursor.index = 1 }

[[step]]
key     = "mod+z"               # a raw chord, when the key itself is the subject

[[step]]
type    = "invoice"             # typed into the widget that holds the keyboard

[[step]]
wait    = { until = { notice.undo = true } }   # or { ms = 200 }

[[step]]
event   = "new_mail"            # an environment change, not an input

[[step]]
command = "archive"
check   = { cursor.index = { same_as = "down" }, notice.undo = true,
            rows.first_visible = { unchanged = true } }
expect  = "The row is gone and the one below takes its place. Nothing scrolls."
settle  = { watch_ms = 500 }
design  = "06-mouse-parity-undo"
routing = "real"                # this step alone needs real routing

# Per-app overrides, keyed by app and then step id or index.
[app.focus.step.archive]
expect = "The dense row collapses; the bulk bar does not appear."

[app.focus.step.3]
skip = { reason = "Focus has no `x` selection mode; it selects with space" }
```

## Rules a reader can rely on

1. **One input per step.** A `[[step]]` with two of `command`, `key`, `type`,
   `wait` and `event` fails to load.
2. **Commands are pressed, not called.** `command = "archive"` presses the
   app's binding for archive in the context the app is in *at that step*. An
   unbound command fails that step as `unbound in <context>`.
3. **Checks are partial.** Only the fields named are checked. An empty
   `check` checks nothing. The reviewer still judges the frame.
4. **Leaves.** A literal means equals. The table forms are `same_as`,
   `changed`, `unchanged`, `absent` and `one_of`. Nothing else is special:
   `"changed"` as a string is the text "changed".
5. **Dotted keys nest.** `keyboard.region = "list"` inside an inline table is
   `{ keyboard = { region = "list" } }`. That is TOML's own rule; the format
   adds nothing to it.
6. **`same_as` looks back only.** It names an earlier step's id or index.
7. **Not applicable is not a pass.** A check on a field the app does not
   observe is reported `not applicable`, counted, and shown.
8. **Applicability.** `apps = "auto"` means every app that provides every
   command and key used, and has the named seed and preset. Naming apps
   explicitly turns a missing command into a load error.
9. **Shared storyboards check shared fields only.** `app.*` and
   `keyboard.widget` may appear only in a storyboard that names exactly one
   app.
10. **Reserved domains only.** Any address in any string ends in
    `example.com`, `.test`, `.invalid`, `.example` or `.localhost`
    (FR-027).
11. **Comments are expected.** A storyboard says *why* in a comment at the
    top. That is what the reviewer and a later reader need, and it costs
    nothing.

## A screen (FR-030)

```toml
# storyboards/screens/settings-account-form.toml
source = { kind = "design", ref = "13-settings-accounts" }
preset = "settings/account-form"
vary   = { scheme = ["light", "dark"] }
design = "13-settings-accounts"
```

This has zero steps, so the runner records one frame and one observation, of
the starting state.

## Calibration (R10)

```toml
# storyboards/calibration/archive-returns-to-top.toml
# must_fail: the #1687 defect, stated as if it were the intent.
source      = { kind = "issue", ref = "#1687" }
calibration = "must_fail"
seed        = "thirty-threads"

[[step]]
command = "select_next"
[[step]]
command = "archive"
expect  = "After archiving, the cursor returns to the first row of the list."
```

## Gap list

```toml
# storyboards/gaps/focus.toml
[[gap]]
command = "show_images"
context = "reader"
reason  = "No seeded message has blocked images in the small seed"
```
