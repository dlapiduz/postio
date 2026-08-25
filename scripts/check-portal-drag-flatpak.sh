#!/usr/bin/env bash
# Run the portal drag test *inside* the Flatpak sandbox.
#
# `crates/postio-app/tests/drag_out_portal.rs` proves the round trip a
# receiving application drives: serialise a drag to
# `application/vnd.portal.filetransfer`, take the transfer key, call
# RetrieveFiles, read the bytes back. On the host that proves the mechanism.
# Inside the sandbox it proves the thing #121 is actually about -- there,
# `application/vnd.portal.filetransfer` is the only spelling that carries
# files out at all, and a bare `file://` URI points at a path the receiver
# cannot open.
#
# The test is the same either way. This script is only the plumbing that gets
# it running in there:
#
#   1. generate the offline cargo sources the sandboxed build needs
#   2. build a *check* manifest: the real one, plus commands that build the
#      test binary and install it to /app/libexec
#   3. run it under `flatpak run`, on a compositor of its own
#
# Step 2 is derived from `flatpak/dev.postio.Postio.json` with `jq` rather
# than committed as a second manifest, and deliberately: two manifests drift,
# and the one that drifts is always the one nobody releases. The shipped
# manifest is not modified.
#
# Usage:
#   scripts/check-portal-drag-flatpak.sh            # build if needed, then run
#   scripts/check-portal-drag-flatpak.sh --rebuild  # force a fresh build
#   scripts/check-portal-drag-flatpak.sh --watch     # on the real display
#
# Exit status: 0 the sandboxed drag hands over readable files, non-zero
# otherwise. It cannot pass by skipping -- POSTIO_REQUIRE_PORTAL=1 turns this
# test's own skip paths into failures, because a sandbox check that quietly
# checked nothing is the exact failure the test exists to catch, one level up.
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
cd "$TREE"

APP_ID=dev.postio.Postio
MANIFEST=flatpak/dev.postio.Postio.json
# Beside the real manifest, not under target/, and that is not cosmetic: a
# manifest's `sources` paths -- `cargo-sources.json`, and the `dir` source
# pointing at `..` -- resolve relative to the directory the manifest is in.
# Anywhere else and flatpak-builder looks for the repository one level above
# `target/`. Gitignored; see .gitignore.
CHECK_MANIFEST=$TREE/flatpak/${APP_ID}.Check.json
BUILD_DIR=$TREE/target/flatpak-check/build
STATE_DIR=$TREE/target/flatpak-check/state
CHECK_BIN=/app/libexec/postio-drag-portal-check

REBUILD=0
WATCH=0
for argument in "$@"; do
    case "$argument" in
        --rebuild) REBUILD=1 ;;
        --watch)   WATCH=1 ;;
        *) echo "unknown argument: $argument" >&2; exit 2 ;;
    esac
done

missing() {
    echo "missing: $1" >&2
    echo "  $2" >&2
    exit 2
}
command -v flatpak >/dev/null      || missing flatpak "sudo dnf install flatpak"
command -v flatpak-builder >/dev/null || missing flatpak-builder "sudo dnf install flatpak-builder"
command -v jq >/dev/null           || missing jq "sudo dnf install jq"

RUNTIME_VERSION=$(jq -r '."runtime-version"' "$MANIFEST")
SDK=$(jq -r '.sdk' "$MANIFEST")
if ! flatpak info "$SDK//$RUNTIME_VERSION" >/dev/null 2>&1; then
    missing "$SDK//$RUNTIME_VERSION" \
        "flatpak install --user -y flathub $SDK//$RUNTIME_VERSION"
fi

# ── 1. the offline cargo sources ────────────────────────────────────────
# Generated, never committed: a stale copy silently builds something other
# than what Cargo.lock resolves, which is worse than not having one.
if [ "$REBUILD" = 1 ] || [ ! -s flatpak/cargo-sources.json ] \
   || [ Cargo.lock -nt flatpak/cargo-sources.json ]; then
    echo "--- generating offline cargo sources ---"
    python3 flatpak/flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
    echo "$(jq length flatpak/cargo-sources.json) crate sources pinned"
fi

# ── 2. the check manifest ───────────────────────────────────────────────
mkdir -p "$(dirname "$CHECK_MANIFEST")"
# `cargo test --no-run` names the binary with a hash, so the install line
# picks the newest matching one rather than guessing it. `--message-format`
# would be tidier and needs a JSON parser inside the build sandbox.
jq --arg bin "$CHECK_BIN" '
    .["finish-args"] += ["--filesystem=xdg-cache/postio-portal-check:create"]
  | .modules[0]["build-commands"] += [
      "cargo --offline test --release --package postio-app --test drag_out_portal --no-run",
      "install -Dm755 $(ls -t target/release/deps/drag_out_portal-* | grep -v \"\\\\.d$\" | head -1) " + $bin
    ]
' "$MANIFEST" > "$CHECK_MANIFEST"

if [ "$REBUILD" = 1 ] || [ ! -d "$BUILD_DIR" ]; then
    echo "--- building $APP_ID with the check binary ---"
    echo "    (a --release build of the whole workspace inside the sandbox;"
    echo "     the first one is slow and nothing else should be building)"
    flatpak-builder --user --force-clean --disable-rofiles-fuse \
        --state-dir "$STATE_DIR" --install \
        "$BUILD_DIR" "$CHECK_MANIFEST"
fi

# ── 3. run it, on a compositor of its own ───────────────────────────────
# The sandboxed test presents a real window. Without this it lands on
# whoever is at the keyboard, which is the thing scripts/test-headless.sh
# exists to stop -- and the socket lives in XDG_RUNTIME_DIR, which flatpak
# already binds into the sandbox, so pointing WAYLAND_DISPLAY at it is the
# whole trick.
RUN_ENV=(--env=POSTIO_REQUIRE_PORTAL=1 --env=RUST_BACKTRACE=1)
if [ "$WATCH" = 0 ]; then
    DISPLAY_NAME="${POSTIO_TEST_DISPLAY:-postio-headless}"
    # It starts the compositor on demand and leaves it running, so wrapping
    # `true` is how you ask for one without running anything on it.
    scripts/test-headless.sh true >/dev/null 2>&1 || true
    if [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/$DISPLAY_NAME" ]; then
        # GDK_BACKEND pins Wayland for the same reason test-headless.sh pins
        # it: a stray DISPLAY would pull the window back onto the real
        # session through XWayland while looking like it had worked.
        # `--env=WAYLAND_DISPLAY=...` alone is not enough, and the reason is
        # worth writing down: `--socket=wayland` binds the socket named by
        # the *host's* WAYLAND_DISPLAY into the sandbox. Naming a different
        # one only inside the sandbox points at a socket flatpak never bound,
        # and the test then reports "no display" from within a working
        # compositor. So the host environment of `flatpak run` is what has to
        # name the headless display; the --env is what the app then reads.
        HOST_WAYLAND_DISPLAY="$DISPLAY_NAME"
        RUN_ENV+=(--env=GDK_BACKEND=wayland --env=GTK_A11Y=none)
        echo "--- running on the headless compositor ($DISPLAY_NAME) ---"
    else
        echo "--- no headless compositor; running on the live display ---" >&2
    fi
else
    echo "--- running on the live display ---"
fi

echo
# HOST_WAYLAND_DISPLAY, not just --env: see the note above.
WAYLAND_DISPLAY="${HOST_WAYLAND_DISPLAY:-${WAYLAND_DISPLAY:-}}" \
    flatpak run "${RUN_ENV[@]}" --command="$CHECK_BIN" "$APP_ID" \
    --nocapture --test-threads=1
