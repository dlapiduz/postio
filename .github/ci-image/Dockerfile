# The machine the Linux CI jobs run in: the system libraries the workspace
# links against, the compositor its GTK suites draw on, and the pinned Rust
# toolchain, installed once here instead of on every job.
#
# Every job used to `apt-get install` about two hundred packages from
# azure.archive.ubuntu.com, and that mirror is often slow: in the 30 CI runs
# before 2026-10-22, 18 had a job spend more than 10 minutes there -- up to
# 40 -- and those runs took a median 40.5 minutes against 14.9 for the rest.
# The tests were never the half-hour; the mirror was.
#
# Built and pushed by `.github/actions/ci-image`, tagged with a hash of this
# file and `rust-toolchain.toml`, so a change to either builds a new image
# in the pull request that makes it, and nothing else ever does.
#
# The base is pinned by digest for the same reason the toolchain is pinned:
# the image changing should look like a commit, not like weather. Bump it on
# purpose, with the security updates it brings.
FROM docker.io/library/ubuntu:26.04@sha256:42527649dbc4f9197a6073f426f488cbff0e9cc9f2afce32d91c383e4875634a

ARG DEBIAN_FRONTEND=noninteractive

# The libraries are the Ubuntu equivalents of the Fedora packages in
# README.md. `mold` is the linker `scripts/linker.sh` prefers (lower peak
# memory per link, see there); `mutter` and `dbus-daemon` are the headless
# compositor and session bus the GTK suites run on. That first group is
# also the list `.github/actions/rust-workspace` installs on a bare runner
# (`release.yml`); the two must agree.
#
# The rest is what a GitHub runner image has and this one would not: git
# for checkout and `scripts/restore-mtimes.py`, gh for the rustdoc job's
# changed-crate query, curl for `scripts/install-nextest.sh`, procps for
# `scripts/headless-runner.sh`'s pgrep, zstd for `actions/cache`, sudo for
# steps written for the runner, and fonts for anything that draws text.
RUN apt-get update \
 && apt-get install --no-install-recommends -y \
      build-essential \
      pkg-config \
      libgtk-4-dev \
      libadwaita-1-dev \
      libwebkitgtk-6.0-dev \
      libsqlite3-dev \
      libsecret-1-dev \
      libglib2.0-dev \
      libpango1.0-dev \
      mutter \
      dbus-daemon \
      mold \
      ca-certificates \
      curl \
      git \
      gh \
      procps \
      python3 \
      sudo \
      xz-utils \
      zstd \
      fonts-dejavu-core \
      fonts-cantarell \
 && rm -rf /var/lib/apt/lists/*

# The jobs run as the runner's own user (uid 1001, the owner of the
# workspace and of HOME on the host), not as root. Root reads a file whose
# permissions say it may not, so a test of that refusal would pass for the
# wrong reason. `sudo` without a password, as on the runner.
RUN groupadd --gid 1001 runner \
 && useradd --uid 1001 --gid 1001 --no-create-home --home-dir /github/home runner \
 && echo 'runner ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/runner \
 && git config --system --add safe.directory '*'

# rustup and the pinned toolchain live under /opt, read-only to the jobs.
# CARGO_HOME is left to default to ~/.cargo, which is where the build cache
# keeps the registry and where `scripts/install-shims.sh` and
# `scripts/install-nextest.sh` put their binaries -- the same paths as on a
# runner, so neither needed to change.
#
# The toolchain comes from `rust-toolchain.toml` itself, so the pin is still
# written once; `check-toolchain-pinned.py` is about exactly that.
ENV RUSTUP_HOME=/opt/rustup \
    PATH=/github/home/.cargo/bin:/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    POSTIO_CI_IMAGE=1
COPY rust-toolchain.toml /tmp/toolchain/rust-toolchain.toml
RUN curl -fsSLo /tmp/rustup-init https://static.rust-lang.org/rustup/archive/1.29.2/x86_64-unknown-linux-gnu/rustup-init \
 && echo '5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4  /tmp/rustup-init' | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && CARGO_HOME=/opt/cargo /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none \
 && cd /tmp/toolchain \
 && CARGO_HOME=/opt/cargo /opt/cargo/bin/rustup toolchain install \
 && CARGO_HOME=/opt/cargo /opt/cargo/bin/rustup default \
      "$(CARGO_HOME=/opt/cargo /opt/cargo/bin/rustup show active-toolchain | cut -d' ' -f1)" \
 && CARGO_HOME=/opt/cargo /opt/cargo/bin/rustc --version \
 && CARGO_HOME=/opt/cargo /opt/cargo/bin/cargo clippy --version \
 && rm -rf /tmp/rustup-init /tmp/toolchain /opt/rustup/downloads /opt/rustup/tmp \
 && chmod -R a+rX /opt/rustup /opt/cargo

USER runner
