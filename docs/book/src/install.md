# Installing Postio

Postio is an alpha and, for now, Linux only: GTK4/libadwaita, Wayland
first, X11 where it happens to work. It needs GTK 4.20 and libadwaita 1.7
or newer — a current Fedora, or Ubuntu 26.04 and later.

There are three ways to get it: the prebuilt Flatpak bundle from a release,
a build from source that installs like any other app, or a plain `cargo
run` to try it without installing anything.

## The Flatpak bundle

Every tagged release publishes a `.flatpak` bundle on the
[Releases page](https://github.com/dlapiduz/postio/releases), beside a
signed build-provenance attestation and a software bill of materials.
Download `postio-<version>-x86_64.flatpak`, then:

```bash
flatpak install --user ./postio-<version>-x86_64.flatpak
flatpak run dev.postio.Postio
```

The bundle names Flathub as its runtime source, so `flatpak` offers to add
Flathub and fetch the GNOME runtime if you don't have them yet. Postio then
appears in your app grid like anything else.

A mail client holds your credentials and your mail, so it is worth checking
that a downloaded bundle was built by this project's release workflow from
the tagged commit, and hasn't been modified since. With the
[GitHub CLI](https://cli.github.com):

```bash
gh attestation verify postio-<version>-x86_64.flatpak --repo dlapiduz/postio
```

Postio isn't on Flathub yet. When it is, this whole section becomes one
`flatpak install` line.

## From source

### System dependencies

Fedora:

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 libsecret-devel glib2-devel pkgconf-pkg-config
```

Ubuntu 26.04 or newer (earlier releases ship a GTK older than Postio's
floor):

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
                 libwebkitgtk-6.0-dev libsecret-1-dev \
                 libglib2.0-dev libpango1.0-dev
```

Rust is pinned by the project's `rust-toolchain.toml` — with
[rustup](https://rustup.rs) installed, the right compiler arrives
automatically on your first `cargo` command.

### Build and install

```bash
git clone https://github.com/dlapiduz/postio.git
cd postio
scripts/install-local.sh               # builds --release, installs to ~/.local
```

That puts `postio` on your `$PATH` and adds it to your app grid, with its
icon. The script checks the build dependencies first and names every
missing one at once. `scripts/install-local.sh --uninstall` removes exactly
what it installed.

Prefer a sandboxed build you made yourself? Postio ships a Flatpak manifest
under `flatpak/` that builds against the GNOME 50 runtime — see
`flatpak/README.md` in the repository for the one-time SDK setup.

## Just try it

```bash
cargo run -p postio-app
```

builds and runs Postio from the checkout without installing anything. It
uses your real config and keyring, so an account you add here is an
account you have.

## First run

First run opens onto a one-screen setup: type your email address, and
Postio's autoconfig probe fills in the server settings for you (checking a
built-in provider table, then Thunderbird's autoconfig service, then DNS
SRV records — or you can enter everything manually). Providers that
require OAuth 2, such as Gmail and Microsoft 365, open your browser to sign
in; everything else takes a password or an app-specific password. The
credential goes straight into your desktop's keyring; it is never written
to a file.

The first sync brings the newest mail in first, then backfills every folder
to completion in the background so search and offline reading eventually
cover your whole mailbox. Attachments download when you open them.
[How sync works](sync.md) has the details, and `config.toml` can change
both behaviours.

Add a second account any time with `Ctrl+Shift+N`; the unified inbox
groups their threads together.

## Links from other applications

Postio registers itself for `mailto:` links, so a link clicked in a browser
or a "share by email" from another application opens the composer with the
address, subject and body filled in — in the running Postio if there is
one, or in a fresh one once it has opened your mail. To make it the default
mail client for your desktop:

```bash
xdg-mime default dev.postio.Postio.desktop x-scheme-handler/mailto
```

From there, drive it from the keyboard: `j`/`k` to move, `Enter` to open,
`e` to reply, `a` to archive, `u` to undo anything, `/` to search
(`from:ada is:unread …`), `Ctrl+K` for the command palette, `?` for the
full cheat sheet. Every binding is rebindable — see the
[keyboard reference](keyboard.md).

## Troubleshooting

**`cargo build` fails looking for a library** (a `pkg-config` error naming
`gtk4`, `libadwaita-1`, `webkitgtk-6.0`, or `libsecret-1`): a
system dependency from the list above is missing or too old. Check what you
have against what's needed with `pkg-config --modversion gtk4` (and so on
for the others).

**`cargo build` fails with "linker `postio-linker` not found"**: the
repository names its linker and C compiler as bare program names so one
compile cache can serve every checkout. `scripts/install-local.sh` puts
them on your `PATH` itself; a plain `cargo build` in a fresh clone needs
`scripts/install-shims.sh` run once first.

**The window fails to open, or opens with broken rendering**: only Wayland
is verified. If you're on X11 and hit a rendering issue, try a Wayland
session first, or force the X11 backend explicitly with
`GDK_BACKEND=x11 cargo run -p postio-app` before filing an issue.

**Onboarding won't save the account, or every launch reopens onboarding**:
Postio stores credentials in your OS keyring over the Secret Service D-Bus
API, which needs a running keyring daemon — GNOME Keyring or KWallet's
Secret Service integration are the common ones. Minimal desktop
environments often don't start one by default; on Fedora,
`sudo dnf install gnome-keyring` and make sure your session starts it. A
locked keyring blocks the same way — unlock it and try again.

**You see "the login keyring is locked"** on launch: the local store's
encryption key lives in the same keyring, so Postio cannot open your mail
until it is unlocked. Unlock it in your keyring application (Passwords and
Keys, on GNOME) and try again.
