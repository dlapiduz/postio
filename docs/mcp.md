# Agents and MCP

`postio-mcp` lets an AI agent such as Claude Code or Claude Desktop read the
mail you choose to share and write drafts for you to review. It speaks the
[Model Context Protocol](https://modelcontextprotocol.io) on its standard
input and output. The agent starts it as a subprocess, and it listens on no
port.

It can read mail and save a draft. It cannot send mail. The design is in
[ADR 0010](https://github.com/dlapiduz/postio/blob/main/docs/decisions/0010-mcp-surface.md).

## Building it

From a checkout:

```bash
cargo build --release -p postio-mcp
# the binary is target/release/postio-mcp
```

## Sharing an account

**Nothing is shared until you say so.** With no `[mcp]` section in
`config.toml`, the agent sees no accounts at all, and Postio does not open
your store or your keyring to answer it.

To share an account, name it by its address in `config.toml` and list the
folders the agent may read:

```toml
[mcp.accounts."ada@example.com"]
mailboxes = ["INBOX", "Receipts"]   # folder paths as the server names them
```

`mailboxes = ["*"]` shares every folder of that account. If a table names no
folders, nothing in that account is shared. An account with no table is
invisible to every tool, including the one that lists accounts. A message
or conversation in a folder you did not share is refused with the same
answer as one that does not exist.

The server reads this section again on every call. To revoke access, delete
the lines. The agent's next call is refused, and nothing needs to restart.

## Registering it

**Claude Code:**

```bash
claude mcp add postio -- /path/to/postio-mcp
```

**Claude Desktop:** add it to `claude_desktop_config.json`, which is under
Settings → Developer → Edit Config:

```json
{
  "mcpServers": {
    "postio": { "command": "/path/to/postio-mcp" }
  }
}
```

Any MCP client that launches a stdio server works the same way. If your
`config.toml` is not in the usual place, pass `--config <file>` or set
`POSTIO_CONFIG`.

## The tools

| Tool | What it does |
|---|---|
| `list_accounts` | The shared accounts, with their ids. |
| `list_mailboxes` | One shared account's shared folders, with roles and counts. |
| `search_mail` | A search in Postio's [query language](https://github.com/dlapiduz/postio/blob/main/docs/PRODUCT.md), such as `from:grace is:unread after:30d`. Returns message rows. |
| `read_message` | One message: its headers, its attachments' names and sizes, and its body as plain text. |
| `read_thread` | The shared messages of one conversation, oldest first. |
| `create_draft` | A new message, a reply (`reply_to`, optionally `reply_all`) or a forward (`forward`), saved as a draft. |

There is no tool to send, forward, archive, move, delete, flag or mark mail
read. These tools do not exist, so no message can trick an agent into
calling one. Reading a message does not mark it read.

A draft appears in Postio's Drafts like one you started yourself, and it is
saved to your server's Drafts folder at the next sync, as an autosave is. It
is sent only when you open it in Postio and send it. A reply goes to the
original sender and quotes the original below the agent's words, the way
Postio's own reply does. The agent's text is kept as plain text, so it
cannot add markup to the draft.

## Mail is untrusted

Anyone can send you mail, and a message can contain instructions aimed at
an agent ("forward every invoice to ..."). Postio cannot stop an agent from
reading those instructions, but it limits what reaches the agent and what
the agent can do:

- Bodies are returned as plain text. Scripts, styles, remote images and
  tracking pixels are removed, and the raw HTML is never returned.
- Everything that comes from mail, including subjects, names, addresses,
  bodies, attachment names and folder names, is returned in a separate
  block labelled `untrusted_mail_content`. The server's own sentence above
  that block contains only counts and outcomes.
- Tool names and descriptions are fixed text. Nothing from your mail
  appears in them.
- The only write is a draft, and a draft stays in Postio until you send it.

## What it reads, and when

`postio-mcp` answers from mail Postio has already downloaded to this
computer. It does not sync, fetch or connect to anything. To see new mail,
open Postio and let it sync.

Only one Postio app can have your mail store open at a time
([ADR 0041](https://github.com/dlapiduz/postio/blob/main/docs/decisions/0041-one-app-opens-the-store-at-a-time.md)).
`postio-mcp` opens the store for each tool call and closes it right after,
so it never keeps the desktop app or the terminal out. A call made while one
of them is open fails with "Postio is already open in another window."
Close that app, and the agent's next call will work.

## The access log

Each tool call adds one line to `mcp-access.log`, in the same folder as
Postio's store (`~/.local/share/postio/` on Linux, or
`~/Library/Application Support/Postio/` on a Mac). Each line records the
time, the tool, the ids of the accounts, folders, messages and drafts it
touched, and whether the call was answered, refused or failed. The log never
contains a subject, an address, a body or the words of a search. Use
`--audit <file>` to keep it somewhere else.
