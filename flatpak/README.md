# Flatpak packaging

`dev.postio.Postio.json` builds Postio against the GNOME 50 runtime. It is
the manifest a Flathub submission would use as-is.

`dev.postio.PostioTui.json` builds the terminal frontend, `postio-tui`,
against the plain freedesktop runtime: no GTK and no WebKit, which is most of
why it is the smaller package. Flathub does not take console-only
applications, so it is published as a release bundle only.

## One store, two packages

Both packages ship `postio-daemon`, the one process that opens the store
([ADR 0041](../docs/decisions/0041-one-process-owns-the-store.md)), beside
their frontend. Both grant the same three host directories:
- `xdg-data/postio` for the store;
- `xdg-config/postio` for `config.toml`;
- `xdg-run/postio` for the daemon's socket.

Postio reads the host's XDG directories inside a sandbox rather than the
per-app ones under `~/.var/app`. So with both installed, the desktop app and
the terminal show the same mailbox, and whichever starts first starts the
daemon. `packaging.rs` in `postio-tui`'s tests fails if either manifest
drops one of the grants or the daemon.

The two bundles must come from the same release. The daemon refuses a client
from a different build, and says which two versions disagree.

**An existing desktop Flatpak's mail is not moved.** Its store used to live
under `~/.var/app/dev.postio.Postio/data/postio`. It now lives at
`~/.local/share/postio`, the same place a source install and the terminal
use, and Postio syncs it again there. There are no installs this has to be
gentle with (the constitution's no-backwards-compatibility rule), and the
old directory can be deleted.

## One-time setup

```bash
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install -y flathub org.gnome.Platform//50 org.gnome.Sdk//50 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
sudo dnf install flatpak-builder   # or: flatpak install flathub org.flatpak.Builder
```

## Regenerating `cargo-sources.json`

Flatpak builds offline, so every crate Cargo would otherwise fetch has to be
listed up front with its checksum. `cargo-sources.json` is that list,
produced from `Cargo.lock` by the vendored `flatpak-cargo-generator.py`
(MIT, from [flatpak/flatpak-builder-tools], pinned at commit `f03a673`).

Regenerate it whenever `Cargo.lock` changes:

```bash
python3 flatpak/flatpak-cargo-generator.py Cargo.lock -o flatpak/cargo-sources.json
```

It needs `aiohttp`, `PyYAML` and `tomlkit` (`pip install --user aiohttp PyYAML tomlkit`,
or run it with `uv run flatpak/flatpak-cargo-generator.py ...` — the script
carries its own PEP 723 dependency block).

`cargo-sources.json` is not committed: it is a derived artifact of
`Cargo.lock`, and a stale copy is worse than an absent one — the project's
convention (see `docs/keybindings.md`) is that a generated file either stays
in lockstep with a test that catches drift, or isn't checked in at all. This
one has no such test yet, so regenerate it right before building.

## Building

```bash
flatpak-builder --user --install --force-clean flatpak/build-dir flatpak/dev.postio.Postio.json
flatpak run dev.postio.Postio

flatpak-builder --user --install --force-clean flatpak/build-dir-tui flatpak/dev.postio.PostioTui.json
flatpak run dev.postio.PostioTui
```

`flatpak run dev.postio.PostioTui` is a lot to type at a shell prompt. An
alias does it:

```bash
alias postio-tui='flatpak run dev.postio.PostioTui'
```

The `postio` module's source is `type: dir` pointing at the repository root
(`..`, since the manifest lives in `flatpak/`), skipping `target/`, `.git/`,
`flatpak/` itself. That means **the build sees whatever is on
disk, uncommitted changes included** — commit or stash first if you want a
build that matches `HEAD`.

## Why a PNG icon, not just the scalable SVG

The app ships its real icon as
`crates/postio-gtk/data/icons/scalable/apps/dev.postio.Postio.svg`, and that
is what the running app itself uses (via `postio_gtk::resources`, bundled
into the `GResource`). For the *installed* desktop icon —
`/app/share/icons/hicolor/...`, which is what the shell's app grid and
alt-tab switcher read via the freedesktop icon theme spec — this manifest
also installs a 128×128 rasterization,
`crates/postio-gtk/data/icons/128x128/apps/dev.postio.Postio.png`, alongside
16×16 and 32×32 (#1023). Those two smaller ones are not a fallback for the
same reason: the mark is *drawn* heavier as it shrinks — the slash thickens
and the unread dot is dropped below 24px — so they are separate artwork
rather than a downscale, and no scalable SVG can express them. Every other
size is left to the SVG on purpose: a raster there would only override
something sharper.

The scalable SVG is installed too, at `hicolor/scalable/apps/`, with the
symbolic variant at `hicolor/symbolic/apps/`: it is what the shell asks for
at 96 and at every 2x size, and `appstreamcli compose` renders the catalog's
own icon set from it. For a while it was kept out, because the compose step
failed to read it (`file-read-error`) and that was taken for the runtime
lacking an SVG loader. It was the file: an image loader sniffs the first 257
bytes for `<svg` before it trusts the extension, and the icon opened with a
680-byte comment. `desktop_entry.rs` in `postio-gtk`'s logic suite now
asserts the tag sits inside that window for every bundled SVG, and that the
manifest installs both files.

Regenerate the PNG if the SVG ever changes:

```bash
magick -background none crates/postio-gtk/data/icons/scalable/apps/dev.postio.Postio.svg \
    -resize 128x128 crates/postio-gtk/data/icons/128x128/apps/dev.postio.Postio.png
```

## Permissions

| Flag | Why |
|---|---|
| `--share=network` | IMAP/SMTP |
| `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11`, `--device=dri` | GTK4/WebKitGTK windowing and GPU rendering |
| `--talk-name=org.freedesktop.secrets` | Secret Service keyring, where account passwords live (never in `config.toml`) |
| `--filesystem=xdg-download` | Saving attachments directly; ordinary file *choosers* go through the portal and need no static permission |
| `--filesystem=xdg-data/postio:create`, `xdg-config/postio:create`, `xdg-run/postio:create` | The store, `config.toml` and the daemon's socket, shared with the other Postio package (above) |

The terminal package asks for the same, less the GPU, plus
`--talk-name=org.freedesktop.Notifications` for new-mail notices. It keeps
the Wayland and X11 sockets only to read an image off the clipboard when you
paste one.

[flatpak/flatpak-builder-tools]: https://github.com/flatpak/flatpak-builder-tools
