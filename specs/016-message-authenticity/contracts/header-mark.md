# Contract: the header's "Signed" mark and warning lines

Spec D4, D4a, FR-025 and FR-025a. The rule is a pure function in
`postio_ui::reader::authenticity`:

```text
header(headers, from, trusted_authserv_ids, verdict: Option<&Verdict>, accepted: Option<&Fingerprint>)
    -> HeaderAuthenticity { warning: Option<String>, mark: Option<Mark> }
```

## The table (one unit case per row)

| Verdict | Scope | Signer is the accepted key | Mark | Signature warning |
|---|---|---|---|---|
| none | — | — | none | none |
| good | whole | no accepted key | `Signed` | none |
| good | whole | yes | `Signed · accepted` | none |
| good | whole | another key is accepted | none | "Signed with a different key than the one you accepted for ada@example.com." |
| good | part, or inline that is not the whole body | any | none | none |
| bad | any | any | none | "The signature on this message does not match its content." |
| revoked | any | any | none | "This message was signed with a key that has been revoked." |
| cant_check | any | any | none | none |
| not_here_yet | any | any | none | none |

- #1928's domain warning and impersonation line join the signature warning in
  one line, domain first, exactly as `warning()` joins them today. The mark
  is independent of the domain warning.
- A message the person sent gets neither the mark nor any warning, as today.

## Drawing (every frontend)

- **GTK**: a neutral chip after the sender in `message_header.rs`, in the
  quiet label style. It has no icon and does not use the accent. A press
  runs `view_source`.
- **Terminal**: the word `Signed` or `Signed · accepted` after the sender, in
  the dim style.
- **Mac**: the same word, through `postio-ffi`.
- **Accessible name**: "Signed. Open verification", and "Signed with the key
  you accepted for <address>. Open verification".
- **Refused in review**: a lock, tick, shield or seal glyph; green; the
  accent colour. Any of them fails this contract.

## Tests

- The unit table above, in `postio-ui`.
- `postio-session`: a signed fixture arrives, the verdict job runs, and the
  `Reading` carries `mark = Signed`. Accepting the key changes it to
  `SignedAccepted`. `z` restores `Signed`.
- `focus_suite`: one case. Open a signed message, see the chip, press it,
  and see the Verification tab.
