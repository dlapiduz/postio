Postio — Product Specification
1. Product vision

Postio is a fast, beautiful, keyboard-first desktop email client designed for people with large and complex inboxes.

It should combine:

the speed and keyboard efficiency of a terminal client
the polish and visual quality of a modern native desktop application
the search capabilities of a modern information-retrieval system
the intelligence of an AI assistant
the reliability of a traditional email client
a local-first architecture so the application remains responsive regardless of network conditions
Design principles
Instant
Opening the app should feel immediate.
Navigation should never wait for the network.
Search should be local and effectively instantaneous.
Keyboard first
Every important operation should have a keyboard shortcut.
Mouse/touch should remain excellent, but never be required.
Search first
Search is a primary navigation mechanism, not an afterthought.
Users should be able to find messages using natural language as well as traditional operators.
Local first
Mail is synchronized locally.
The UI operates primarily against the local database.
Network operations happen asynchronously.
Native
Postio should feel like a real desktop application.
Linux should use GTK4/libadwaita initially.
Avoid the visual/behavioral feel of an embedded website.
AI-native
AI should help users understand, find, write, and act on email.
AI should be integrated into workflows rather than existing as a chatbot sidebar.
Predictable
Destructive operations require appropriate confirmation/undo.
Synchronization state is visible.
The user should always understand what happened.
2. Target platforms
Phase 1

Linux

GTK4
libadwaita
Wayland first
X11 supported where practical
Phase 2

macOS

Native macOS frontend using the same Rust core.

Phase 3

Windows

Native Windows frontend using the same core.

The application architecture should deliberately allow:

                   postio-core
                       │
          ┌────────────┼────────────┐
          │            │            │
        GTK          SwiftUI      WinUI
       Linux         macOS       Windows

The UI should never contain email synchronization or protocol logic.

3. Account support

Postio should support multiple accounts.

Initial protocols
IMAP
SMTP
Authentication
username/password
OAuth 2
OAuth 2 refresh tokens
app passwords where supported
TLS
STARTTLS where required
Provider presets

Initial presets:

generic IMAP/SMTP

The provider configuration should be extensible rather than hard-coded.

Future
JMAP
Gmail API
Microsoft Graph
Exchange
Gmail
Google Workspace
Microsoft 365
Outlook.com
Fastmail

The architecture should allow multiple protocols to expose the same Postio domain model.

4. Core mail model

Postio should have its own domain model independent of IMAP/JMAP.

Core entities:

Account
Mailbox
Message
Thread
Attachment
Contact
Label
Flag
Draft
Identity
Search
Rule
Message

A message should contain:

ID
account
mailbox
thread
Message-ID
In-Reply-To
References
sender
recipients
CC
BCC
subject
date
received date
text body
HTML body
attachments
flags
labels
size
headers
server identifiers
local synchronization state
5. Threading

Threading should be a first-class local concept.

Postio should not rely entirely on server-provided threading.

Thread reconstruction should use:

Message-ID
In-Reply-To
References
subject normalization
server threading information where available

The UI should present conversations as threads.

Users should be able to:

expand/collapse messages
jump between messages
show only unread messages
show chronological or reverse-chronological order
open an individual message
expand quoted content
collapse quoted content
6. Local storage

SQLite should be the primary local database.

Possible structure:

SQLite
│
├── accounts
├── identities
├── mailboxes
├── messages
├── threads
├── recipients
├── attachments
├── labels
├── message_labels
├── drafts
├── sync_state
└── settings
Search

Use a dedicated search index.

Initial candidate:

SQLite FTS5

Potentially later:

Tantivy
SQLite FTS5 + custom ranking
hybrid lexical/vector search

Search should cover:

sender
recipient
subject
body
attachment filename
labels
mailbox
date
thread
7. Search

Search should be one of Postio's defining features.

Traditional search

Support operators such as:

from:alice@example.com
to:bob@example.com
subject:invoice
has:attachment
is:unread
is:starred
after:2026-01-01
before:2026-02-01
in:archive

Operators should be composable:

from:alice after:2026-01-01 has:attachment
Natural-language search

Users should eventually be able to type:

invoices from Alice from last quarter

or:

conversations about the Kubernetes migration

Postio should translate this into a structured search query and/or semantic search.

Search UI

/ should open search immediately.

Search results should appear while typing.

Results should show:

sender
subject
date
mailbox
snippet
relevance
attachment indicator
unread state
8. Keyboard interaction

Keyboard navigation should be a core design system, not a collection of shortcuts.

Navigation
j / ↓       next message
k / ↑       previous message
h / ←       previous view
l / →       open
gg          first
G           last
Message actions
r           reply
R           reply all
f           forward
a           archive
d           delete
e           archive
s           star
u           mark unread
m           move
c           compose
Search
/           search
Esc         exit search
Enter       open result
Composition
Ctrl/Cmd+Enter   send
Ctrl/Cmd+S       save draft
Esc              close composer
Command palette

A universal command palette:

Ctrl/Cmd+K

Examples:

Archive message
Move to...
Mark unread
Add label...
Reply
Forward
Summarize
Find related messages
Snooze

Every command should have:

keyboard shortcut
command-palette entry
accessible UI action
9. Navigation model

The UI should avoid the traditional giant folder/sidebar consuming the entire screen.

Conceptually:

┌────────────────────────────────────────────────────┐
│ Search                                        User │
├────────────┬───────────────────────┬───────────────┤
│            │                       │               │
│ Inbox      │   Message list       │   Message     │
│ Starred    │                       │               │
│ Drafts     │   ○ Alice            │   Alice       │
│ Sent       │   ● Bob              │   Re: ...     │
│ Archive    │   ○ Carol            │               │
│            │                       │               │
│ Accounts   │                       │               │
└────────────┴───────────────────────┴───────────────┘

But the UI should be adaptable:

Three-pane mode

For desktop monitors.

Two-pane mode

For laptops.

Message-focused mode

For reading/writing.

Search-focused mode

Search results take over the main workspace.

10. Compose

Composer should feel extremely fast.

Features:

rich text
plaintext
Markdown-like composition internally if desired
formatting
attachments
drag/drop attachments
inline images
signatures
multiple identities
recipients autocomplete
CC/BCC
reply/forward
draft autosave
Recipient handling

Typing:

diego@

should immediately search local contacts and previous recipients.

11. Attachments

Attachments should be locally indexed.

Support:

download
open
save as
preview
inline images
PDF preview
common document formats where practical

Search should include:

has:attachment
filename:contract
type:pdf
12. AI

AI should be deeply integrated but optional.

Understand

For any message/thread:

summarize
summarize entire conversation
identify action items
identify decisions
extract dates
extract people
extract commitments
identify questions awaiting response

Example:

Thread summary

Decision
• Launch moved to September 14.

Action items
• Diego → review migration plan
• Alice → update documentation

Open question
• Who owns production deployment?
Search

AI-powered semantic search:

"Find the conversation where we discussed delaying the launch."

Write
draft reply
shorten
expand
make more professional
make more casual
summarize before replying
translate
fix grammar
Triage

AI can identify:

newsletters
automated mail
likely important messages
messages requiring response
action items

But AI must never silently modify or send mail.

All externally visible actions require explicit user confirmation.

13. AI architecture

AI should be a separate subsystem:

                  postio-core
                       │
                  postio-ai
                       │
        ┌──────────────┼──────────────┐
        │              │              │
      OpenAI         Ollama         Other

Support both:

cloud models
local models

The user should control:

which provider is used
what data can leave the machine
whether message content can be sent to AI
per-account/per-feature permissions
14. Synchronization

Synchronization should be invisible most of the time.

The application should maintain:

Local state
     ↕
Sync engine
     ↕
Remote server

Requirements:

incremental sync
reconnect automatically
exponential backoff
IMAP IDLE
QRESYNC/CONDSTORE where available
efficient mailbox discovery
attachment lazy downloading
cancellation
progress reporting
offline operation
Important principle

The UI should never block waiting for synchronization.

15. Offline mode

Postio should be fully usable offline after initial synchronization.

Users should be able to:

read messages
search
compose
reply
forward
archive
delete
move
label
mark read/unread

Operations performed offline should enter a local operation queue.

User action
     ↓
Local database
     ↓
Operation queue
     ↓
Sync engine
     ↓
Server
16. Undo

Destructive operations should be undoable.

For example:

Archived 12 messages — Undo

Undo should work locally immediately, while the sync engine reconciles the remote state.

17. Notifications

Desktop notifications should support:

new mail
important mail
mentions/requests
AI-detected action-required mail

Notifications should be configurable per account and mailbox.

18. Performance requirements

This should be treated almost like a performance budget.

Startup

Target:

<500 ms to usable UI on a modern machine with an existing local database.

Navigation

Target:

<16 ms for ordinary UI interactions where possible.

Search

Target:

<100 ms for ordinary local searches.

Opening mail

Should not require network access if the message is synchronized.

Memory

The application should avoid the Electron-style architecture of multiple Chromium processes.

Large mailboxes should not require loading the entire mailbox into memory.

19. Visual design

I'd make this a beautiful native application, not a terminal-inspired application.

Think:

Arc + Linear + modern GNOME + Apple Mail, but with much more keyboard efficiency.

Principles:

generous typography
excellent spacing
subtle hierarchy
restrained use of color
excellent dark mode
excellent light mode
minimal chrome
strong typography
smooth transitions
extremely good message rendering
Important

Don't make it look like:

"GTK developer made an email client."

It should look like a premium application that happens to be built with GTK.

20. Accessibility

First-class:

keyboard navigation
screen readers
high contrast
reduced motion
scalable text
focus indicators
semantic controls
21. Security

Email contains extremely sensitive information.

Requirements:

credentials stored using OS credential/keychain facilities
OAuth tokens never stored plaintext
TLS required where possible
secure attachment handling
HTML sanitization
remote image blocking by default
tracking pixel protection
external content controls
phishing/link warnings
AI data-sharing controls

Potential future:

PGP
S/MIME
22. Architecture

I'd recommend:

                     ┌──────────────────┐
                     │   Native UI      │
                     │ GTK/libadwaita   │
                     └────────┬─────────┘
                              │
                     ┌────────▼─────────┐
                     │   postio-core    │
                     │                  │
                     │ Domain model     │
                     │ Search           │
                     │ Commands         │
                     │ State            │
                     └────────┬─────────┘
                              │
            ┌─────────────────┼─────────────────┐
            │                 │                 │
      ┌─────▼─────┐     ┌─────▼─────┐    ┌─────▼─────┐
      │ Sync      │     │ Storage   │    │ AI        │
      │           │     │           │    │            │
      │ IMAP      │     │ SQLite    │    │ Providers │
      │ JMAP      │     │ FTS5      │    │ Local LLM │
      │ SMTP      │     │ blobs     │    │ Cloud     │
      └─────┬─────┘     └───────────┘    └────────────┘
            │
      Pimalaya libraries
Rust crates

Potential structure:

postio/
├── crates/
│   ├── postio-core
│   ├── postio-model
│   ├── postio-storage
│   ├── postio-search
│   ├── postio-sync
│   ├── postio-imap
│   ├── postio-smtp
│   ├── postio-ai
│   └── postio-gtk
│
└── tests/
23. MVP

Despite the full vision, don't build everything first.

The first genuinely useful Postio should do only this:

Accounts
 Gmail/IMAP account
 OAuth
 SMTP
Mail
 Inbox
 folders
 threads
 read/unread
 archive
 delete
 star
 move
Reading
 HTML
 plaintext
 attachments
 quoted-message collapsing
Composition
 compose
 reply
 reply all
 forward
 attachments
 drafts
Search
 local FTS5
 Gmail-style operators
 instant / search
Keyboard
 Vim-style navigation
 command palette
 configurable shortcuts
Architecture
 SQLite
 background sync
 offline reading
 undo

No AI yet.

Get the core mail experience really good first.

24. V1

Then add:

multiple accounts
unified inbox
labels
snooze
scheduled send
rules/filters
contacts
advanced search
notifications
semantic search
AI summaries
AI reply assistance
AI action-item extraction
local AI support
25. The killer feature

I think Postio should ultimately have a workflow that looks something like:

/

> invoices from Acme that I haven't responded to

Postio finds the relevant messages.

Then:

Enter

opens the conversation.

a

archives it.

Or:

Ctrl+K → Summarize

Acme is waiting for approval of the $42,300 invoice.
They sent the revised invoice on August 19.
You have not responded.

Then:

r

opens the reply composer.

AI can suggest:

"Thanks, I've reviewed the revised invoice..."

You edit it and:

Ctrl+Enter

sends.

That entire workflow should be possible without reaching for the mouse.

That's what would make Postio more than another Thunderbird alternative: the application is fundamentally designed around navigating and acting on a huge information stream quickly.

The north star

I'd put this at the top of the project README:

Postio is a local-first, keyboard-first email client built for people who have too much email.

Read less. Find anything. Act faster.

And I'd make speed, search, and keyboard interaction the three things that Postio absolutely must do better than existing email clients. Everything else—AI, beautiful UI, protocols, integrations—should reinforce those three rather than distract from them.
