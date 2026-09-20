# Dev-profile debug info is `line-tables-only`

*Archived 2026-09-14: the workspace is `[profile.dev] debug = 0` now — [`debug = "line-tables-only"` was still most of the binary](../2026-09-03-debug-line-tables-only-was-still-most-of-the-binary.md) measured it and took it out.*

(Workspace `Cargo.toml`.)
Backtraces keep file:line — what tests and `RUST_BACKTRACE` need — while the
heaviest part of compiling and linking the GTK/WebKit stack goes away. What
is lost is variable inspection in a debugger; delete the one line to get it
back. Changing it invalidates every cached compile once (sccache keys on
flags), so the first build after it lands pays full price.
