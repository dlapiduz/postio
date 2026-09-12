# Quickstart: validating the Search and Command Bar

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

How to prove this feature works, in the order that fails fastest. Sections 1–5
are runnable; section 6 is the part that needs a person.

**Every command below carries the number of tests it should select.** That is
not decoration: `test result: ok. 0 passed` and `ok. 1 passed` look alike at a
glance and mean opposite things, and a command that matched nothing reported
success while running nothing during the last feature's walkthrough. Where a
count reads `[n]`, the implementation fills it in — a task is not done while
its own count is a placeholder.

## Prerequisites

```bash
cd ~/src/postio-worktrees/search-command-bar    # this feature's worktree
scripts/install-nextest.sh                      # pinned nextest, if not already present
```

No account, no network and no display are needed for sections 1–4. Test
binaries are put on a private compositor automatically.

## 1. The registry knows the destinations — cheapest layer

```bash
cargo test -p postio-core --lib                                    # 81 passed
cargo nextest run -p postio-core --test core_suite command_registry # 20 passed
```

Expect: every new id has a title and a non-empty default binding; no binding
collides with `g g`, `g f` or `g a`; each is reachable in a list context.

## 2. The generated documentation followed

*(The generator moved to `postio-ui` during implementation: the document is
rendered from two tables now, and only that crate can see both. See
[research.md](./research.md) R5 and R6.)*

```bash
cargo nextest run -p postio-ui --test ui_suite keybindings_doc      # 2 passed
git diff --stat docs/keybindings.md
```

Expect: the test regenerates `docs/keybindings.md` and fails if the checked-in
file drifted. The diff shows the four destination rows **and** the modes
section. If the modes section is absent, FR-032 is not met — the table is being
read by the bar and not by the docs.

## 3. The mode table is shared and whole

```bash
cargo test -p postio-ui --lib finder                                # 3 passed
```

Expect: prefixes unique, exactly one mode without one, every mode named and
explained. `postio-ui` must still compile with no toolkit dependency —
section 5 is what checks that.

## 4. The bar and the keys, where a person would see them

```bash
cargo nextest run -p postio-gtk --test gtk_suite gtk_finder         # 4 passed
cargo nextest run -p postio-gtk --test gtk_suite gtk_cheatsheet     # 1 passed
cargo nextest run -p postio-gtk --test gtk_suite gtk_go_to           # 1 passed
cargo nextest run -p postio-gtk --test gtk_accessibility             # 1 passed
cargo test -p postio-app --test app_suite                           # 89 passed, 1 failed -- see below
```

**One failure in that third command is expected and is not yours.**
`render_dedup::one_gesture_renders_once_and_reselecting_renders_nothing`
fails in a full run and passes alone: it is order-dependent, it predates this
feature, and it is #1497. The baseline when this feature started was
`89 passed; 1 failed`. If you see two failures, the second one is yours.

The third is the one that matters, and the reason is in
[research.md](./research.md) R8: the GTK cases hand the finder a fixture and
assert a handler fired, which would pass unchanged if nothing in the running
application ever fed it. **The `app_suite` case must press `g i` at the
composition root and assert on the folder then showing.** A green section 4
whose `app_suite` count did not go up has not tested this feature.

## 5. The invariants

```bash
scripts/check.sh                                                    # all clean
```

Expect: crate boundaries clean — this is what refuses a GTK dependency creeping
into `postio-ui` when the mode table moves there.

## 6. By eye — what no test here covers

```bash
cargo run -p postio-app
```

1. **The bar says what it can do.** Look at it before typing. Then open it and look again before typing a prefix. Both should tell you the box does more than search mail; neither should be in your way once you start typing.
2. **`g i` from the message list** goes to the inbox. Then `g d`, `g t`, `g s`.
3. **`g` in the composer** types a letter. Press `c`, type `going`, and confirm no navigation happened.
4. **An absent destination says so.** On an account with no Drafts folder, `g d` should tell you, not appear to do nothing.
5. **The route you could not find before.** Type `#`, then a few letters of a folder deep in the tree, and go there.

Items 1 and 5 are the feature's whole point and the only steps that can judge
it: everything above proves the mechanism, and these two are whether a person
can find the thing.
