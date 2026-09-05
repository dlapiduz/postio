# Which session to run

Every role is a skill now. Start a fresh Claude session in this repository
and type the skill's name; `CLAUDE.md` loads on its own, and the skill
carries the rest. The prompts that used to live here drifted from the skills
they restated — one of them contradicted `CLAUDE.md` on how to prove a test
red — so the text lives beside the loop it describes and is reviewed with it.

Five roles, split by **authority, not seniority**:

| Say | Role | Decides |
|---|---|---|
| `/issue` | developer | how a decided thing becomes working code |
| `/initiative` | developer, several interdependent issues on one feature branch | the branch shape, once, before cutting a worktree |
| `/ux-architect` | architect | the thing itself — the `needs-architecture` queue |
| `/product-manager` | product manager, on a loop | which things, in what order, what makes a release |
| `/steward` | maintainer's right hand, on a timer | whether work is real: main green, claims live, closed issues reachable |

Developers get `--label opus` or `--label sonnet` on their claims; the pool
is split by model, and "nothing ready" for one can sit beside a dozen issues
for the other.

## What is blocked right now

```bash
gh issue list --label ready --state open --json number --jq 'length'                # developer work
gh issue list --label ready --state open --limit 200 --json labels \
  --jq '[.[]|select([.labels[].name]|index("opus"))]|length'                        # ...opus-sized
gh issue list --label needs-architecture --state open --json number --jq 'length'   # decisions waiting
gh issue list --label needs-maintainer --state open --json number --jq 'length'     # yours
gh issue list --state open --limit 200 --json milestone \
  --jq '[.[]|select(.milestone==null)]|length'                                      # in no release
```

Run an architect when `needs-architecture` is deep, or when developers keep
stopping on the same undecided question. Run the product manager on a loop,
or whenever the backlog has grown faster than anyone has read it. Run the
steward every couple of hours. Run developers otherwise.

All of them can run at once — the worktrees keep them out of each other's
way. Two things to avoid: two product managers, since the whole point is a
single coherent view, and a steward that starts taking issues, since then
nobody is watching.
