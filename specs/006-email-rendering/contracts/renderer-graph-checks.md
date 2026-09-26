# Contract: the checks that prove the renderer is disconnected and memory-safe

This contract implements spec FR-001 and FR-023a, and ADR 0042 (to be
written on this branch). Both checks run under `scripts/check.sh`. Like
every check in `scripts/checks/`, each names its own fix when it fails.

## 1. `check-crate-boundaries.py`: a new `RULES["postio-render"]`

This uses the existing mechanism: `find_violations` walks `cargo metadata`
over normal, build and own-dev edges, transitively.

```python
"postio-render": {
    "banned": [
        # toolkit / web engine
        "gtk4", "gtk4-sys", "glib", "gio", "webkit6", "webkit6-sys",
        # Postio crates that network or store
        "postio-transport", "postio-sync", "postio-runtime", "postio-storage",
        "postio-account", "postio-jmap", "postio-gmail", "postio-smtp",
        "io-http", "pimalaya-stream",
        # Blitz's own networking
        "blitz", "blitz-net",
        # network and TLS stacks
        "reqwest", "hyper", "h2", "ureq", "curl", "curl-sys", "isahc", "surf",
        "rustls", "tokio-rustls", "native-tls", "openssl", "openssl-sys",
        "socket2", "mio", "tokio", "async-std",
    ],
    "why": "spec 006 FR-001 / ADR 0042: the renderer is incapable of a "
           "network connection by construction; remote bytes enter only "
           "through postio-runtime's RemoteImageFetcher",
},
```

## 2. `check-renderer-is-memory-safe.py`: new

It walks the **resolved** graph (`cargo metadata --format-version 1`,
`resolve.nodes`, with features) from `postio-render`, over normal and build
edges.

| Fails when | Why | Named fix |
|---|---|---|
| any package declares `links`, other than the allowlist `rayon-core` and `servo_style_crate` (uniqueness markers only) | native code linked | "find which feature pulled it in (`cargo tree -i <pkg> -e features`) and turn it off" |
| any package name ends in `-sys` | native bindings | same |
| `cc`, `cmake` or `bindgen` is a build dependency of any package in the graph | compiled C/C++ | same |
| `image` has `avif`, `avif-native`, `tiff`, `exr`, `bmp` or `ico` | extra decoders; avif pulls C/asm | "postio-render enables png, jpeg, gif, webp only (research R4)" |
| `blitz-dom` has `net` or `system-fonts`; `fontique` or `parley` has `system` | networking; fontconfig (C) on the content path | "fonts come from FontSet via fontdb (research R3)" |
| `blitz-paint` lacks `svg` | SVG parsed but never painted | "enable blitz-paint/svg (research R5)" |
| the workspace sets `panic = "abort"` in any profile | a caught render panic would become an app crash | "keep unwind (research R6)" |

**Allowlisted build-time tool:** stylo's `build.rs` runs `python3` to
generate Rust. That is not linked code, and nothing in the check needs to
know about it.

## 3. The observed half (a test, not a check)

`crates/postio-render/tests/egress.rs` binds a loopback listener. It renders
every hostile corpus fixture with every remote URL rewritten to point at the
listener, the fixtures in the consented state included (the renderer must
not fetch even then; the app does). It asserts that **zero** connections
were accepted.

This test does not take the listener's silence as proof. Its **control**
first proves the listener would see a connection: it connects to the
listener from the test itself and asserts the connection was counted. Only
then does it render and assert zero. That is the same two-directional
discipline as #1336.
