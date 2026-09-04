# postio — app icon

Mark: the `/` search key. Accent field, paper slash, one square dot for unread mail.

## Colors

| Role | Hex | Token |
| --- | --- | --- |
| Field | `#5980a6` | `--color-accent` |
| Slash / dot | `#f2f2f3` | `--color-bg` |
| Dark-mode field | `#7fa3c4` | `--color-accent-400` |
| Dark-mode slash | `#1d1f20` | `--color-neutral-900` |

## Files

- `postio.svg` — master, 64×64 grid, square plate. Scale this for 128/256/512.
- `postio-rounded.svg` — same mark on the libadwaita rounded plate (r=14 on a 64 grid).
- `postio-32.svg` — 32px optical: slash thickened to 7, dot to 8.
- `postio-16.svg` — 16px optical: slash thickened to 9, dot removed.
- `postio-symbolic.svg` — monochrome `currentColor` symbolic for the GTK icon theme.

## Optical sizing rule

The slash gets heavier as the icon shrinks (6 → 7 → 9 on the 64 grid) and the dot is dropped
below 24px. Never scale `postio.svg` down to 16px directly — the stroke goes thin and the dot
turns to mush.

## Install (GTK / Flatpak)

Scalable, named for the app ID:

    share/icons/hicolor/scalable/apps/dev.postio.Postio.svg      # postio.svg
    share/icons/hicolor/symbolic/apps/dev.postio.Postio-symbolic.svg

Raster fallbacks, if you want them:

    for s in 16 24 32 48 64 128 256 512; do
      rsvg-convert -w $s -h $s postio.svg -o postio-$s.png
    done

Use `postio-16.svg` and `postio-32.svg` as the sources for those two sizes instead of the master.
