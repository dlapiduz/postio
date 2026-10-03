# Contract: Runners, the Driver Script, and the Output Tree

A **runner** plays storyboards against one app. **`scripts/storyboards.sh`**
is the one command people and skills use. **`postio-storyboard`** is the pure
tool behind the script, for loading, comparing and building pages. The script
is the interface, and the other two are its parts.

## The runner (one per app)

Focus is the one desktop app (ADR 0043), and its runner is the one design
review plays on: `postio_focus::demo::storyboard`, over the demo store
`postio_focus::demo` shares with `shot`, built as `postio-focus`'s
`storyboard` example (specs/007-postio-focus T265). The GTK half it drives
the window with -- chain delivery, typing, reachability, settling, the
outline -- is `postio_widgets::storyboard`. The classic app's runner
(`postio-app`) is still built until T256 removes it.

```text
cargo run -p postio-focus --example storyboard --features demo -- <subcommand>
cargo run -p postio-app --example storyboard --features demo -- <subcommand>   # until T256
```

**Seeds and presets.** Focus's runner builds every seed the catalogue names
but `first-run`, whose orientation strip Focus dropped (`classic-parity.md`
row 12): `small` (today's inbox), `empty`, `long-newsletter`, `long-thread`,
`thirty-threads`, `two-accounts`, `outbox`, `draft-left-over` and
`backfilling`. The store halves of the shared ones are
`postio_storage::seed`'s. Its presets are the window conditions no key
reaches without a server or a person: `settings` and `settings/<section>`
(every section Focus shows), `settings/account-form`,
`settings/signature-editor`, `settings/account-weights`,
`add-account/{route,browser,syncwindow}` and `locked`. Its variant axes are
`scheme`, `width` and `text`; Focus has no `contrast` or `density`
(`classic-parity.md` rows 18 and 19).

| Subcommand | Does | Output |
|---|---|---|
| `list` | Describes this runner: app, delivery modes, seeds, presets, variant axes and contexts with their starting state | JSON on stdout |
| `run <storyboard.toml>... --out <dir> [--variant axis=value]... [--delivery chain\|direct] [--no-frames]` | Plays each storyboard in every variant it asks for that this app supports | the output tree below; exits non-zero on any `failed` or `error` (FR-014) |
| `every-command --out <dir> [--gaps storyboards/gaps/<app>.toml]` | The generated pass (US6): each bound command, in each context, from that context's starting state | a `coverage.json` listing `effect`, `no_effect`, `listed_gap` and `stale_gap`; exits non-zero on any `no_effect` or `stale_gap` |

The runner calls `postio-storyboard`'s library for parsing, applicability,
check evaluation and writing `run.json`. It never re-implements them, so two
runners cannot disagree about what a storyboard means.

**Hermetic re-exec (research R5).** On start, the runner re-executes itself
with:
- `TZ=UTC` and `LANG=C.UTF-8`;
- a `FONTCONFIG_FILE` holding only the embedded faces;
- a temporary `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME`;
- `GTK_A11Y=test`;
- animations off, and the clock frozen.

It never opens the user's store and never touches the network (FR-013).

## The output tree

```text
<out>/
└── <app>/
    └── <storyboard>/
        └── <variant-key>/          # e.g. scheme=dark,width=narrow, or "default"
            ├── run.json            # data-model § Run
            ├── 00.png              # starting state, plain
            ├── 00.outlined.png     # starting state, focus outlined and captioned
            ├── 01.png
            ├── 01.outlined.png
            ├── 01.s3.png           # extra frames named by jumped/blanked
            └── ...
```

Frame names are step numbers, zero-padded, with `00` as the starting state.
Paths inside `run.json` are relative to the `run.json` itself, so a tree can be
moved or cached intact (research R8).

## `scripts/storyboards.sh`

```text
scripts/storyboards.sh run     [--app focus|classic|all] [--only <glob>] [--variants] [--no-frames]
                               [--delivery chain|direct] [--calibration]
scripts/storyboards.sh bundle  --acceptance <file> [--calibration]   # what a reviewer reads
scripts/storyboards.sh tool    <postio-storyboard arguments>          # the pure tool, built
scripts/storyboards.sh base    [--app ...]           # run on the merge-base (cached)
scripts/storyboards.sh page    [--open]              # build Design/review/<branch>/index.html + summary.md
scripts/storyboards.sh key                           # print the review key for this tree (R13)
scripts/storyboards.sh screens [--only <glob>]       # the contact sheet; replaces screens.sh
scripts/storyboards.sh lint                          # load and lint the whole catalogue
scripts/storyboards.sh coverage [--app ...]          # every-command, per app
```

- **`--app` is Focus unless it says otherwise**, for every subcommand that
  plays: `run`, `base`, `screens` and `coverage`. `classic` plays the classic
  runner until T256 removes it; `all` plays every runner whose crate is on
  the branch.
- **There is no `--changed`.** A selection by changed crates would select,
  for a GTK app, every storyboard applying to it -- a shared type's callers
  are not knowable from a diff (#419) -- so the flag could never narrow
  anything, and SC-002 makes the whole set affordable. What narrows a review
  is the comparison with the base (`base`, then `bundle --base`): a reviewer
  is asked only about runs that are new or whose frames or observations
  changed.
- **`--variants`** runs each storyboard's `vary` matrix. Without it, only the
  default variant runs. The default run is the per-edit loop, and variants are
  the pre-review sweep.
- **Default output** is `Design/review/<branch>/runs/`, which is gitignored.
  Base runs go to the cache at `~/.cache/postio/storyboards/` (research R8).
  The script links them into `Design/review/<branch>/base/`.
- **The runner binary is built once per invocation**, the same way
  `screens.sh` builds `shot` once today. It never runs `cargo run` per
  storyboard.
- **Exit codes.** `0` means everything ran and passed. `1` means a storyboard
  failed. `2` means a storyboard failed to load, or a runner errored. A
  storyboard that is not applicable or not covered never makes it non-zero.
  It is counted and shown.

## `postio-storyboard` (bin)

The script calls this. People rarely do.

| Subcommand | Does |
|---|---|
| `lint <dir>` | Loads every storyboard; validates per data-model; prints applicability per app |
| `key --tree <path=id>...` | Prints the review key for those trees |
| `compare --base <dir> --branch <dir> [--out <file>]` | Writes `comparison.json` (data-model § Comparison) |
| `sheet --runs <dir> --catalogue <dir> --design-dir <dir> --out <file>` | The screen sweep's contact sheet, design beside app |
| `bundle --runs <dir> [--base <dir>] --acceptance <file> --catalogue <dir> --design-dir <dir> --out <dir>` | Writes the review bundle (contracts/review.md) |
| `prompt <bundle> (--list \| --batch <n>)` | The batch count, or one batch's reviewer prompt, from the fixed template |
| `verdicts merge <bundle>` | Merges `verdicts.N.json` into `verdicts.json` |
| `verdicts check <bundle>` | Validates the citations and completeness (FR-019) |
| `page --runs <dir> --out <file> [--prefix <p>] [--title <t>] [--key <k>]` | The page; parity rows are drawn on it, not written to a file |

Every invocation of the script plays on a compositor of its own
(`postio-storyboard-<pid>`, 1920×1200), stopped on exit: on the shared one,
whether a window was the active one depended on who else was running, and
an inactive window draws in GTK's backdrop style.

## In the suites (research R14)

| Suite | Case | Runs |
|---|---|---|
| `postio-storyboard` lib tests (sanity tier) | `the_catalogue_loads_and_lints` | the lint over `storyboards/` |
| `postio-widgets` `widgets_suite` | `storyboard_*` (19) | the GTK half: chain delivery, typing, reachability, settling, the outline |
| `postio-focus` `focus_suite` | `storyboards`, `observe`, `storyboard_determinism` | a run is pressed, checked and written; the window says where everything is; two processes film one storyboard identically |
| `postio-focus` `focus_suite`, nightly | `storyboard_catalogue`, `every_command` | the whole catalogue holds on Focus; every bound command shows or is a listed gap (`POSTIO-MEASUREMENT`) |
| `postio-app` `app_suite` | the classic app's five | the same for the classic app, until T256 removes it |

A failing storyboard is named in the catalogue case's panic message, with
the step and the check. A storyboard whose defect is still open (`proof =
"open"`) is red by design and counts as expected there.
