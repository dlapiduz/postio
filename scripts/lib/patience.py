"""One dial for the self-tests' deadlines, and a timeout that says it was one.

# Why this exists

`postio_test_support::patience` has multiplied every deadline in the Rust
suite by `POSTIO_TEST_PATIENCE` since #842, and
`scripts/checks/check-test-deadlines-scale.py` has refused a Rust deadline
that cannot hear it since #957. The argument in both is the same: a wait long
enough on an idle workstation is not long enough on a loaded runner, and with
the deadline written into every test the only available fix is to enlarge the
one copy that failed -- which slows the suite for everyone, permanently, and
does nothing for the next runner that is busier.

**Both stop at the crate boundary.** The 81 self-tests under `scripts/tests/`
shell out to the scripts they cover with deadlines written by hand, so the one
dial anybody knows about moves none of them. #1243 is what that cost:
`test-sccache-restart.py` gave a stubbed shell script 30 seconds, which is
generous on an idle box and not obviously generous on the runner that builds
81 sandboxes four at a time. It expired once, on a branch that touched nothing
to do with sccache.

# The other half, which cost more

That expiry surfaced as a plain `FAILED`. A `subprocess.TimeoutExpired` and a
script returning the wrong answer looked identical, so establishing that the
branch was innocent meant reading it, re-running it, and checking main either
side. [`run`] raises [`ScriptHung`] instead, which says which of the two
happened and names the dial to turn.

# Where this lives, and why not beside the tests

`scripts/lib`, not `scripts/tests`, because CI runs `ls scripts/tests/*.py`
and executes every file it finds as a self-test -- discovered rather than
listed, deliberately, since a hand-maintained enumeration had already drifted
by nine tests. A helper module in that directory would be *run* as a test.

# Use

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))
    import patience

    result = patience.run([...], capture_output=True, text=True, timeout=30)

`timeout` stays the number that is right on an idle machine. The dial is what
makes it right on a busy one.
"""

from __future__ import annotations

import os
import subprocess
from typing import Any

#: The environment variable, spelled exactly as the Rust suite spells it.
#:
#: One name, because two dials meaning the same thing is one dial nobody sets.
PATIENCE_ENV = "POSTIO_TEST_PATIENCE"

#: What a self-test gets if it names no deadline of its own.
#:
#: Long, on purpose. These deadlines guard against a hang, not against slowness
#: -- no self-test asserts anything about how long its subject takes -- so the
#: only cost of a generous one is how long a genuine hang blocks the job, and
#: the cost of a tight one is #1243.
DEFAULT_TIMEOUT = 120.0


class ScriptHung(Exception):
    """A subprocess ran out of wall clock rather than misbehaving.

    Its own type because the two findings are not the same and used to look
    identical. Raised by [`run`] in place of `subprocess.TimeoutExpired`.
    """


def patience() -> float:
    """The multiplier `POSTIO_TEST_PATIENCE` asks for, or 1.

    An unparseable, empty or non-positive value is ignored rather than
    honoured, matching `postio_test_support::patience_from`: a typo in a
    workflow should not quietly set every deadline in the suite to zero and
    turn every wait into an instant failure.
    """
    raw = os.environ.get(PATIENCE_ENV)
    if not raw:
        return 1.0
    try:
        value = float(raw)
    except ValueError:
        return 1.0
    return value if value > 0 else 1.0


def deadline(base: float = DEFAULT_TIMEOUT) -> float:
    """`base` seconds, as patient as this machine has been told to be."""
    return base * patience()


def run(
    args: Any,
    *,
    timeout: float = DEFAULT_TIMEOUT,
    **kwargs: Any,
) -> subprocess.CompletedProcess:
    """`subprocess.run`, with the deadline scaled and expiry reported as itself.

    Every other argument is passed straight through, so this is a drop-in
    wherever a self-test already ran a subprocess with a timeout.

    Raises:
        ScriptHung: the command did not finish within the scaled deadline.
    """
    limit = deadline(timeout)
    try:
        return subprocess.run(args, timeout=limit, **kwargs)
    except subprocess.TimeoutExpired as expired:
        shown = args if isinstance(args, str) else " ".join(str(part) for part in args)
        raise ScriptHung(
            f"`{shown}` did not finish within {limit:g}s "
            f"({patience():g}x a {timeout:g}s base). That is a hang or a "
            f"machine too loaded to finish the work, not the command "
            f"answering wrongly -- raise {PATIENCE_ENV} if it is the latter."
        ) from expired
