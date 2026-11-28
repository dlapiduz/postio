# ADR 0012 — Adding a second account, and orienting a first-time user

- **Status:** Accepted (2026-08-24). The orientation plate (Q4–Q6) was removed with the classic app: the desktop app teaches each key inside the control it runs.
- **Decision:** the onboarding form stays one widget with two hosts. The real work is in the composition root: an account can join a running application without a restart.

## Rules
- The onboarding form (`postio_widgets::present::onboarding`) is a plain widget with no opinion about where it lives. First run presents it as the window's content; add-account presents it in a dialog over the running shell. The widget is never forked into a second form.
- Add-account is a registry command, `account.add`, in the palette and cheat sheet.
- Attaching an account is one function, `attach_account`, used at startup and at runtime: create the row and keyring entry, start an engine, register feeds, emit events. Startup and "the app gained one" are the same path. It depends on [ADR 0005](0005-multiple-accounts.md) (engine per account, `Scope`).
- The probe runs on commit, never per keystroke. Closing the dialog cancels the probe's `CancelToken` (`postio_account::cancel`); a callback after its screen is gone must not write into a reopened dialog. ADR 0006 reuses the same token.
- The view layer links neither `io-imap` nor the store engine; the probe, account write, keyring entry and engine start live below it (`check-crate-boundaries.py`).
- Onboarding is one screen: no multi-step wizard, no second add-account form in Settings.
- App-owned state ("seen" flags) lives in the `settings` table, never in `config.toml`.

## Why
- A second implementation would drift on autoconfig, app passwords, manual entry and error states.
- Each wizard step is a place to abandon setting up a mail client.

## Revisit when
- A first-run orientation returns: render its bindings from the resolved keymap, never static text, and trigger it on the first dispatched command, not a key press.
