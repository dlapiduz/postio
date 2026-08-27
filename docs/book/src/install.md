# Installing Postio

Postio is pre-release and, for now, Linux only: GTK4/libadwaita, Wayland
first, X11 where it happens to work. There's no packaged release yet, so
installing it means building from source.

## System dependencies

Fedora 40+:

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Ubuntu 26.04 (earlier releases ship a GTK older than Postio's floor):

```bash
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
                 libwebkitgtk-6.0-dev libsqlite3-dev libsecret-1-dev \
                 libglib2.0-dev libpango1.0-dev
```

Rust is pinned by the project's `rust-toolchain.toml` — with
[rustup](https://rustup.rs) installed, the right compiler arrives
automatically on your first `cargo` command.

## Build and install

```bash
git clone https://github.com/dlapiduz/postio.git
cd postio
scripts/install-local.sh               # builds --release, installs to ~/.local
```

That puts `postio` on your `$PATH` and adds it to your app grid, with its
icon. `scripts/install-local.sh --uninstall` removes exactly what it
installed.

Prefer to just try it without installing anything?

```bash
cargo run -p postio-app
```

Prefer a sandboxed build? Postio ships a Flatpak manifest under `flatpak/`
that builds against the GNOME 50 runtime — see `flatpak/README.md` in the
repository for the one-time SDK setup. Postio isn't on Flathub yet, so for
now that means building the Flatpak yourself too.

## First run

First run opens onto a one-screen setup: type your email address, and
Postio's autoconfig probe fills in the server settings for you (checking a
built-in provider table, then Thunderbird's autoconfig service, then DNS
SRV records — or you can enter everything manually). Your password goes
straight into your desktop's keyring; it is never written to a file.
iCloud accounts need an app-specific password, generated at
<https://account.apple.com>.

From there, drive it from the keyboard: `j`/`k` to move, `Enter` to open,
`e` to reply, `a` to archive, `u` to undo anything, `/` to search
(`from:ada is:unread …`), `Ctrl+K` for the command palette, `?` for the
full cheat sheet. Every binding is rebindable — see the
[keyboard reference](keyboard.md).

## Troubleshooting

**`cargo build` fails looking for a library** (a `pkg-config` error naming
`gtk4`, `libadwaita-1`, `webkitgtk-6.0`, `sqlite3`, or `libsecret-1`): a
system dependency from the list above is missing or too old. Check what you
have against what's needed with `pkg-config --modversion gtk4` (and so on
for the others).

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
