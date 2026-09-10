# Whose script runs in the reader

*2026-09-09 — #1367, #1370, and the two proofs that keep them honest.*

The reader's WebKit view used to have JavaScript off wholesale. It does not any
more, and the distinction that replaced it is easy to read as a weakening when
it is the opposite. This note exists so the next person to find
`set_enable_javascript(true)` in `hardened_settings()` does not "fix" it.

## The setting is two settings

WebKitGTK has both:

- `enable_javascript` — whether the engine can execute script **at all**,
  including script the application injects itself.
- `enable_javascript_markup` — whether script that arrives *in the markup*
  runs: a `<script>` element, an `onload=` attribute, a `javascript:` href.

Postio now sets the first **on** and the second **off**. A sender's script is
refused exactly as before; Postio's own injected script runs.

## Why the change was needed

The conversation rail has to mark the message you are actually reading, and
"actually reading" is a fact about geometry — which message occupies most of
the viewport (#1359). Nothing outside the document can measure that. With
JavaScript off wholesale, the rail's mark could only ever have been "the last
one you clicked", which the designer's brief rules out in as many words:

> The rail's entire value is the marked row being correct, and correct is
> **not** "the last one you clicked."

## Why it is not a weakening

ADR 0003 already stated the principle as *script that arrived in a message
never executes*. It never said "the engine cannot run script" — that was the
mechanism, and a blunter one than the principle required. The ADR needed a
note, not a correction.

Two proofs keep it honest, and both matter because a setting is easy to flip
and hard to notice:

- `gtk_reader.rs::sender_script_is_refused_even_with_javascript_enabled`
  renders a document carrying all three kinds of markup script — an element, a
  handler on the body, and an `onerror` on an image guaranteed to fail — each
  writing a distinct title so a failure names *which* got through. It carries
  **no CSP**, deliberately: the real documents have `script-src 'none'`, and
  with both present a pass would not say which one refused the script.
- Its control runs the same document with markup script *allowed*. If the
  title does not change there either, something else in the harness is
  refusing it and the spike proves nothing.

## Where the injected script lives, and why it is injected

`watch_for_the_current_message()` adds a debounced scroll observer **after the
load**, through `UserContentManager`, rather than writing it into the document.
Two consequences worth keeping:

- The markup a sender's message sits in still carries no script at all. The
  document would be inert if the setting were flipped back, and the observer is
  unmistakably Postio's rather than something that arrived with the mail.
- The handler is registered on the **view**, not the context. The context is
  shared, and a handler there would deliver one reader's scrolling to another.

The payload is treated as untrusted even though Postio wrote the sender: it
arrives from a page that also holds several senders' markup, so a scope the
document never rendered is dropped rather than trusted.

## What did not change

The network stays off. `script-src 'none'` stays in the CSP of every rendered
document — the engine setting and the policy are two independent refusals, and
the test above deliberately exercises only one of them so that neither can hide
a failure of the other.
