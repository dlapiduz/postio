# Contract: spec 014's held tasks, and where this spec finishes them

Spec 014 (`specs/014-compose-formatting/tasks.md` on `feature/compose-formatting`,
PR #2199) holds T054 to T069 for ADR 0039. Each is done here, when its own
statement in that file is true on the native surface. The commit that
finishes one removes its `⛔ ADR 0039` marker from spec 014's `tasks.md`
and ticks it there.

Spec 014 named its target `crates/postio-widgets/src/composer/native/`. This
spec places the surface at that path (plan, Project Structure).

| 014 task | What it is | Done in this spec by | Needs |
|---|---|---|---|
| T054 | Register the Held registry rows, `Context::TablePicker` | T061 | 014 Part A merged |
| T055 | Picker frame keys `picker_choose_0` to `9` | T062 | T061 |
| T056 | Edit ops as `apply` arms (R6) | T007, T008 (main's types: inline code, indent, outdent) and T072 (014's types: tables, tasks, list start, code language) | Phase 2; 014 Part A |
| T057, T058 | Inline typing; strikethrough and inline code drawn | T063, T064 | Phase 4 |
| T059, T060 | Block typing, Tab and Shift+Tab, `line_break`, drawing of each block | T065, T066 | Phase 4 |
| T061 | Code language picker's words and picker | T067 | T062 |
| T062, T063, T064 | The formatting picker, its suite, the toolbar buttons, the Mac picker | T068, T069, T070 | T062; the Mac's: Phase 6 |
| T065, T066 | Tables: sequence, table picker, cell navigation, grid | T071, T072 | T062 |
| T067 | `paste_markdown` on the native surface | T073 | Phase 4 |
| T068 | Remote image chip drawn inline | T074 | Phase 4 |
| T069 | No FR-029 refusal remains | T075 | all above |

The 014 spec's own acceptance is the arbiter. If this spec's task and 014's
differ, 014's statement wins and this table is corrected.
