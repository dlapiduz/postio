# Storyboards

An interaction written down once, as steps with expectations, in the command
vocabulary every Postio frontend shares. Each app's runner plays it, films
every step, and records where the keyboard, the cursor and the overlays are.
A design/UX reviewer that did not build the change judges the filmstrips
before the maintainer sees them.

The storyboards here play on Postio, the one desktop app (ADR 0043), and on
nothing else: its runner is `postio-gtk`'s `storyboard` example over the
demo store in `postio_gtk::demo`, driven by `scripts/storyboards.sh`. A
storyboard names `apps = ["focus"]` -- the app's name in the format -- when
its checks are the app's own.

The format's design record is
`specs/008-storyboards/contracts/storyboard-format.md`, the runner's
`specs/008-storyboards/contracts/runner.md`, and what each field of an
observation means `specs/008-storyboards/contracts/observation.md`.
