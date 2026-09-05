#!/usr/bin/env bash
# Verify, commit, push and open a PR for the issue this worktree is for.
#
# Replaces `/land` for GitHub-tracked work. The differences from the shared-tree
# ritual are all simplifications: this worktree is private, so staging
# everything is correct rather than dangerous, and there is no bead to close --
# `Closes #N` in the PR body closes the issue when it merges.
#
# Usage:
#   scripts/issue-land.sh -m "feat(gtk): teach the list to do the thing"
#   scripts/issue-land.sh                       # tree already committed; -m not needed
#   scripts/issue-land.sh -m "..." --wip        # push without opening a PR
#   scripts/issue-land.sh -m "..." --no-merge   # open the PR, do not wait
#   scripts/issue-land.sh --gates-only          # run the checks, commit nothing
#   scripts/issue-land.sh --full                # integration suites too, not just units
#   scripts/issue-land.sh --detach [args]       # the same, in a process no tool call can kill
#   scripts/issue-land.sh --status              # what the detached run did, or is doing
#
# -m is only for uncommitted work: CLAUDE.md says commit as you go, so the
# ordinary case is a clean tree with the branch's commits already on it, and
# this must not demand a message it would only throw away. #120.
#
# By default this waits for CI and merges. A PR nobody merges is work that
# looks finished and is not: it goes stale, it conflicts with whatever lands
# next, and the issue it closes stays open. The session that wrote it is the
# one that knows what to do if the checks fail, so it is the one that waits.
#
# This script rebases the tree it lives in, so a rebase that brings in new
# landing machinery would otherwise be invisible to the very run that pulled
# it in. When that happens it hands over -- execs the copy the rebase brought
# in, from the top -- so the gates and the merge decision are both the new
# machinery's. See the rebase step below and #160.
set -euo pipefail

# # Which runner the integration gates use
#
# `cargo nextest` gives every test its own process and runs test *binaries*
# concurrently, where `cargo test` runs them one after another. With 140
# binaries that is most of the cost: measured on this workspace, sccache off,
# idle box, `app_suite` goes 200s -> 20.4s and the whole workspace ~500s ->
# 118.6s.
#
# It is **not** faster at everything, and the difference decides where it is
# used here. A process per test is a process per test: the unit tier's ~1,459
# small tests become 1,459 spawns where `cargo test` runs them on threads
# inside ~19 binaries. Interleaved on a warm tree, `--workspace --lib`:
#
#     cargo test   4511ms  4648ms  4988ms
#     nextest      9311ms 11260ms 10700ms     <- 2.2x slower
#
# So: nextest for the integration tiers, `cargo test` for `--lib`. Same reason
# `scripts/test-fast.sh` and `scripts/test-sanity.sh` were left alone -- they
# are the between-edits loop, and switching them would be a tidy-looking
# regression.
#
# Fail open, like the wrappers in `.cargo/config.toml`: a machine without
# nextest runs `cargo test` and gets the same answer, slower.
#
# Defined here rather than sourced from `lib/`: `issue-land.sh` is the only
# caller, and the self-tests under `scripts/tests/` copy this file into a
# sandbox on its own -- a `source` line makes every one of them fail on a
# missing path rather than on anything they are testing.
if cargo nextest --version >/dev/null 2>&1; then
    POSTIO_TEST_RUNNER="${POSTIO_TEST_RUNNER:-nextest}"
else
    POSTIO_TEST_RUNNER="${POSTIO_TEST_RUNNER:-cargo}"
fi

# Takes `cargo test` syntax; the selectors used here mean the same to both.
run_tests() {
    if [ "$POSTIO_TEST_RUNNER" = "nextest" ]; then
        cargo nextest run "$@"
    else
        cargo test "$@"
    fi
}

# Always `cargo test`: **nextest does not run doctests**, and does not say so.
# A doctest is compiled and run by rustdoc, not by a test binary it can list,
# so anything moving off `cargo test` has to ask for them by name or ~20 of
# them stop running with nothing to report it.
run_doctests() {
    cargo test --doc "$@"
}

TREE=$(git rev-parse --show-toplevel)

# --detach: run this very script in a session of its own and return (#1129).
#
# A landing run from a tool call is killed when the call's cap runs out --
# two in one day, 79 in the transcript history -- and a killed run commits
# nothing, then re-pays the push and the CI wait. `setsid` puts the run in
# its own process group, so the tool giving up on its call cannot reach it;
# `nohup` alone where there is no setsid (macOS). Output goes to a log in
# the worktree's private git dir, so `--status` can find it from any shell
# and it goes away with the worktree. The PreToolUse hook refuses a
# foreground landing and names this flag.
LAND_LOG="$(git rev-parse --absolute-git-dir)/postio-land.log"
LAND_PID="$(git rev-parse --absolute-git-dir)/postio-land.pid"
case " $* " in
    *" --status "*)
        if [ ! -f "$LAND_LOG" ]; then
            echo "no landing has been detached in this worktree (scripts/issue-land.sh --detach)."
            exit 0
        fi
        if pid=$(cat "$LAND_PID" 2>/dev/null) && [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            echo "running (pid $pid), log: $LAND_LOG"
        else
            echo "finished, log: $LAND_LOG"
        fi
        grep -E '^\[timing\]|^issue:|^crates:|https://github.com/|^merged\.|auto-merge|MERGE DID NOT|Checks failed|hit a conflict|^Refusing|^error|^issue-land exit' "$LAND_LOG" || true
        echo "--- last lines ---"
        tail -n 5 "$LAND_LOG"
        exit 0
        ;;
    *" --detach "*)
        DETACHED_ARGS=()
        for arg in "$@"; do
            [ "$arg" = "--detach" ] || DETACHED_ARGS+=("$arg")
        done
        : > "$LAND_LOG"
        if command -v setsid >/dev/null 2>&1; then
            setsid bash -c 'bash "$0" "${@:1}"; echo "issue-land exit $?"' "$0" ${DETACHED_ARGS[@]+"${DETACHED_ARGS[@]}"} \
                > "$LAND_LOG" 2>&1 < /dev/null &
        else
            nohup bash -c 'bash "$0" "${@:1}"; echo "issue-land exit $?"' "$0" ${DETACHED_ARGS[@]+"${DETACHED_ARGS[@]}"} \
                > "$LAND_LOG" 2>&1 < /dev/null &
        fi
        printf '%s\n' "$!" > "$LAND_PID"
        echo "detached (pid $!). It lands on its own; nothing you run now can kill it."
        echo "log:    $LAND_LOG"
        echo "status: scripts/issue-land.sh --status"
        exit 0
        ;;
esac

# `.cargo/config.toml` names `postio-linker` and `postio-cc` rather than
# paths, so the compile cache is shared across worktrees (#1101); this is
# what puts them on PATH. Guarded because the self-tests copy this file into
# a sandbox on its own.
[ -x "$TREE/scripts/install-shims.sh" ] && "$TREE/scripts/install-shims.sh"
# The gates below draw compile jobs from the machine-wide pool rather than
# from `jobs = 2` (#1104). Same guard, same reason.
[ -x "$TREE/scripts/jobserver.sh" ] && eval "$("$TREE/scripts/jobserver.sh" env 2>/dev/null || true)"
BRANCH=$(git rev-parse --abbrev-ref HEAD)
# How many times one landing may hand over to a rebased copy of itself before
# it gives up rather than merging. Two is enough for the case this exists for
# -- machinery landing while a branch is being landed -- and a run that needs
# a third is one where main is moving faster than a landing takes, which a
# session should look at rather than a script should push through.
REEXEC_LIMIT="${POSTIO_LAND_REEXEC_LIMIT:-2}"

# Kept whole, because the handover below re-runs this script with exactly what
# this run was asked for; the loop underneath shifts them away.
ORIGINAL_ARGS=("$@")

MSG=""; WIP=0; GATES_ONLY=0; MERGE=1; FULL=0; WAIT=0
while [ $# -gt 0 ]; do
    case "$1" in
        -m|--message) MSG="$2"; shift 2 ;;
        --wip)        WIP=1;    shift ;;
        --gates-only) GATES_ONLY=1; shift ;;
        --no-merge)   MERGE=0;      shift ;;
        --full)       FULL=1;       shift ;;
        --wait)       WAIT=1;       shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ "$BRANCH" = "main" ]; then
    echo "Refusing to run on main. This lands a branch from a worktree;" >&2
    echo "claim an issue first with scripts/issue-claim.sh." >&2
    exit 2
fi
# `[0-9][0-9]*` rather than `[0-9]\+`: BSD sed has no `\+`, so this yielded
# the empty string on macOS and the guard below reported "not an issue branch"
# -- true-sounding, and about the wrong thing entirely. #559.
ISSUE=$(printf '%s' "$BRANCH" | sed -n 's/^issue-\([0-9][0-9]*\)-.*/\1/p')
if [ -z "$ISSUE" ]; then
    echo "Branch '$BRANCH' is not an issue branch (expected issue-<n>-<slug>)." >&2
    exit 2
fi

# Which branch this work belongs to, recorded by `issue-claim.sh` when the
# worktree was cut. Everything below -- the crate list, the rebase, the PR's
# base -- follows it. Defaulting to `main` is what keeps every ordinary
# landing exactly as it was; the alternative, a flag repeated on every
# command, is a flag somebody eventually forgets, and forgetting this one
# merges initiative work straight into `main`. #290.
BASE=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
if ! git ls-remote --exit-code --heads origin "$BASE" >/dev/null 2>&1; then
    echo "This worktree records base '$BASE', which is not on origin any more." >&2
    echo "Nothing was pushed. If the initiative branch was merged and deleted," >&2
    echo "rebase this branch onto main and record that instead:" >&2
    echo "    printf 'main\n' > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
    exit 2
fi

# Everything below compares against the base, and nothing here had been
# fetching it -- so the crate list, the PR body and the rebase were all reading
# whatever the last fetch happened to leave behind.
git fetch --quiet origin "$BASE"

# Which crates actually changed, so the gates run over those rather than the
# whole workspace. CI proves the workspace; this proves your own work fast.
CRATES=$(git diff --name-only "origin/$BASE...HEAD"; git status --porcelain \
         | sed 's/^...//') 
CRATES=$(printf '%s\n' $CRATES | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u)

echo "issue:  #$ISSUE"
echo "branch: $BRANCH"
echo "base:   $BASE"
echo "crates: ${CRATES:-none}"
# #178 gave every worktree its own target/ because sharing one compiled a
# worktree's crate against a sibling's. Nothing defaults this any more: these
# gates are the run a merge is staked on, so they are the last place that
# should share artifacts with whatever else is landing right now. A caller who
# genuinely wants a directory of their own still gets it -- see #253 and
# docs/engineering-notes.md.
echo "target: ${CARGO_TARGET_DIR:-$TREE/target (this worktree)}"
echo

# What this host cannot build, and what that means for the gates below.
#
# The gate chain runs over the crates this branch changed. On a host missing
# their system libraries that is not a weaker gate, it is no gate at all, and
# the work lands anyway. A macOS session cannot build `postio-gtk` or
# `postio-app`: gtk4 and libadwaita have arm64 bottles but webkitgtk has none,
# and the reader and composer are both WebKit views. CI would notice on the
# pull request, but only after the branch is pushed and only if someone reads
# it -- and on a repository where several agents work at once on different
# machines, a gate that silently proved nothing is worth refusing outright.
#
# Keyed on what the host can actually build, not on `uname`: a Linux box
# without the -dev packages is in exactly the same position, and would
# otherwise pass a check that named an operating system. #555.
#
# Derived rather than named. The roots are the two crates whose system
# libraries can be missing; the answer is those *plus everything that reaches
# them*, which is not the same set -- `postio-bench` dev-depends on
# `postio-gtk`, so it needs WebKit too and nothing in its own manifest says
# so. A hardcoded pair was right the day it was written and would go wrong
# the next time a crate dev-depends on the frontend, silently and only on
# macOS (#1152). See `scripts/unbuildable-crates.sh`.
MISSING_LIBS=$(scripts/unbuildable-crates.sh --libs)
UNBUILDABLE=$(scripts/unbuildable-crates.sh | tr '\n' ' ')
UNBUILDABLE="${UNBUILDABLE% }"

BLOCKED=""
for crate in $CRATES; do
    case " $UNBUILDABLE " in
        *" $crate "*) BLOCKED="${BLOCKED:+$BLOCKED }$crate" ;;
    esac
done
if [ -n "$BLOCKED" ]; then
    echo "This host cannot build: $BLOCKED" >&2
    echo "  missing system libraries: $MISSING_LIBS" >&2
    echo >&2
    echo "Nothing was committed and nothing was pushed. A gate that cannot run" >&2
    echo "has to say so: a crate the host cannot build is not a crate that" >&2
    echo "passed. Land this from a host that has them." >&2
    exit 2
fi

# A crate the unbuildable ones depend on still lands -- refusing would leave a
# macOS session unable to do any work at all -- but the gap goes on the PR
# rather than into somebody's memory. `postio-app` depends on every other
# workspace crate, directly or transitively, so when it is unbuildable any
# changed crate is unproven against the frontend.
VERIFY_LABEL=""
VERIFY_NOTE=""
if [ -n "$UNBUILDABLE" ] && [ -n "$CRATES" ]; then
    VERIFY_LABEL="needs-linux-verify"
    VERIFY_NOTE="Gates ran on a host that cannot build ${UNBUILDABLE// /, } (missing: $MISSING_LIBS), so this is unverified against the GTK frontend."
    echo "note: this host cannot build $UNBUILDABLE, so the changed crates were"
    echo "      never compiled against them. The PR will carry $VERIFY_LABEL."
    echo
fi

# rust-toolchain.toml pins the compiler, and RUSTUP_TOOLCHAIN in the
# environment beats it -- this workstation's mise config sets it, so a
# session builds, lints and tests on the wrong compiler while every gate here
# looks green. A warning in the log is weaker than the pin was supposed to
# give, so the value is captured for the diagnostic below and then cleared:
# every cargo invocation from here on runs on whatever rust-toolchain.toml
# names, whatever this shell exports. See docs/engineering-notes.md and #112.
HOST_RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-}"
unset RUSTUP_TOOLCHAIN

# Whether the session had committed everything before this script touched the
# tree. Captured here because `cargo fmt` is about to make that unknowable:
# after it runs, "there are uncommitted changes" no longer distinguishes work
# the session forgot to commit from whitespace this script just rewrote. See
# the amend below.
WORK_WAS_COMMITTED=0
[ -z "$(git status --porcelain)" ] && WORK_WAS_COMMITTED=1

# This worktree is private, so formatting the whole thing is safe here. In the
# shared checkout it would reach into files another session has open, which is
# why CLAUDE.md forbids it there and permits it here.
echo "--- rustfmt ---"
PHASE_START=$(date +%s)
cargo fmt --all
echo "[timing] rustfmt: $(( $(date +%s) - PHASE_START ))s"

# Stage now, before the invariants below rather than after them: this tree
# is private, so staging is safe this early too. check-no-personal-data.py
# and three of its neighbours read `git ls-files`, which only sees tracked
# files -- so a file this session is adding was invisible to them until
# some later, unrelated branch happened to run the same check once it was
# already on `main`. That is exactly how #269's PNG got through unscanned,
# and would have done the same for a real leak. #270.
git add -A

# Settle who owns the staged changes *now*, before the gates rather than after
# them. Two failure shapes, and the old ordering handled both badly by leaving
# this until after clippy and the per-crate suites had run:
#
#   * A session that committed its work and then ran this script had `cargo
#     fmt` rewrite whitespace underneath it, and was told "Uncommitted
#     changes: pass -m" -- asked for a commit message for changes it did not
#     make, about work it had already committed. The formatter's output
#     belongs to the commit it reformats, so it is amended into HEAD.
#   * A session that genuinely forgot to commit still has to say what the
#     commit is called, and finding that out is worth ten seconds, not the
#     ten-plus minutes of gates it used to run first.
#
# `git write-tree` below hashes the staged tree, and committing does not
# change it, so the gates cache key is the same either way.
# `--gates-only` is exempt: it promises to check without committing, and an
# amend is a commit. It leaves rustfmt's changes staged, as it always did.
if [ "$GATES_ONLY" != 1 ] && [ -n "$(git status --porcelain)" ] && [ -z "$MSG" ]; then
    OWN_COMMITS=$(git rev-list --count "origin/$BASE..HEAD" 2>/dev/null || echo 0)
    if [ "$WORK_WAS_COMMITTED" = 1 ] && [ "$OWN_COMMITS" -gt 0 ]; then
        echo "--- rustfmt touched a committed tree; amending ---"
        echo "Your work was committed before this run and the only changes are"
        echo "the ones cargo fmt just made, so they go into the commit they"
        echo "reformat rather than asking you for a message:"
        echo "    $(git log -1 --format=%s)"
        git commit -q --amend --no-edit
    else
        echo "Uncommitted changes: pass -m \"<type>(<scope>): <summary>\"," >&2
        echo "or commit them yourself first." >&2
        echo >&2
        echo "Checked before the gates deliberately: they take minutes, and" >&2
        echo "this answer does not change once they pass." >&2
        exit 2
    fi
fi

# Green gates are recorded against the exact content they proved, and a tree
# that has not changed a byte since is not re-proven. Long commands on this
# workstation get killed sometimes (docs/engineering-notes.md), and every
# killed landing used to re-pay clippy and the full per-crate test suite on
# a retry that changed nothing -- #109's landing paid its postio-app gates
# three times that way. `git write-tree` hashes the staged tree, staging
# just happened above, and rust-toolchain.toml is tracked: an edit, a
# rebase, whatever `cargo fmt` just rewrote, or a toolchain bump all change
# the key, so only a byte-identical tree with the same crate list skips.
# The invariants (`check.sh`, seconds) still run every time. #742.
GATES_STAMP="$(git rev-parse --git-dir)/postio-gates-green"
GATES_KEY="$(git write-tree) tier:$FULL crates:$(printf '%s' "$CRATES" | tr '\n' ' ')"
GATES_GREEN=0
if [ "$(cat "$GATES_STAMP" 2>/dev/null)" = "$GATES_KEY" ]; then
    GATES_GREEN=1
    echo "--- clippy and tests: already green for this exact tree ---"
    echo "A previous run proved this staged content with this crate list"
    echo "(recorded in $GATES_STAMP),"
    echo "so clippy and the per-crate tests are not re-run. Any change to"
    echo "the tree re-runs them; rm that file to force a re-run now."
fi

if [ "$GATES_GREEN" != 1 ]; then
    for crate in $CRATES; do
        [ -d "$TREE/crates/$crate" ] || continue
        echo "--- clippy: $crate ---"
        PHASE_START=$(date +%s)
        cargo clippy -p "$crate" --all-targets -- -D warnings
        echo "[timing] clippy $crate: $(( $(date +%s) - PHASE_START ))s"
    done

    # The test tier. Default is the whole workspace's *unit* tests: 1,313
    # tests in ~5s warm, 19 binaries instead of 197. `--full` adds the
    # per-crate integration suites, which is what this used to always do.
    #
    # The reason for the change is the machine, not the tests. Several
    # sessions share this workstation, and the per-crate integration suites
    # are minutes each -- so landing became something you queued for.
    # (This once blamed an "~11-minute compile and link" of one binary;
    # that was a cold worktree, fixed at the claim -- #1101, #1102.)
    #
    # This is safe only because something else still proves the combination:
    # CI runs the full workspace on every pull request, and the nightly run
    # does it again on a schedule. That is not a formality. The bugs this
    # project ships are layers that each pass and are not joined up -- the
    # Reader was built, tested and never mounted -- and unit tests are
    # precisely the tier that cannot see them. If the pull-request suite ever
    # goes away, this default has to go back with it (#847).
    if [ "$FULL" = 1 ]; then
        for crate in $CRATES; do
            [ -d "$TREE/crates/$crate" ] || continue
            echo "--- test (full): $crate ---"
            # Headless without asking: .cargo/config.toml's runner puts every
            # test binary on a compositor of its own.
            PHASE_START=$(date +%s)
            run_tests -p "$crate"
            # `cargo test -p X` used to cover doctests as a side effect. The
            # nextest path does not run them at all, so they are asked for by
            # name rather than quietly dropped.
            run_doctests -p "$crate"
            echo "[timing] test $crate: $(( $(date +%s) - PHASE_START ))s"
        done
    else
        # The suites the sanity tier cannot fail for (#1047).
        #
        # `--lib` proves the units and `cargo check --all-targets` proves
        # everything compiles; neither can see a test that *enumerates* a
        # vocabulary -- the golden binding table, `docs/keybindings.md`,
        # `[keys]`, `docs/config.md`. Adding one `CommandId` touches six or
        # seven places, the compiler checks two, and the branch builds and
        # then fails CI ten minutes later on an assertion about a table.
        #
        # So the crates you changed get their integration suites too, minus
        # the handful whose suites are minutes. `full-suite-crates.sh` holds
        # that rule and the measurements behind it; its self-test holds the
        # direction it has to fail in.
        for crate in $(printf '%s\n' $CRATES | scripts/full-suite-crates.sh); do
            [ -d "$TREE/crates/$crate" ] || continue
            echo "--- test (suites): $crate ---"
            PHASE_START=$(date +%s)
            # `--no-fail-fast`, because this class of change breaks several
            # tables at once and plain `cargo test` abandons the remaining
            # binaries at the first failure. #1003 fixed one table, re-landed,
            # and failed on a *different* one twenty-five minutes later; a
            # probe of this gate reproduced exactly that -- `command_registry`
            # and `keybindings_doc` both fail, and only the first is reported
            # without this. (CLAUDE.md says the same about the reconcile pass.)
            run_tests --no-fail-fast -p "$crate"
            echo "[timing] suites $crate: $(( $(date +%s) - PHASE_START ))s"
        done

        echo "--- test: workspace unit tests (sanity tier; --full for the rest) ---"
        PHASE_START=$(date +%s)
        # `cargo test` deliberately, not `run_tests`. This tier is ~1,459
        # small unit tests, and a process per test costs more than it saves
        # there -- measured 4.5s here against nextest's 9.3s on a warm tree
        # (scripts/lib/test-runner.sh has the numbers). nextest earns its
        # keep on the integration suites above, where 140 binaries are the
        # bottleneck rather than the tests inside them.
        #
        # `--lib` excludes doctests under either runner, so this tier loses
        # nothing by not calling run_doctests.
        #
        # Narrowed, not skipped, on a host missing the GTK libraries. The
        # workspace *check* below is skipped outright there and that is the
        # right call for a compile probe -- but this is the only real test
        # gate a landing has, and skipping it would let a macOS session land
        # with nothing run at all. Every crate the host can build still runs;
        # the two it cannot are excluded by name and the PR already carries
        # `needs-linux-verify` to say so.
        #
        # Without this a scripts-only change could not land from a Mac: no
        # crate changed, so the #555 hard stop correctly did not fire, and
        # then the tier compiled the whole workspace anyway and died on
        # `glib-sys` (#1152). Excluding the two obvious crates is not enough
        # either -- `postio-bench` dev-depends on `postio-gtk` and drags the
        # stack back in -- which is why the list is derived.
        SANITY_EXCLUDES=()
        for crate in $UNBUILDABLE; do
            SANITY_EXCLUDES+=(--exclude "$crate")
        done
        if [ ${#SANITY_EXCLUDES[@]} -gt 0 ]; then
            echo "note: excluding ${UNBUILDABLE// /, } -- this host cannot build them."
        fi
        cargo test --workspace --lib ${SANITY_EXCLUDES[@]+"${SANITY_EXCLUDES[@]}"}
        echo "[timing] sanity tier: $(( $(date +%s) - PHASE_START ))s"
    fi

    # Everyone else's *test* targets, against what this branch just changed.
    #
    # The gates above run over the crates you changed, which is the right
    # trade for speed and the wrong one for blast radius: a shared type that
    # gains a field breaks call sites in crates you never named, and when
    # those call sites are in test code the libraries still compile. So
    # `cargo build` is green, `cargo check -p <the crate you changed>` is
    # green, and `main` goes red for the next session unlucky enough to touch
    # the crate whose tests stopped building. That happened twice in one day
    # (#419): `Event::BackfillProgress` gained `footprint`, and six call
    # sites in postio-gtk's tests were never updated.
    #
    # `check`, not `build` or `test`: no codegen, no linking, nothing
    # executed -- the cheapest question that covers the whole workspace, and
    # the only one that would have caught either of them.
    #
    # What it costs, measured on this workstation (#419): 6m20s against a
    # cold target directory, 0.6s warm. A landing pays somewhere between,
    # depending on how much of the graph the per-crate gates above already
    # compiled -- close to nothing for a postio-gtk branch, most of the
    # frontend for a leaf-crate one. A retry after a killed run pays the warm
    # number, and the gate cache above usually skips it entirely.
    #
    # Skipped where the hard stop above would not have fired: a host without
    # the GTK libraries would fail this on system headers rather than on the
    # branch, and refusing there would stop such a host doing any work at
    # all. Its PR already carries `needs-linux-verify`.
    if [ -n "$UNBUILDABLE" ]; then
        echo "--- workspace check: skipped ---"
        echo "This host cannot build ${UNBUILDABLE// /, }, so a workspace-wide"
        echo "check would fail on system libraries rather than on this branch."
    else
        echo "--- workspace check: every crate's test targets ---"
        PHASE_START=$(date +%s)
        cargo check --workspace --all-targets
        echo "[timing] workspace check: $(( $(date +%s) - PHASE_START ))s"

        # Adding a crate is the one edit whose blast radius is definitionally
        # outside the crates it touches, so it is the one edit the per-crate
        # gate cannot describe. `postio-session/src/logging.rs` keeps a list
        # of every workspace crate and a test that fails when one is missing;
        # `postio-ui` (#566) and `postio-ffi` (#571) each landed a red
        # `postio-session` because neither branch changed it, so nothing in
        # either chain had any reason to compile that test (#585).
        #
        # `check` above cannot catch it: the test compiles fine, it just
        # fails. Only running it finds this, and the other things that
        # enumerate the workspace -- check-lint-floor.py,
        # check-crate-boundaries.py -- fail the same way for the same reason.
        #
        # Rare enough to cost nothing in the ordinary case: this fires only
        # when the root manifest's `members` actually changed, not merely
        # because Cargo.toml was touched for a dependency bump.
        if git diff -U0 "origin/$BASE...HEAD" -- Cargo.toml \
           | grep -qE '^[+-].*crates/'; then
            echo "--- workspace tests: this branch changes the members list ---"
            PHASE_START=$(date +%s)
            run_tests --workspace
            run_doctests --workspace
            echo "[timing] workspace tests: $(( $(date +%s) - PHASE_START ))s"
        fi
    fi
fi

echo "--- repository invariants ---"
PHASE_START=$(date +%s)
"$TREE/scripts/check.sh"
echo "[timing] invariants: $(( $(date +%s) - PHASE_START ))s"

# Recorded only now, after every gate above has passed -- a failure exits
# via set -e before this line, so a red run can never mark the tree green.
printf '%s\n' "$GATES_KEY" > "$GATES_STAMP"

# rust-toolchain.toml pins the compiler, so CI and this shell agree by
# construction -- the gates above ran with RUSTUP_TOOLCHAIN cleared, so
# `rustc --version` here reports the pinned compiler regardless of what this
# shell exports.
echo "--- toolchain ---"
echo "local: $(rustc --version)"
echo "pinned: $(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$TREE/rust-toolchain.toml")"
if [ -n "$HOST_RUSTUP_TOOLCHAIN" ]; then
    echo "note: this shell exports RUSTUP_TOOLCHAIN=$HOST_RUSTUP_TOOLCHAIN, which" \
         "overrides rust-toolchain.toml -- cleared above, so the gates ran" \
         "pinned regardless."
fi

[ "$GATES_ONLY" = 1 ] && { echo; echo "gates passed; nothing committed."; exit 0; }

if [ -n "$(git status --porcelain)" ]; then
    # Only reachable with a message. The no-message case was settled before
    # the gates ran -- amended into HEAD when the changes were rustfmt's own,
    # refused there otherwise -- so nobody is asked for a commit message after
    # sitting through ten minutes of clippy for an answer that was knowable
    # before it started.
    #
    # Already staged, above -- before the invariants ran rather than here.
    git commit -m "$MSG

Refs: #$ISSUE"
else
    echo "no local changes to commit"
fi

# A clean tree is not the same question as an empty branch: the guard above
# only ever asked whether *this run* had something to commit. A branch that
# never had any work on it -- claimed and landed without a line changed --
# would otherwise sail through the push and open a PR with nothing in it.
AHEAD=$(git rev-list --count "origin/$BASE..HEAD")
if [ "$AHEAD" = 0 ]; then
    echo "Nothing to land: this branch has no commits beyond origin/$BASE." >&2
    exit 2
fi

# Rebase onto current main before pushing. Other sessions land while you
# work -- four commits arrived during one recent piece of work -- and a branch
# built on a stale base means CI tests a combination that will never exist,
# the merge is a surprise, and the push can be rejected outright.
BEHIND=$(git rev-list --count "HEAD..origin/$BASE")
if [ "$BEHIND" -gt 0 ]; then
    echo "$BASE moved $BEHIND commit(s) while you worked; rebasing onto it"
    # Both sides of the comparison are read before the rebase runs, because
    # after it the question cannot be asked honestly any more.
    BEFORE_HEAD=$(git rev-parse HEAD)
    BEFORE_SCRIPTS=$(git rev-parse "HEAD:scripts" 2>/dev/null || echo none)
    if ! git rebase "origin/$BASE"; then
        git rebase --abort 2>/dev/null || true
        echo >&2
        echo "Rebase onto origin/$BASE hit a conflict. Nothing was pushed." >&2
        echo "Resolve it here, then run this script again:" >&2
        echo "    git rebase origin/$BASE" >&2
        exit 1
    fi
    echo "rebased."

    # This script has just rebased the tree it lives in, so every line below
    # is being decided by a copy that no longer matches what is on disk. That
    # is how #50 merged a three-crate change without waiting for CI: the fix
    # that would have stopped it arrived in that run's own rebase, and the run
    # kept executing the copy bash already had open. It is worse than stale --
    # bash reads a script by byte offset as it goes, so a file rewritten
    # underneath it does not even reliably behave like the old version.
    #
    # So the run hands over: exec the machinery the rebase brought in, from
    # the top, with the same arguments. The commit is already made and the
    # tree is now zero behind, so the new run re-runs the gates against the
    # combination CI will actually see and skips straight past both -- which
    # is also the only way the gates and the merge decision can be talking
    # about the same tree. Nothing has been pushed at this point, so a
    # handover cannot double anything.
    #
    # The whole decision lives inside this `if`, which bash parsed before the
    # rebase touched anything: that is what makes it reachable at all.
    AFTER_SCRIPTS=$(git rev-parse "HEAD:scripts" 2>/dev/null || echo none)
    if [ "$BEFORE_SCRIPTS" != "$AFTER_SCRIPTS" ]; then
        DEPTH="${POSTIO_LAND_REEXEC_DEPTH:-0}"
        echo "the rebase brought in the landing machinery itself:"
        git diff --name-only "$BEFORE_HEAD" HEAD -- scripts/ | sed 's/^/    /'
        if [ "$DEPTH" -ge "$REEXEC_LIMIT" ] || [ ! -f "$TREE/scripts/issue-land.sh" ]; then
            echo >&2
            echo "Refusing to merge: after $DEPTH handover(s) the machinery is" >&2
            echo "still moving underneath this run, so it cannot say which copy" >&2
            echo "of itself would be deciding. Nothing was pushed. Run this" >&2
            echo "script again on the rebased tree -- #160." >&2
            exit 1
        fi
        echo "handing over to the landing machinery this rebase brought in (#160)."
        export POSTIO_LAND_REEXEC_DEPTH=$((DEPTH + 1))
        exec bash "$TREE/scripts/issue-land.sh" \
            ${ORIGINAL_ARGS[@]+"${ORIGINAL_ARGS[@]}"}
    fi

    echo "the gates above ran on the previous base -- CI is what checks the"
    echo "combination, which is why this waits for it."
fi

# Leased force, not a plain push. This script rebases onto origin/$BASE just
# above, so the *second* push of a branch already on the remote is necessarily
# non-fast-forward -- there is no non-forcing spelling of it, which is why
# this file's own header authorises --force-with-lease here.
#
# It was a plain push until #781, and that was invisible while CI was paused:
# a landing succeeded on its first attempt, pushed the branch once, and
# merged. A CI-gated landing fails, gets fixed, and lands again -- and every
# one of those second attempts died on "Updates were rejected because the tip
# of your current branch is behind its remote counterpart", after the gates
# had already run.
#
# Leased rather than bare: --force-with-lease compares against the
# remote-tracking ref this worktree last pushed, so it goes through for our
# own rebase and refuses if the remote moved for any other reason. A bare
# --force cannot tell those apart, and the guard hook refuses it everywhere
# for that reason.
#
# And leased only when there is something to lease *against*. The first push
# of a branch the remote has never seen is a create, and a create cannot
# clobber anything -- there is no counterpart to be behind. Asking for a lease
# there buys nothing and can cost a landing: `--force-with-lease` without an
# explicit <expect> also turns on `--force-if-includes`, which inspects the
# reflog, and #860 watched three green gate chains in a row die on
#
#     ! [rejected]  issue-411-... (stale info)
#
# for a branch that was not on the remote at all (`git ls-remote` empty, no
# remote-tracking ref). A plain `git push -u` created it immediately and the
# next run landed with no other change.
#
# The window between asking and pushing is safe: if somebody creates the
# branch in it, the plain push is rejected as non-fast-forward rather than
# overwriting them. It fails closed, which is the property the lease was for.
if git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
    git push -u --force-with-lease origin "$BRANCH"
else
    echo "--- pushing $BRANCH for the first time (no lease: nothing to clobber) ---"
    git push -u origin "$BRANCH"
fi

[ "$WIP" = 1 ] && { echo; echo "pushed $BRANCH without a PR (work in progress)."; exit 0; }

# Checked here, not at the top: everything above -- the commit guard, the
# gates, the push -- never touches `gh` at all (see
# scripts/tests/test-issue-land-commit-guard.py's own reasoning), so a
# script that refused a dirty tree or a red gate before this point had no
# reason to demand `gh` either. This is the last point before the first
# real `gh` call below.
source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"

# The *state*, not merely the existence, of a PR for this head branch.
# `gh pr view` resolves the most recent PR for the branch whatever state it is
# in, so a branch name that has been used before -- which
# `issue-claim.sh` makes likely, since it generates the name from the issue
# title and two sessions on one issue is the normal state of this repository --
# resolves to somebody else's *merged* PR. Adopting that as "already open"
# then merges nothing and reports success. #312.
PR_STATE=$(gh pr view --json state -q .state 2>/dev/null || echo "")
if [ "$PR_STATE" = "OPEN" ]; then
    echo "PR already open for $BRANCH; the push updated it."
    # It may have been opened before this worktree recorded a base -- or
    # against a different one entirely. Merging on that mismatch puts the work
    # on a branch nobody chose, so it stops here instead.
    OPEN_BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || echo "")
    if [ -n "$OPEN_BASE" ] && [ "$OPEN_BASE" != "$BASE" ]; then
        echo >&2
        echo "The open PR targets '$OPEN_BASE' and this worktree records" >&2
        echo "'$BASE'. Not merging: one of the two is wrong, and guessing" >&2
        echo "which would land the work somewhere nobody chose. Retarget the" >&2
        echo "PR, or correct the recorded base:" >&2
        echo "    printf '%s\\n' <branch> > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
        exit 2
    fi
else
    if [ -n "$PR_STATE" ]; then
        # A closed or merged PR on this head branch: the name was reused.
        # Opening a new one is correct -- the alternative is adopting a PR
        # that is not this work's.
        echo "the previous PR for $BRANCH is $PR_STATE; opening a new one."
    fi
    TITLE=$(git log -1 --format=%s)
    gh pr create --base "$BASE" --head "$BRANCH" --title "$TITLE" --body "$(cat <<BODY
$(git log "origin/$BASE..HEAD" --format='- %s')

Closes #$ISSUE
${VERIFY_NOTE:+
> [!WARNING]
> $VERIFY_NOTE}

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
fi
URL=$(gh pr view --json url -q .url)
echo "$URL"

# After `pr view` rather than as a `pr create --label`, so it applies to a PR
# that already existed too. Loud on failure: the entire point of the label is
# that the unverified gap is on the record, so silently not applying it is the
# one outcome worse than not trying.
if [ -n "$VERIFY_LABEL" ]; then
    if gh pr edit --add-label "$VERIFY_LABEL" >/dev/null 2>&1; then
        echo "labelled $VERIFY_LABEL"
    else
        echo "WARNING: could not apply $VERIFY_LABEL to $URL." >&2
        echo "         Create the label, or add it by hand -- this PR was not" >&2
        echo "         verified against $UNBUILDABLE." >&2
    fi
fi

# What this branch is actually landing, recorded before the merge: `--rebase`
# gives every commit a new hash, so "did it land" cannot be asked by ancestry
# afterwards. Subjects survive a rebase; that is what gets checked. See the
# verification below.
LANDING=$(git log "origin/$BASE..HEAD" --format=%s)

[ "$MERGE" = 1 ] || { echo "left open at your request (--no-merge)."; exit 0; }

# Hand the merge to GitHub and go (#1107). PR open -> merged was p50 10 min
# and p90 41 across 95 landings, all of it spent by the session that opened
# the PR sitting in wait-for-checks.sh. A ruleset now requires the CI
# checks, so `--auto` cannot merge before they are green -- which is the
# hazard the waiting existed for (#135, #139). Asked of the repository
# rather than assumed: a repository with auto-merge off (the self-tests'
# sandboxes, a fork) takes the watching path below, and `--wait` asks for
# it on purpose. Nobody waits in front of the PR, so a red check has to be
# found afterwards: the next claim lists the caller's red PRs, /steward
# sweeps them, and `issue-claim.sh --resume <n>` comes back to the branch.
# GitHub deletes the head branch on merge; until then it is the PR.
# The REST field, not `gh repo view --json`: that command has no field for
# auto-merge at all, and asking it for one is an error whose empty answer
# reads as "no" -- which is how the first landing after #1107 quietly took
# the watching path (#1136).
AUTO_MERGE_ALLOWED=$(gh api "repos/{owner}/{repo}" --jq .allow_auto_merge 2>/dev/null || true)
if [ "$WAIT" != 1 ] && [ "$AUTO_MERGE_ALLOWED" = "true" ]; then
    echo
    echo "--- auto-merge ---"
    gh pr merge --auto --rebase
    echo "auto-merge armed on $URL: GitHub merges it when the required checks pass."
    echo "Nothing waits here. If a check fails, your next claim will say so, and"
    echo "    scripts/issue-claim.sh --resume $ISSUE"
    echo "comes back to this branch to fix it on the same PR."
    echo "Now claim the next issue -- finishing an issue is not finishing a session."
    exit 0
fi

# Watch, do not fire and forget. GitHub's own --auto would merge immediately
# here: it waits for *required* checks, branch protection is what makes a check
# required, and this repository cannot set any (private repo, free plan). So
# auto-merge would land the PR before CI had started.
echo
echo "--- waiting for checks ---"
# Not inline, because this is the step that decides whether an unreviewed
# commit reaches main and it needs a test of its own -- see
# scripts/tests/test-wait-for-checks.py's stubbed `gh`. It exits non-zero when a
# check failed *or* when one was due and never registered; both mean the same
# thing here, which is do not merge.
if ! "$TREE/scripts/wait-for-checks.sh" "$URL"; then
    exit 1
fi

# Rebase, not squash. This history is linear and the project's convention is
# small focused commits; squashing a multi-commit branch throws away exactly
# the structure the commit rules exist to produce.
#
# Not --delete-branch: it deletes the *local* branch too, which makes `gh`
# switch this worktree off it first (to `main`, the PR's base) -- and `main`
# is permanently checked out in the shared checkout, so git refuses the
# checkout and `gh pr merge` reports failure even though the merge already
# went through on GitHub. #167. issue-release.sh already deletes the local
# branch once the worktree it belongs to is removed, so only the remote copy
# is left for this script to clean up, and that half never needed the local
# checkout at all.
gh pr merge --rebase

# Believe it only after checking. `gh pr merge` prints
# "! Pull request #N was already merged" and exits **0** when there is nothing
# to do, so the exit status alone once let this script announce "merged.",
# delete the remote branch, and leave the work existing nowhere but the local
# worktree -- which the line it prints next tells you to remove. #312.
#
# Ancestry cannot answer this: `--rebase` rewrites every commit, so the local
# tip is never an ancestor of the base even on complete success. Subjects
# survive, so they are what is compared, and `main` having moved on underneath
# is fine -- this asks whether the work arrived, not whether it is the tip.
#
# Asked repeatedly rather than once. `gh pr merge` returns as soon as GitHub
# *accepts* the merge, and the fetch below can still be answered before the
# new tip is visible -- which turned a few seconds of replication lag into
# "MERGE DID NOT LAND" twice in three landings (#406), telling the session to
# open a second PR for commits that were already on main. The state this
# guards against (#312) is permanent, not late, so waiting a little costs it
# nothing at all.
#
# #406's own fix picked 30s and it was not enough: PR #417, a ten-commit
# rebase merge, recurred the same false alarm because replication of a
# bigger rebase just takes longer. 120s is the floor -- a defensible
# starting point on its own -- and it scales up from there with the number
# of commits landing, since that is what #418 found correlates.
LANDING_COUNT=$(printf '%s\n' "$LANDING" | grep -c .)
LANDED_TIMEOUT_DEFAULT=$(( 60 + 10 * LANDING_COUNT ))
[ "$LANDED_TIMEOUT_DEFAULT" -ge 120 ] || LANDED_TIMEOUT_DEFAULT=120
LANDED_TIMEOUT="${POSTIO_LANDED_TIMEOUT:-$LANDED_TIMEOUT_DEFAULT}"
LANDED_POLL="${POSTIO_LANDED_POLL:-3}"
LANDED_DEADLINE=$(( $(date +%s) + LANDED_TIMEOUT ))

# The subjects of $LANDING that $1 does not carry, one per line.
missing_from() {
    while IFS= read -r subject; do
        [ -n "$subject" ] || continue
        printf '%s\n' "$1" | grep -Fqx -- "$subject" || printf '%s\n' "$subject"
    done <<EOF_LANDING
$LANDING
EOF_LANDING
}

while :; do
    git fetch -q origin "$BASE"
    MISSING=$(missing_from "$(git log "origin/$BASE" --format=%s)")
    [ -n "$MISSING" ] || break
    [ "$(date +%s)" -lt "$LANDED_DEADLINE" ] || break
    sleep "$LANDED_POLL"
done

# Out of patience is not out of options. The deadline above is a guess at how
# long replication takes; the PR's own state is a fact about whether the merge
# happened. MERGED means GitHub completed it and the only thing outstanding is
# the new tip becoming visible here -- so keep waiting rather than hand back a
# failure for work that is already on the base branch. #749's landing spent
# its 120s, reported "MERGE DID NOT LAND", and the commits were there a minute
# later; the session then had to verify by hand what this loop existed to
# answer.
#
# This does not weaken #312's guard, which is the case this whole block is
# for: success is still only ever declared by *seeing the subjects arrive*.
# A PR that says MERGED while its commits never appear still fails, just
# later and with the extra waiting spent on the one state where waiting is
# known to be the right move.
if [ -n "$MISSING" ]; then
    STATE=$(gh pr view --json state --jq .state 2>/dev/null || echo "unknown")
    if [ "$STATE" = "MERGED" ]; then
        echo
        echo "GitHub says the PR is MERGED but origin/$BASE has not shown the"
        echo "commits yet after ${LANDED_TIMEOUT}s. That is replication lag, not a"
        echo "failed merge, so this waits rather than giving up."
        # 300s, not longer: the lag this exists for resolves in seconds to a
        # couple of minutes, while the state it must not hide -- #312's merge
        # that put nothing anywhere -- is permanent, and making a session wait
        # ten minutes to be told so is its own cost. The branch is preserved
        # either way, so the price of the wait is patience, not safety.
        MERGED_DEADLINE=$(( $(date +%s) + ${POSTIO_MERGED_TIMEOUT:-300} ))
        while [ -n "$MISSING" ]; do
            [ "$(date +%s)" -lt "$MERGED_DEADLINE" ] || break
            sleep "$LANDED_POLL"
            git fetch -q origin "$BASE"
            MISSING=$(missing_from "$(git log "origin/$BASE" --format=%s)")
        done
        [ -n "$MISSING" ] || echo "the commits are on origin/$BASE now."

        # Still nothing? Then stop asking this clone a question the remote can
        # answer. Four landings on 2026-09-02 (#818, #830, #833) failed here
        # for work that was on `main`: the subject matching was verified
        # correct by hand against the real commits, and `git reflog` showed
        # origin/main moving to the merge commit one second after the PR's own
        # mergedAt -- so neither the comparison nor replication explains it,
        # and raising the timeout from 120s to 300s did not help. Rather than
        # guess at a longer deadline, ask the side that cannot be stale.
        #
        # This is a STRONGER guarantee than the subject match, not a weaker
        # one, which is what makes it safe against #312: it identifies the
        # merge by SHA rather than by a string that a reworded commit could
        # duplicate, and it asks specifically whether the base branch contains
        # that SHA. A `gh pr merge` that put nothing anywhere still fails,
        # because the compare will not say so.
        if [ -n "$MISSING" ]; then
            MERGE_SHA=$(gh pr view --json mergeCommit --jq '.mergeCommit.oid // empty' 2>/dev/null || true)
            if [ -n "$MERGE_SHA" ]; then
                # "identical" when the merge IS the tip, "ahead" once other
                # work has landed on top of it. Anything else -- including the
                # empty string from a failed call -- is not a confirmation,
                # and falls through to the failure below.
                # `{owner}/{repo}` is gh's own placeholder for the repository
                # this working directory belongs to, so there is no remote URL
                # to parse and no second place for it to be wrong.
                CONTAINMENT=$(gh api "repos/{owner}/{repo}/compare/$MERGE_SHA...$BASE" --jq '.status // empty' 2>/dev/null || true)
                case "$CONTAINMENT" in
                identical | ahead)
                    echo
                    echo "origin/$BASE still does not show the commits here, but GitHub"
                    echo "reports $BASE contains the merge commit $(printf '%.7s' "$MERGE_SHA")"
                    echo "(compare: $CONTAINMENT). The work landed; this clone is behind."
                    MISSING=
                    ;;
                esac
            fi
        fi
    fi
fi

if [ -n "$MISSING" ]; then
    printf '%s\n' "$MISSING" | while IFS= read -r subject; do
        echo "not on origin/$BASE: $subject" >&2
    done
    echo >&2
    echo "MERGE DID NOT LAND. gh reported success and origin/$BASE does not" >&2
    echo "carry the commits above. The PR itself says:" >&2
    echo "    state: $STATE" >&2
    if [ "$STATE" = "MERGED" ]; then
        echo "which disagrees with $BASE, and waiting the extra" >&2
        echo "${POSTIO_MERGED_TIMEOUT:-300}s did not resolve it -- so this is no longer" >&2
        echo "the replication lag that wait is for. Fetch again before doing" >&2
        echo "anything; if the commits are there under different hashes the" >&2
        echo "work landed and release the worktree as usual. If they are not," >&2
        echo "the PR merged something other than this branch -- check what it" >&2
        echo "points at before re-landing." >&2
        exit 1
    fi
    echo "The remote branch $BRANCH is deliberately NOT deleted -- with the PR" >&2
    echo "merged elsewhere or closed, it may be the only copy of this work" >&2
    echo "besides this worktree." >&2
    echo >&2
    echo "Do not run issue-release.sh. Check the PR, then land onto a branch" >&2
    echo "name that is not already spoken for:" >&2
    echo "    git branch -m <a-new-name> && scripts/issue-land.sh" >&2
    exit 1
fi

echo
echo "merged."
if git push origin --delete "$BRANCH" 2>&1; then
    echo "remote branch deleted."
else
    echo "warning: could not delete the remote branch $BRANCH -- it may" >&2
    echo "already be gone. Not fatal: the merge above already succeeded." >&2
fi
echo "Next: scripts/issue-claim.sh   (from here: reuses this worktree, build and all)"
echo "      scripts/issue-release.sh $ISSUE   only if you are stopping."
echo "Finishing an issue is not finishing a session."
