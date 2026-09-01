#!/usr/bin/env bash
# The linker driver rustc invokes for the x86_64 Linux target, wired up in
# .cargo/config.toml as `[target.x86_64-unknown-linux-gnu] linker`: use mold
# when the machine has it, get out of the way when it does not.
#
# Measured on this box (2026-09-01), relinking postio-app after a one-line
# change: mold's peak RSS at the link step was 36-56% below lld's across two
# runs (497-723 MB vs. a steady ~1.1 GB), with wall-clock roughly a wash.
# Memory is the number that matters here -- four sessions each linking this
# GTK+WebKit binary at once is exactly the scenario the neighbouring
# `-Wl,--threads=2` tuning already exists for, and mold lowers the ceiling
# further rather than replacing that tuning. `-Wl,--threads=2` still reaches
# whichever linker is in use, mold included -- it is forwarded through `cc`
# from rustc's own `-C link-arg`, unaffected by which linker driver this
# script hands off to.
#
# `cc` is still the linker driver rustc hands off to (see cc-wrapper.sh for
# why `cc` and not a hardcoded gcc/clang); this wrapper only adds
# `-fuse-ld=mold` to what it forwards. `sudo dnf install mold` on Fedora.
#
# Fail open, like rustc-wrapper.sh and cc-wrapper.sh: no mold on the box and
# this is exactly `cc "$@"`, unmodified -- a fresh clone still links, just
# with whatever `cc` resolves to by default (lld or bfd) and the RSS this
# wrapper exists to cut.
if command -v mold >/dev/null 2>&1; then
    exec cc -fuse-ld=mold "$@"
fi
exec cc "$@"
