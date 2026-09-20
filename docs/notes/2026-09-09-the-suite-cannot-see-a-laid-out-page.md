# The suite cannot see a laid-out page (2026-09-09, #1334)

Four consecutive CI failures on one assertion, four local passes, and the same
mistake underneath each: **asserting on rendered geometry in an environment
that renders nothing.**

Written down because the assertion looked right every time, and because the
numbers CI returned looked like *wrong behaviour* rather than like *no
measurement*, which is what kept the diagnosis moving in the wrong direction.

## What the numbers were

The claim under test was FR-019a: a sender's declared width is honoured. Each
version, and what CI said:

| Assertion | CI answered |
|---|---|
| `scrollWidth > clientWidth` on the container | `fits` — needs the window to be narrower than the content, and a compositor need not honour a requested size (#933) |
| container `scrollWidth >= 3000` | `32px` |
| styled element wider than an unstyled one | `0px against an unstyled 0px` |
| `getComputedStyle(el).width` | `33.554428px` |

Every one of those reads as a layout that disagrees. None of them is. On the
suite's display **nothing is laid out at all**: the window is never presented,
so every `getBoundingClientRect` is zero and every length that resolves against
layout is meaningless.

`32px` and `33.554428px` are not near-misses. They are what a resolved length
looks like when there is no box to resolve against.

## The rule that follows

**A test here may assert on the cascade. It may not assert on the layout.**

- `getComputedStyle(el).color` — fine. Colour comes from the cascade and needs
  no box. `gtk_reader`'s `computed` helper has worked this way all along, which
  is why it looked like a counter-example.
- `getComputedStyle(el).width` — **not** fine. A computed `width` is the *used*
  value and resolves against layout like everything else. This is the one that
  looks safe and is not.
- `getBoundingClientRect`, `elementFromPoint`, `scrollWidth`, `clientWidth` —
  not fine, in any comparison, including comparisons between two of them.
- `el.style.width`, `el.getAttribute('width')`, the presence of a rule in the
  document — fine. These are DOM facts.

Where a claim is genuinely about layout — containment, overlap, whether one
message can paint over another — the honest options are to **guard and skip**
with a message saying why (`one_senders_styling_cannot_reach_another_message`
does this now) or to move the claim to a pure rule over given geometries
(`postio_ui::reader::rail::current` is the example). Never to weaken it into
something that passes on zeroes: `0 > 0` is false, so a pane-overflow assertion
"passed" on CI for months by being meaningless, which is worse than failing.

## Why this is not fixable by trying harder

`scripts/headless-runner.sh` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which is
the right mitigation for #272 and should stay. #1307 records the consequence
from the other end: no test in this repository exercises the rendering path a
user actually gets. This note is the same fact stated as a rule for whoever
writes the next reader test.

The cost of not knowing it was four CI rounds at roughly ten minutes each,
spread over three landings, while the real failure — an environment variable
leaking between test cases in one process — sat underneath and was read as
flakiness.
