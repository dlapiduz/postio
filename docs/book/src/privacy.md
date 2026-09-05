# Privacy and security

Email is probably the most sensitive thing on your computer, and mail is
attacker-controlled content that actively tries to phone home. Postio's
commitment is one sentence: **nothing leaves this machine that you did not
ask for.** Concretely, that means:

- **Remote images and tracking pixels are blocked by default**, and stay
  blocked per sender until you explicitly allow them. There is no global
  "always load images" switch — that would defeat the point.
- **Read receipts are never sent automatically.**
  `Disposition-Notification-To` is tracking with a friendly name, and
  Postio treats it as such.
- **One-click unsubscribe (`List-Unsubscribe`) only fires when you
  deliberately click it.** Sending it automatically would confirm to a
  sender that your address is live — which is exactly what a spammer wants
  to learn.
- **No link prefetch, no favicon fetching, no speculative connections of
  any kind.** The message reader has JavaScript and network access turned
  off entirely; inline (`cid:`) images resolve from the local blob store,
  not the network.
- **Forwarding and replying can't be used to smuggle out an attack.**
  Quoted content is sanitized on the way in, and the mail Postio sends is
  generated fresh from its own internal document — never a pass-through of
  whatever HTML arrived. A phishing email you forward can't make the
  recipient's client run something your own client already protected you
  from.
- **No telemetry, no crash reporting, no update ping.** Postio doesn't
  know you're using it, and neither does anyone else.
- **Your local mail store is encrypted at rest.** Because Postio backfills
  a complete copy of your mailbox rather than a recent slice, this machine
  ends up holding all of it — so that copy is encrypted, and the key lives
  in your OS keyring, not next to the data it protects. What that does and
  does not cover is worth reading in full, below.
- **Credentials live in your OS keyring**, never in a config file and
  never in a log. Postio connects over TLS wherever the server offers it.
- **Logs never contain message content** — no bodies, subjects, or
  recipient addresses, at any log level. Just ids, counts, and outcomes,
  which is enough to debug a sync problem without ever writing down what
  your mail says.

## What encryption at rest protects — and what it doesn't

"Encrypted at rest" gets oversold a lot, so here's exactly what it means for
Postio, stated honestly rather than left to your assumptions.

**Protected:** a stolen or discarded disk; a backup, an rsync copy, or a
cloud sync of Postio's data directory that wanders somewhere it shouldn't;
another user on a shared machine who gets past your account's file
permissions; anyone reading those files while your OS keyring is locked or
you're logged out. In every one of those cases, what they get is
ciphertext — the database and every blob (message and attachment) are
encrypted, and without the key sitting in your keyring, that's all it is.

**Not protected — and this matters:** a live, unlocked session. If someone
is running as you while your keyring is unlocked, they can read the
encryption key exactly the way Postio does, because that's what unlocking
the keyring means. This is not a gap Postio can close from inside a mail
client; it's why **full-disk encryption stays recommended even though
Postio encrypts its own store** — the two protect different moments. Postio
covers the disk at rest and copies that wander; full-disk encryption
additionally covers the machine while it's off or between boot and login.
Neither one covers a session an attacker already has open.

**The keyring entry is part of your mailbox, not an accessory to it.** If
you copy the Postio data directory to another machine without also moving
the key, you've copied ciphertext with no way to open it — that's not a
bug, it's the same property that makes a wandering backup safe. If you lose
the keyring entry on your own machine (a wiped keyring, a fresh OS install
without a keyring backup), the recovery path is the same in both cases: a
resync from the server. Nothing about that loses mail — everything but
drafts and queued outgoing messages is a cache of what the server already
has — but it does mean the key is not something to treat as disposable.

## This applies to the documentation site too

A privacy-first mail client whose own website loads a third-party font or
an analytics beacon would be making a claim its product contradicts, so
this site holds itself to the same rules:

- **No analytics.** Not Google's, not a "privacy-friendly" alternative,
  not a self-hosted page-view counter. A visit to these docs is not logged
  anywhere.
- **No CDN, no third-party fonts.** The typefaces on this page — the same
  ones the application itself uses — are served from this site's own
  origin. Loading this page never causes your browser to request anything
  from a third-party server.
- **No embedded video, no third-party search widget, no comment system.**
  The search box on this site runs entirely in your browser, against an
  index shipped with the page.

## When the answer might be "not yet"

Phishing and link warnings, and PGP/S-MIME support, are not implemented in
v1. They're real gaps, not oversights, and they're tracked like everything
else Postio hasn't built yet.
