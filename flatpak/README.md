# Flatpak packaging

`dev.postio.Postio.json` builds Postio against the GNOME 50 runtime. It is
the manifest a Flathub submission would use as-is.

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
`crates/postio-gtk/data/icons/128x128/apps/dev.postio.Postio.png`.

That duplication exists because `flatpak-builder`'s export step validates
every icon it installs by loading it through the host's `gdk-pixbuf`, and on
at least one real Fedora 44 box that library has no SVG loader module
registered (`gdk-pixbuf-query-loaders` lists none, and no package on that
system provides `libpixbufloader-svg.so`) — so exporting the SVG directly
fails with `is not a valid icon: Format not recognized`, even though the SVG
itself is valid (it rasterizes correctly with ImageMagick, which does not
go through gdk-pixbuf's loader modules). Shipping a PNG fallback is normal
XDG Icon Theme practice independent of this bug and sidesteps it entirely.
If your `gdk-pixbuf` does have the SVG loader, installing the scalable SVG
instead (or in addition, at `hicolor/scalable/apps/`) works too.

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

[flatpak/flatpak-builder-tools]: https://github.com/flatpak/flatpak-builder-tools
