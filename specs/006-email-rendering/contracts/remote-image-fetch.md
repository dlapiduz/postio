# Contract: fetching remote images for an allowed sender

This contract implements spec FR-025, FR-026 and US5, and research R12.
It lives in `postio-runtime`, is marked `POSTIO-CONSENT:` for
`check-no-silent-tracking.py`, and is never linked into `postio-render`.

## When a fetch may start

Only when **all** of these hold:
- the user opened a message;
- `RemoteImageAllowList::remote_images_for(sender)` is `Allowed`, **or** the
  user activated "Show once" for that message;
- the URL appears as an image source in **that message's** sanitized
  document: `<img src>`, `background=`, or `background-image` in its scoped
  style.

A prefetch for an unopened message, for a neighbour in the list, or for
anything named by CSS other than a background image is never a fetch.

## The request

| Aspect | Rule |
|---|---|
| Method | `GET` |
| Schemes | `http`, `https`; each redirect target is re-checked |
| Redirects | at most 3 |
| Headers | `Accept: image/*`, and a fixed generic `User-Agent` that names neither Postio nor a version. **No** `Cookie`, `Referer`, `Origin` or `Authorization` |
| Cookies | never stored, never sent |
| Timeout | 10 s per request |
| Size | at most 16 MiB. Larger responses are aborted and recorded as `Failed { TooLarge }` |
| Concurrency | 4 per message |
| Acceptance | the body must sniff as png, jpeg, gif or webp (R4). Anything else is `Failed { NotAnImage }`, whatever `Content-Type` says |

## The result

- **Cache.** Bytes go to an in-memory, per-process cache keyed by URL. They
  are **never written to disk**.
- **Delivery.** The reader's owner adds each arrival to that message's
  resource table and requests one re-render. Arrivals within one frame are
  coalesced into one render.
- **Before arrival.** A placeholder of the declared size, so that nothing
  jumps when the image arrives (FR-026).
- **Offline.** Fetches fail quietly. The message has already rendered with
  its placeholders (001 FR-032).
- **Logs.** Only the message id, the count of URLs, and outcome counts. Never
  a URL or a host: a URL is message content.

## Tests

- **Loopback, two directions** (the #1336 discipline). An allowed sender's
  image is served and is painted, which is asserted on the snapshot's pixels.
  The same message with consent revoked sees zero connections, and the
  listener's control proves it would have seen one.
- **Headers.** The listener records the request. It has no `Cookie`,
  `Referer` or `Origin`, and its `User-Agent` does not contain "postio".
- **Prefetch.** Moving the list cursor across ten allowed senders' messages
  without opening them causes zero connections.
