# A test keyring that was quietly the login keychain (2026-09-06, #1279)

Adding an account across the FFI needs a keyring, so the new tests handed the
session a `MemorySecretStore` — the in-memory double the whole workspace uses
— and asserted a password was stored. They passed when run with a filter, and
**hung the whole binary** when the suite ran: 143 tests reported `ok`, the
summary line never appeared, and `sample` showed the main thread waiting for
a `CompletedTest` that never came.

## What it actually was

`Session::open` reads `options.secrets` on the real path and **not on the
in-memory one**. The in-memory branch built its `Wiring` without
`.with_secrets(...)`, so a test that had carefully supplied a fake keyring
got `platform_keyring()` — the developer's login keychain.

On macOS that is not merely wrong, it is *interactive*. An unsigned test
binary reading back an item it wrote raises a permission prompt, and a prompt
in a process with no GUI is a thread that never returns. Filtered runs passed
because the item did not exist yet: creating one does not prompt, reading one
does.

It also means the first run wrote a password into a real keychain. It was
deleted by hand:

```bash
security delete-generic-password -a "ada@ostwald.invalid"
```

## What to take from it

* **A silently ignored builder is worse than a missing one.** `with_secrets`
  compiled, read well, and did nothing on the path every test uses.
* **All tests pass and the process does not exit** means a test thread is
  gone or blocked, not that the assertions are wrong. `sample <pid>` and
  comparing `--list` against the reported names finds it in two minutes:
  the tests that never printed are the ones that hung.
* On macOS, anything reaching the platform keyring in a test is a potential
  modal prompt. If a test needs a keyring it must be given one, and the code
  under test has to actually use what it was given.
