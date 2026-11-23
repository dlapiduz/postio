# Quickstart: seeing each slice work

How to see each slice working, and which test proves it. The contracts hold
the details. Nothing here touches the network or the real `~/.gnupg`.

## Prerequisites

- A worktree on `feature/message-authenticity`, seeded
  (`scripts/worktree-seed.sh`).
- The corpus fixtures that `tasks.md` adds through `/add-fixture`.
- For the app: `scripts/run-isolated.sh` on the branch's commit, which opens
  the demo store.

## Slice A: the Source surface

```bash
scripts/test-fast.sh                                         # postio-ui authenticity and source units
cargo nextest run -p postio-host --test host_suite source_view
cargo nextest run -p postio-gtk --test focus_suite view_source
scripts/storyboards.sh reader/source-surface                 # red on main, green here
```

In the app, open a message from the demo store that carries
`Authentication-Results`, then:

1. press `V`. Verification lists the provider's results in words;
2. press `2`. The Headers tab shows the boundary line under the provider's
   header;
3. press `3`. Large parts are folded;
4. press `Esc`. You are back on the same row.

## Slice B: OpenPGP and the mark

```bash
cargo test -p postio-signature                               # policy, scope, sweeps
cargo nextest run -p postio-session --test session_suite signature
cargo nextest run -p postio-gtk --test focus_suite signed_mark
```

In the app:

1. a signed fixture shows **Signed** after the sender;
2. the tampered copy shows the warning line, and no mark;
3. press the mark, then "Accept this key". The mark reads **Signed ·
   accepted**;
4. `z` undoes the acceptance.

## Slice B2: the GnuPG keyring

```bash
cargo nextest run -p postio-host --test host_suite gnupg_keyring
```

The test builds a temporary `GNUPGHOME` for each format. In the app, point
`GNUPGHOME` at a scratch directory holding a test key (`run-isolated.sh`
passes the environment through), and turn on Settings → Privacy → "Use keys
from my GnuPG keyring". Settings shows the count read, and a message signed
by that key shows **Signed**, with "From your GnuPG keyring" on its tab.

## Slice C: S/MIME (only after S2 passes)

```bash
cargo test -p postio-signature smime
```

An S/MIME fixture that chains to the test anchor shows **Signed** and
"Revocation not checked".

## The privacy proof, every slice

```bash
cargo nextest run -p postio-host --test host_suite source_surface_makes_no_connection
python3 scripts/checks/check-no-silent-tracking.py
python3 scripts/checks/check-crate-boundaries.py
```
