# Contract: Runners, the Driver Script, and the Output Tree

A **runner** plays storyboards against one app. **`scripts/storyboards.sh`**
is the one command people and skills use. **`postio-storyboard`** is the pure
tool behind the script, for loading, comparing and building pages. The script
is the interface, and the other two are its parts.

## The runner (one per app)

Classic's runner is built from `postio-app`; Focus's is built from
`postio-focus`.

```text
cargo run -p postio-app --example storyboard --features demo -- <subcommand>
cargo run -p postio-focus --example storyboard --features demo -- <subcommand>
```

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
scripts/storyboards.sh run     [--app classic|focus|all] [--only <glob>] [--variants] [--no-frames]
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

- **`--changed`** selects storyboards whose app's crates changed against the
  base, plus any storyboard file that changed. For a GTK app that is every
  storyboard applying to it: there is no finer selection, because a shared
  type's callers are not knowable from a diff (#419). SC-002 keeps that
  affordable.
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
| `select --changed-crates <list> --changed-files <list>` | Prints the storyboards to run |
| `compare --base <dir> --branch <dir>` | Writes `comparison.json` (data-model § Comparison) |
| `parity --runs <dir>` | Writes `parity.json` |
| `bundle --runs <dir> --base <dir> --acceptance <file> --out <dir>` | Writes the review bundle (contracts/review.md) |
| `prompt <bundle>` | Prints the reviewer prompt, from the fixed template |
| `verdicts check <bundle>/verdicts.json` | Validates the citations and completeness (FR-019) |
| `page <bundle> --out <dir>` | Writes `index.html` and `summary.md` |
| `key --tree <git-tree-ids>...` | Prints the review key |
| `verdicts merge <bundle>` | Folds per-batch `verdicts.N.json` into `verdicts.json` |
| `sheet --runs <dir> --catalogue <dir> --design-dir <dir> --out <index.html>` | The screen sweep's contact sheet; exits 1 naming any screen that did not render |

Every invocation of the script plays on a compositor of its own
(`postio-storyboard-<pid>`, 1920×1200), stopped on exit: on the shared one,
whether a window was the active one depended on who else was running, and
an inactive window draws in GTK's backdrop style.

## In the suites (research R14)

| Suite | Case | Runs |
|---|---|---|
| `postio-storyboard` lib tests (sanity tier) | `the_catalogue_loads_and_lints` | the lint over `storyboards/` |
| `postio-app` `app_suite` | `storyboards` | `run --no-frames` over every Classic storyboard, default variant |
| `postio-focus` `focus_suite` | `storyboards` | the same for Focus (on `feature/postio-focus`) |
| nightly | `coverage` | `every-command` for each app, measured |

One case per suite leaves each harness's `--list` output unchanged
(CLAUDE.md, the harness's contract). A failing storyboard is named in that
case's panic message, with the step and the check.
