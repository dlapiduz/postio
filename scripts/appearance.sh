#!/usr/bin/env bash
#
# Look at the composer, and check the claims a test cannot.
#
# Every WebKit test in this repository runs on the software rendering path
# (`headless-runner.sh` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, the mitigation
# for #272), so a green suite says nothing about how the editor paints. That is
# #1307, and it is why `specs/002-compose-editor`'s T048 is "look at it" rather
# than a test.
#
# This is the looking, made repeatable. It renders the surfaces through the
# *real* paths — a row is clicked, the reader loads from the blob store, `e`
# opens a reply, and the quote is built by `quote_of` — and then reads the
# pixels back, so the answers are numbers rather than impressions.
#
#   scripts/appearance.sh              # render and check, into target/appearance
#   scripts/appearance.sh /tmp/look    # somewhere else
#
# Deliberately **not** a test and not in `check.sh`. It needs a compositor and
# several seconds per shot, and the half that matters most is a person opening
# the PNGs. What it automates is the half that can be: whether the colours are
# the ones the tokens name.
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-target/appearance}"
mkdir -p "$OUT"

command -v magick >/dev/null || {
    echo "appearance: needs ImageMagick for the colour checks (dnf install ImageMagick)" >&2
    exit 1
}

shot() {
    local name="$1"; shift
    cargo run --quiet -p postio-app --example shot -- "$OUT/$name.png" "$@" 2>/dev/null \
        | grep -E '^shot:' || true
}

# The colour at a point, as `r,g,b`.
at() { magick "$1" -format "%[pixel:p{$2,$3}]" info: | sed 's/.*(\([^)]*\)).*/\1/' | cut -d, -f1-3; }

# The value of a `--r-*` token in one scheme, as `r,g,b`.
token() {
    local name="$1" scheme="$2" css=crates/postio-ui/data/reader-tokens.css hex
    if [ "$scheme" = dark ]; then
        hex=$(sed -n '/prefers-color-scheme: dark/,$p' "$css" | grep -m1 -- "--$name:" )
    else
        hex=$(sed -n '1,/prefers-color-scheme: dark/p' "$css" | grep -m1 -- "--$name:")
    fi
    hex=$(printf '%s' "$hex" | sed 's/.*: *//; s/;.*//')
    magick -size 1x1 "xc:$hex" -format "%[pixel:p{0,0}]" info: | sed 's/.*(\([^)]*\)).*/\1/' | cut -d, -f1-3
}

problems=0
same() {
    local what="$1" got="$2" want="$3"
    if [ "$got" = "$want" ]; then
        printf '  ok    %-46s %s\n' "$what" "$got"
    else
        printf '  WRONG %-46s %s (expected %s)\n' "$what" "$got" "$want"
        problems=$((problems + 1))
    fi
}

echo "rendering, through the paths a person drives —"
shot reply-light  demo reply
shot reply-dark   demo reply dark
shot reader-light demo open
shot reader-dark  demo open dark

echo
echo "the composer's editing surface is the reader's ground (FR-073, FR-074) —"
for scheme in light dark; do
    # Inside the editing surface, and inside the reader's message body: both
    # well clear of their chrome at the shot's default 1120x700.
    editor=$(at "$OUT/reply-$scheme.png" 900 520)
    reader=$(at "$OUT/reader-$scheme.png" 900 380)
    ground=$(token r-ground "$scheme")
    same "$scheme: editing surface is --r-ground" "$editor" "$ground"
    same "$scheme: reader body is --r-ground"     "$reader" "$ground"
    same "$scheme: and they agree"                "$editor" "$reader"
done

# The control, and it is not decoration. Dark `--r-ground` and the chrome
# beside it differ by one unit, so "the editor is the ground" is a strict test
# and a nearly invisible one -- which means a sample landing somewhere wrong,
# or a stale PNG from a shot that failed, would sail through it. What cannot
# sail through: the same point reading the same colour in both schemes. That is
# also the failure FR-074 is actually about, since a `WebView` resolves
# `prefers-color-scheme` from its own settings and not from libadwaita.
echo
echo "and it follows the scheme rather than ignoring it —"
light=$(at "$OUT/reply-light.png" 900 520)
dark=$(at "$OUT/reply-dark.png" 900 520)
if [ "$light" = "$dark" ]; then
    printf '  WRONG %-46s %s\n' "the surface is the same in both schemes" "$light"
    echo "        (either the scheme is not reaching the WebView, or these"
    echo "         PNGs are stale and the checks above proved nothing)"
    problems=$((problems + 1))
else
    printf '  ok    %-46s %s -> %s\n' "light and dark differ" "$light" "$dark"
fi

echo
echo "PNGs in $OUT — open them. The rest of T048 is a person:"
echo "  the quote opens folded, and opens when clicked"
echo "  the caret sits above it, and typing goes where you look"
echo "  a scheme change keeps the caret and the undo history"
echo
if [ "$problems" -gt 0 ]; then
    echo "$problems colour(s) not what the tokens name."
    exit 1
fi
echo "colours match the tokens."
