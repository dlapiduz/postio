# Two costs measured and left alone

*2026-09-23, from the snappiness pass that followed #1602.*

Two suspects from the felt-speed review were measured before anything was
changed for them. Neither was worth the change it would have needed; this
records the numbers so the next session does not measure them again, or
make the change on the strength of the suspicion.

## A sync commit empties the readers' page caches

The engine clears a connection's whole page cache when it begins a read
after another connection's commit (`storage/pager.rs`, `begin_read_tx`).
A first sync commits a small write unit every few milliseconds, so the
warm readers `Store::read` keeps (#1602) are cold again after every unit.
The proposal was to make units larger, or rarer, while the user is active.

Measured with a throwaway example over `seed_large(20_000)` and one
thread-list page of the inbox (`ThreadRepository::page`), median of 30,
debug Postio over opt-level-2 dependencies:

| | page |
|---|---|
| warm reader | 0.88 ms |
| the same reader right after another connection's one-row commit | 2.68 ms |

About 1.8 ms per read that follows a commit -- inside the 16 ms budget with
room to spare, and paid only by the first read after each unit. A larger
unit would hold the write lock longer, which is what `WRITE_UNIT` and
`UNIT_BUDGET` in `postio-sync/src/initial.rs` exist to prevent: a person's
write waits behind the unit in progress. The trade runs the wrong way.

## Vulkan loads every driver, and LLVM with them

`/proc/<pid>/maps` of the UI process lists `libLLVM` and three or more
`libvulkan_*` drivers on a machine with one Intel GPU. `LD_DEBUG=files`
says why: the Vulkan loader `dlopen`s every installed ICD (asahi,
broadcom, freedreno, intel, hasvk, nouveau, panfrost, powervr, radeon, lvp,
dzn) to ask whether it has a device, and `libvulkan_radeon` links LLVM,
which never unloads. The GL renderer maps it too, through `libgallium`.

What it costs, from `/proc/self/smaps` of a presented 800x600 window on the
headless compositor (`GSK_RENDERER` forced per run):

| renderer | RSS | LLVM private | drivers private | everything else private |
|---|---|---|---|---|
| vulkan (the default) | 121 MB | 2.4 MB | 1.2 MB | 37.5 MB |
| gl | 139 MB | 2.4 MB | 3.9 MB | 50.3 MB |
| cairo | 84 MB | -- | -- | 33.5 MB |

RSS is mostly shared, clean file pages; the private cost of LLVM and the
drivers together is under 4 MB. Vulkan is already the cheapest GPU
renderer. `VK_LOADER_DRIVERS_DISABLE='*lvp*,*dzn*'` trims ~4 MB of RSS and
leaves LLVM mapped (radeon still brings it), and narrowing the loader to
one driver means naming the user's hardware in the manifest, which Postio
cannot know. Cairo would drop ~16 MB more by giving up the GPU. None of it
was worth it.

## Addendum, 2026-09-24: the reader's cache model and GPU policy

#1603 asked for both, measured on the app. A scratch `gtk_suite` case built
the application's own `Reader`, drew forty table-heavy newsletters into it
one after another, and read the resident size of its WebKit processes, two
runs per arm, toggling one setting on the shared reader context:

| setting | web process | network process |
|---|---|---|
| default (`CacheModel::WebBrowser`, acceleration on demand) | 135 MB | 52 MB |
| `CacheModel::DocumentViewer` | **153 MB** | 52 MB |
| `HardwareAccelerationPolicy::Never` | 135 MB | 52 MB |

`DocumentViewer` is the setting mail clients are usually told to use, and
here it cost 18 MB more, reproducibly. The acceleration policy made no
difference on the headless compositor, which renders in software either
way, so this says nothing about a real GPU -- but nothing here argues for
changing it. Both stay at the default.
