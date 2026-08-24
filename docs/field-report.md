# The Postio field report

A survey of the mail clients Postio is positioned against, kept as a citable
document rather than scattered claims. Several open issues already reference
"the field report" by name (#1, #3, #4, #7, #13, #16, #24) — this is that
document, reconstructed because the original was never committed anywhere,
and extended with Superhuman at the maintainer's request.

**Peers:** Gmail, Thunderbird (and its fork Betterbird), Aerion, Superhuman.
Aerion is the closest architectural peer — a native, protocol-based,
keyboard-first Linux client, the same weight class Postio is in. Superhuman
is the closest *positioning* peer — "fastest email client," keyboard-driven,
built for people with too much email — but a structurally different product,
which is most of what makes the comparison useful.

Researched 2026-08-24 via public sources; see citations at the end of each
section. Superhuman specifics are dated — subscription products change
pricing and feature gating often — so treat exact numbers as approximate and
the shape of the comparison as the durable part.

---

## Comparison matrix

| | Gmail | Thunderbird | Aerion | Superhuman | **Postio (v1)** |
|---|---|---|---|---|---|
| Protocol | proprietary API | IMAP/SMTP/POP | IMAP/SMTP + Gmail/MS APIs, Proton Bridge | **Gmail + Outlook only — no IMAP** | IMAP/SMTP |
| Providers supported | Google only | any | Gmail, MS 365, iCloud, GMX, generic IMAP | Gmail, Outlook | one account, any IMAP+SMTP |
| Where mail lives | Google's servers | local profile | local | **Superhuman's servers** | local SQLite + blob store |
| Offline | partial (PWA) | full | full | **not offered** | full, by design |
| Keyboard-first | partial | partial, addon-dependent | yes, vim-style | yes — 100+ shortcuts, the flagship pitch | yes — founding principle |
| Command palette | no | no | — | `Cmd+K` | `Ctrl+K`, generated from one registry |
| Multi-account | yes | yes | yes | yes (within Gmail/Outlook) | not in v1 — roadmap #1 |
| OAuth | native | yes | yes, CASA Tier 2 (2025-04-25) | n/a (IS Gmail/Outlook) | app password only in v1 — roadmap #2 |
| Rich-text compose | yes | yes | yes | yes | plaintext in v1 — roadmap #3 |
| Address book | yes | yes | CardDAV + Google/MS contacts | — | mail-history only in v1 — roadmap #4 |
| Filters / rules | yes | yes | — | Split Inbox (fixed streams) | schema built, not wired — roadmap #5 |
| AI: summarize / draft | yes (Gemini) | no | planned (Ollama) | **yes — flagship**, acquired by Grammarly Oct 2025 | deferred by design — epic #20 |
| Read receipts / open tracking | opt-in, sender-visible only | no | — | **yes — "Read Statuses": device, count, timing** | **never — CLAUDE.md §21** |
| Follow-up nudges on sent mail | "Nudges" | no | — | yes — "Auto Reminders" | not tracked (see below) |
| Snippets / canned responses | yes ("Templates") | addon | — | yes, base tier | not tracked (see below) |
| Pricing | free (ad-supported) | free, OSS | free, OSS | **$30–40/month subscription** | free, OSS |
| Local-first / privacy stance | no | partial (local storage, no protocol privacy design) | yes | **no — cloud processing, SOC 2 / GDPR compliant, not local-first** | founding principle |

Cells in bold are the ones worth reading twice.

---

## Superhuman, in more detail

The reason it's worth its own section: Superhuman is the product most likely
to get cited *at* Postio ("why not just use Superhuman") and the one whose
gaps are least visible from the outside, because its marketing is entirely
about speed and its structural limits don't show up until you try to connect
an account it doesn't support.

**What it is.** A $30–40/month subscription layer on top of Gmail and
Outlook — not a mail client in the protocol sense. It has never supported
IMAP as a first-class path; multiple current user reports describe generic
IMAP and iCloud as unsupported today, with at least one account of IMAP
support existing in the past and being removed. [Fastio](https://fast.io/resources/superhuman-ai-review-2026/),
[Gmelius](https://gmelius.com/blog/superhuman-ai-review),
[getmailbird.com](https://www.getmailbird.com/best-superhuman-alternatives-macos/).

**What it's actually good at.** 100+ keyboard shortcuts and a `Cmd+K`
command bar are core to the product, not an add-on — genuinely the same bet
Postio is making. Split Inbox automatically separates VIP mail, newsletters,
and notifications into streams. Snippets standardize repeated replies.
[Morgen](https://www.morgen.so/blog-posts/superhuman-pricing),
[usecarly.com](https://www.usecarly.com/blog/superhuman-pricing/).

**AI.** AI Replies, Summarize, Auto Labels, Instant Reply Suggestions on the
Starter tier; Auto Drafts, Ask AI, and CRM integration (HubSpot, Salesforce)
on Business. Acquired by Grammarly in October 2025, which is now folding its
own AI writing work in. This is a shipped, flagship surface — years ahead of
where Postio's AI work sits by design (epic #20, deferred to post-v1 so core
mail/search/keyboard land excellently first).
[ventureburn.com](https://ventureburn.com/superhuman-email-review/),
[Gmelius](https://gmelius.com/blog/superhuman-ai-review).

**Read Statuses.** The feature most worth naming directly: Superhuman tells
the sender when a recipient opened an email, how many times, and on what
device. This is a read-receipt / open-tracking system, marketed as a trust
and follow-up tool. [usecarly.com](https://www.usecarly.com/blog/superhuman-pricing/).

**Data handling.** SOC 2 compliant, GDPR/CCPA/FERPA-aware, encrypted at
rest, and explicit that customer content doesn't train third-party AI
models — a real privacy posture, but a *compliance* one: data still lives on
Superhuman's servers and is processed there, including by AI.
[superhuman.com/legal/privacy-policy](https://superhuman.com/legal/privacy-policy),
[Superhuman Agents Privacy Overview](https://help.superhuman.com/hc/en-us/articles/46242109586061-Superhuman-Agents-Privacy-and-Security-Overview).

---

## Gaps worth tracking

Real capabilities Superhuman ships that nothing in the current backlog
covers directly. Noted here as findings, not filed as issues — worth a
maintainer call on whether and how they fit the roadmap.

1. **Snippets / canned responses.** No corresponding issue exists today.
   Cheap relative to its value: a named template, inserted at compose time,
   is a small feature next to what's already built (drafts, identities,
   signatures).
2. **Follow-up reminders on sent mail.** Distinct from Snooze (#6, which
   defers an *incoming* message) — this tracks a message you *sent* and
   resurfaces it if nobody replies within a chosen window. No existing issue
   covers the "resurface a sent message" half of that; Snooze covers the
   deferral mechanism it would reuse.
3. **Split Inbox as shipped today.** Conceptually close to Smart labels
   (#8) and Mailing-list grouping (#9), but Superhuman's version is simpler
   and always-on rather than AI-classified. Worth checking #8/#9 against
   this simpler shape specifically when either is picked up — the cheap
   version might be worth having before the AI-driven one.

## Gaps that are deliberate — not to be closed

Differences worth stating plainly rather than treating as backlog items,
because Postio has already decided against them.

1. **Read Statuses.** Superhuman's flagship trust feature is exactly what
   CLAUDE.md's privacy section rules out by name: *"Read receipts are never
   sent automatically. `Disposition-Notification-To` is tracking with a
   friendly name."* Any comparison that invites "why doesn't Postio do
   this" should say so directly rather than read as an oversight.
2. **Cloud-hosted, server-processed mail and AI.** Structural to
   Superhuman, not a phase it will grow out of — it's a thin client over two
   providers' APIs. Postio's local SQLite store and blob directory are the
   opposite bet, made on purpose (`docs/PRODUCT.md` §6, §21).
3. **No iCloud, no generic IMAP.** The most concrete version of the above:
   Superhuman cannot connect the account this project's own v1 targets.
   Every comparison against Superhuman starts from a case it structurally
   cannot handle.

---

## Gmail, Thunderbird, Aerion — the shorter version

These three were the original comparison set (#1–#15 cite them directly)
and are lower-risk to characterize from what's already in the backlog and
public knowledge, so they get less space here than the newly-added
Superhuman section.

**Gmail.** The default most people are leaving. Free, ad-supported, Google
account only. Strong search, native Gemini AI (summarize, draft, Gemini in
Gmail chat), built-in "Nudges" for follow-up, Templates for canned replies,
opt-in read-receipt requests (sender-visible, recipient-approved, not
silent tracking). The bar Postio's search and keyboard work are implicitly
measured against, and the reason free-and-ad-supported is not a bar Postio
is trying to clear — it's a different trade entirely.

**Thunderbird / Betterbird.** Open source, cross-platform, any protocol,
genuinely local storage. The closest thing to "what Postio would be if it
were a 20-year-old C++/XUL codebase" — full-featured, but keyboard support
is largely addon-dependent rather than designed in, and there's no
generated single source of truth for shortcuts, palette, and menu the way
`postio-core::registry` provides. Betterbird is a fork focused on power-user
features and faster fixes; same architecture, same gap.

**Aerion.** The real peer. GTK, Linux-native, vim-style keyboard
navigation, resource-light (explicitly optimized against Electron-weight
clients), CASA Tier 2 certified (2025-04-25), Google- and Microsoft-verified
OAuth apps (2026), CardDAV contacts in alpha, Ollama-based AI composition
named as a future direction but not yet shipped.
[GitHub](https://github.com/hkdb/aerion). Worth periodic re-checking — it is
moving in almost exactly the same direction as Postio's own roadmap
(#1 OAuth+multi-provider, #4 contacts, #7 local-model AI), just from a
different starting codebase.

---

## Sources

- [Fastio — Superhuman AI Review 2026](https://fast.io/resources/superhuman-ai-review-2026/)
- [Gmelius — Superhuman AI Review](https://gmelius.com/blog/superhuman-ai-review)
- [ventureburn.com — Superhuman Email Review 2026](https://ventureburn.com/superhuman-email-review/)
- [Morgen — Superhuman Pricing & Features](https://www.morgen.so/blog-posts/superhuman-pricing)
- [usecarly.com — Superhuman Pricing in 2026](https://www.usecarly.com/blog/superhuman-pricing/)
- [getmailbird.com — Best Superhuman Alternatives for macOS](https://www.getmailbird.com/best-superhuman-alternatives-macos/)
- [Superhuman — Privacy Policy](https://superhuman.com/legal/privacy-policy)
- [Superhuman — Agents Privacy and Security Overview](https://help.superhuman.com/hc/en-us/articles/46242109586061-Superhuman-Agents-Privacy-and-Security-Overview)
- [Aerion — GitHub](https://github.com/hkdb/aerion)

## Note on provenance

This document did not exist anywhere in the repository or the local
filesystem before 2026-08-24, despite being cited by name in six open
issues. It was reconstructed from those citations plus fresh research
rather than recovered, because there was nothing to recover. If a fuller
original exists somewhere the maintainer has access to and this version
should be reconciled against it rather than replacing it, say so.
