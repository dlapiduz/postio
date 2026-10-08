# Triage labels

| Role (mattpocock/skills) | Label in this repo | Meaning here |
|---|---|---|
| `needs-triage` | _(no label)_ | An open issue without `ready` has not been triaged. |
| `needs-info` | `question` | Waiting on the reporter for more information. |
| `ready-for-agent` | `ready` | Triaged and unblocked: an agent may claim it. Always paired with a priority, `p0` (highest) to `p4`. |
| `ready-for-human` | `needs-maintainer` | Only the maintainer can decide it; comment the question and the options. |
| `wontfix` | `wontfix` | Will not be actioned. |

Beside the five roles:

- `needs-architecture` is a design or architecture call an agent can make
  (`/ux-architect`'s queue), not a question for the maintainer.
- `epic`, `icebox`, `needs-architecture` and `needs-maintainer` are never claimable.
- `in-progress` is set by `scripts/issue-claim.sh`.
