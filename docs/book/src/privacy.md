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
  in your OS keyring, not next to the data it protects. Worth knowing
  plainly: a backup of your Postio data directory is a backup of your
  entire mailbox. Losing the keyring entry costs you a re-sync; it never
  costs you mail.
- **Credentials live in your OS keyring**, never in a config file and
  never in a log. Postio connects over TLS wherever the server offers it.
- **Logs never contain message content** — no bodies, subjects, or
  recipient addresses, at any log level. Just ids, counts, and outcomes,
  which is enough to debug a sync problem without ever writing down what
  your mail says.

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
