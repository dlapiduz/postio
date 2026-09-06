# Moving code out of a crate you cannot compile (2026-09-06, #1221)

Ten blocks of toolkit-free logic came out of `postio-gtk` during the macOS
port. Three of those moves shipped a red pull request, every time for the same
reason and never for the interesting one: **imports left behind**.

`postio-gtk` needs gtk4, libadwaita and webkitgtk. A Mac has none of them, so
`cargo clippy -p postio-gtk` cannot run here at all — and an orphaned `use` is
invisible until a Linux runner says `unused imports`. The move itself is
mechanical and always worked; the leftovers cost three round trips of about
twenty minutes each.

## What the check actually is

After moving a function out, for every symbol its `use` lines named:

1. Count the remaining occurrences in the file.
2. **Split them at `mod tests`.** This is the part that is easy to get wrong.
   Twice the symbol was still used — only from the test module — so deleting
   the import broke the tests, and keeping it at the top level left the *lib*
   target with an unused one. The answer is to move that import into
   `mod tests`, not to keep or delete it.
3. Watch for a duplicate: the test module often already imports what you are
   about to add, through its own `use` rather than through `use super::*`.

```bash
T=$(grep -n "^mod tests {" <file> | tail -1 | cut -d: -f1)   # the *last* one
grep -n "\bSymbol\b" <file> | awk -F: -v t="$T" '$1<t'       # lib uses
grep -n "\bSymbol\b" <file> | awk -F: -v t="$T" '$1>t'       # test uses
```

The `tail -1` matters: `row.rs` has three `#[cfg(test)]` blocks and the first
is a hundred lines in, so keying on it splits the file in the wrong place and
reports every lib use as a test use.

## Why not a check script

`scripts/checks/` cannot compile the crate either, and a textual
"is this symbol mentioned again" heuristic is exactly what rustc already does
properly. This is a habit, not an invariant: the compiler on the other
platform *is* the check, and the only thing worth changing is doing the grep
before the push rather than after the red.

Related: `docs/notes/2026-09-05-the-gate-that-runs-cannot-see-the-platform-that-does-not.md`,
which is the same asymmetry pointing the other way.
