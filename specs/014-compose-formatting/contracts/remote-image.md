# Contract: remote images (now)

The spec's "Remote images: the Download and attach chip" is the behaviour,
FR-030 to FR-035 are the requirements, and research R13 is the design.

## The engine (`postio-session`)

```text
trait Fetcher { fn get(&self, href: &Href, limits: Limits) -> Result<Fetched, FetchError> }
attaching::fetch_remote_image(fetcher, draft, href) -> Result<InlineImage, FetchRefusal>
FetchRefusal
  Unreachable { reason }      DNS, connection, TLS, timeout, HTTP status ≥ 400
  TooLarge                    over 10 MB, declared or streamed
  NotAnImage                  magic bytes not PNG, JPEG, GIF or WebP (SVG refused)
  Offline                     the session knows the connection is down; no request is made
Limits { max_bytes: 10 MB, max_redirects: 3, timeout: 15 s, schemes: [http, https] }
```

The production `Fetcher` is `io-http`'s client. It sends no cookie,
credential or `Referer`, and sends `User-Agent: Postio`.

## The privacy cases (a counting fake `Fetcher`)

| Gesture | Requests |
|---|---|
| Type `![x](https://example.com/x.png)` in the source view or the terminal | 0 |
| Switch views; autosave; save; reopen the draft | 0 |
| Sync the draft to the server's Drafts | 0 |
| Send with the chip unpressed | 0. The sent HTML has `<a href="https://example.com/x.png">x</a>`, and the plain text `x <https://example.com/x.png>` |
| Paste a web page's HTML with `<img src="https://…">` into the rich view | 0. The image narrows away, as today |
| Reply to a message with remote images | 0 |
| Press `mod+alt+i` once | 1 |
| Press, fail, wait, go online | 1 (no retry) |
| Press again | 2 |

A check holds `fetch_remote_image`'s only caller to be the
`CommandId::AttachRemoteImage` handler, by the same grep-style rule
`scripts/checks` uses.

## The outcomes

| Fake returns | Draft after | The row says |
|---|---|---|
| 40 KB PNG | `Image { content_id }` at the chip's place, and an inline part | (row gone; the image is in the attachment list) |
| 11 MB, declared | unchanged chip | "Larger than 10 MB · not attached · it will be sent as a link" |
| 11 MB, undeclared | unchanged; the stream is stopped at 10 MB | as above |
| `text/html` body, or an SVG | unchanged | "example.com sent something that isn't a picture · it will be sent as a link" |
| Timeout or status 404 | unchanged | "Couldn't download from example.com · {reason} · Try again ctrl+alt+i" |
| (offline) | unchanged, and no request | "You're offline · press again when you're online" |

## The composer (`composer_remote_image.rs`)

- The row lives in the attachment list, under the heading "Not attached
  yet". It shows the alt text, the host, "Download and attach" with its
  keycap, and a remove button. Remove turns the chip into a link.
- The warning line before Send reads "1 image isn't attached and will be
  sent as a link · ctrl+alt+i to attach".
- While a download runs, the row reads "Downloading…". Send is live and
  sends a link for it.
- The log line for a fetch carries the draft id, the outcome and the byte
  count. A case asserts that the address does not appear in the captured
  log.

The terminal draws the same row in its attachment list, from the same
`postio_ui::compose` words.
