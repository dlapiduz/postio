# Contract: what a formatted message is on the wire

The test that holds this contract is `postio-body`'s
`tests/body_suite/formatting_wire.rs` (T010). It renders the document
below through `postio_body::render`, then:
- pins both parts byte for byte;
- requires `harden` to change nothing;
- requires `markdown::to_document(&markdown::from_document(&d)) == d`
  and `parse(&to_html(&d)) == d`.

## The document

Written here as the Markdown that reads into it:

````markdown
# Plan

Some **bold**, *italic*, ~~struck~~ and `code`, and a [link](https://example.com/x).

## Steps

1. first
2. second
   - nested
- [ ] open task
- [x] done task

7. seventh

> quoted
>
> > nested quote

```rust
fn main() {}
```

---

| Item | Qty |
|:-----|----:|
| Widget | 3 |

line one\
line two
````

## The HTML part

One line per block here for reading; the real output has no whitespace
between blocks (the normal form).

```html
<h1>Plan</h1>
<p>Some <strong>bold</strong>, <em>italic</em>, <del>struck</del> and <code>code</code>, and a <a href="https://example.com/x">link</a>.</p>
<h2>Steps</h2>
<ol><li><p>first</p></li><li><p>second</p><ul><li><p>nested</p></li></ul></li></ol>
<ul data-postio-task="list" style="list-style-type:none"><li data-postio-task="open">☐ <p>open task</p></li><li data-postio-task="done">☑ <p>done task</p></li></ul>
<ol start="7"><li><p>seventh</p></li></ol>
<blockquote><p>quoted</p><blockquote><p>nested quote</p></blockquote></blockquote>
<pre data-postio-lang="rust">fn main() {}
</pre>
<hr>
<table border="1" cellpadding="6" cellspacing="0"><thead><tr><th align="left">Item</th><th align="right">Qty</th></tr></thead><tbody><tr><td align="left">Widget</td><td align="right">3</td></tr></tbody></table>
<p>line one<br>line two</p>
```

The exact place of the glyph against an item's first paragraph (inside the
`<p>` or before it) is pinned by T010 against how the reader draws it. The
rule is fixed: the glyph comes first and is followed by one space.

## The plain-text part (`format=flowed`)

```text
Plan
====

Some bold, italic, ~~struck~~ and code, and a link <https://example.com/x>.

Steps
-----

1. first
2. second
    - nested

- [ ] open task
- [x] done task

7. seventh

> quoted
>
>> nested quote

```rust
fn main() {}
```

----

| Item   | Qty |
|:-------|----:|
| Widget |   3 |

line one
line two
```

- The link form (`label <url>`) is `Links::Spelled` as it is today; T010
  pins whatever today's form is and does not change it.
- Every code line and table row is a fixed line, with no trailing space.
- A line in a code block that begins with ` `, `>` or `From ` is
  space-stuffed.

## The MIME shape

- **A document with any construct other than paragraphs and text:**
  `multipart/alternative` (`text/plain; format=flowed`, `text/html`). It is
  wrapped in `multipart/related` when an inline image is present. This is
  unchanged.
- **A plain-text document:** `text/plain; format=flowed` alone. This is
  unchanged too (spec FR-018).
