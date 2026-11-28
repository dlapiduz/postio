# Contract: reading the GnuPG keyring

Spec D12 and FR-031a; plan R7.

## The switch

- Settings → Privacy → "Use keys from my GnuPG keyring". It is off by
  default, and stored as `[signatures] gnupg_keyring = true` in
  `config.toml`.
- Turning it on reads the keyring at once. Turning it off removes keys whose
  only source is `gnupg` and that nobody accepted.
- Command: `reload_gnupg_keys` ("Reload GnuPG keys"), in the command bar
  only, offered when the switch is on.

## What is opened (asserted by a test listing every path opened)

| Path, under `$GNUPGHOME` or else `~/.gnupg` | Opened | How |
|---|---|---|
| `public-keys.d/pubring.db` | if present, first | read-only through the store engine, after S3; otherwise the unreadable state |
| `pubring.kbx` | if present and no keyboxd database | read whole, then `keyblocks_from_keybox` |
| `pubring.gpg` | if present and neither of the above | read whole, then `keyblocks_from_pubring_gpg` |
| `private-keys-v1.d/`, `secring.gpg`, `trustdb.gpg`, any lock file | **never** | — |

- No process is spawned. `check-no-silent-tracking.py` refuses a `gpg`
  spawn anywhere in `crates/*/src`.
- Nothing is written, renamed or locked.
- A read whose stated lengths do not add up is retried once, and then
  reports `skipped`.

## Results

- Each keyblock becomes a `public_keys` row, with a `public_key_sources` row
  for `gnupg`, and `public_key_addresses` rows for every user id address.
- Settings shows "312 keys read from your GnuPG keyring · 2 skipped", or the
  unreadable state: "Postio can't read this GnuPG keyring. Export the keys
  you need with `gpg --export --armor <address> > keys.asc` and import the
  file."
- Logs carry the counts only: no fingerprints, user ids or paths beyond the
  directory.

## Flatpak

`flatpak/dev.postio.Postio.json` gains `--filesystem=~/.gnupg:ro`, with a
manifest comment that says why. A non-default `GNUPGHOME` outside it is the
unreadable state, which names the variable.

## Tests

- `postio-signature`: keybox fixtures built in the test (a header blob, an
  OpenPGP blob, an X.509 blob, a truncated blob), and `pubring.gpg` with
  trust packets between keys.
- `postio-host`: a temp `GNUPGHOME` with each format. Assert the keys read,
  that secret files are never opened, the switch off and on, and a reload
  after the file changes.
- No test reads the real `~/.gnupg`.
