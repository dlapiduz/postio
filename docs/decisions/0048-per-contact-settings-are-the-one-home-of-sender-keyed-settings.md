# ADR 0048 — Per-contact settings are the one home of sender-keyed settings

- **Status:** Accepted (2026-10-20). Built for pins, markers, one-sender digests and the HTML treatment. `notify`, remind-after and archive-after are columns awaiting surfaces.
- **Decision:** What a person decided about one correspondent (where their mail arrives, which markers are off, how their HTML is drawn) is kept on the contact in `contact_settings`, not per address in a file. The engine reads the file and the contacts together. What is not one correspondent's stays in `config.toml`.

## Rules
- `contact_settings` has one row per correspondent who decided something: `arrival` (`inbox`, `digest`, `filtered`), digest name, cadence, day and time, marker kinds off (bitmask), `notify`, `html_treatment` (`app` or `paper`), remind and archive periods. No decision, no row.
- `FocusConfig::with_contacts` adds contact decisions as `never` entries, `stop_markers` entries and `[[focus.digests]]` rules (one `from:` query per address). Focus installs the result and `FocusSettings` republishes it whenever a verb changes a correspondent. Nothing downstream knows there are two homes.
- Writers write the person: pinning an address, turning a marker off for one, and saving a rule that is one `from:` query for one address land on the contact. Taking one back clears whichever home holds it. A pin for someone who has a digest, and a second digest for one person, stay in the file, because one `arrival` cannot be both.
- A one-shot move runs the first time Focus starts with a config file: what a contact can hold moves to them, the file is saved as an editor saves it, and the original is kept once as `config.toml.pre-contacts`. The `[Treatment]` section of the remote-image list moves the same way. A fixture files a corpus under both engines and compares folder, reason and digest per message.
- A digest keeps its rule's name for held mail and deliveries.
- Domains, `list:` or `subject:` rules and multi-sender rules stay in `config.toml`. Remote-image grants stay where they are: a privacy rule that fails closed.
- A frontend editing `config.toml` as text cannot take a person's decision back by editing the file. It asks the host.
- Merging contacts does not yet carry settings from the absorbed contact (spec 011 Q8).

## Why
- A person is not an address. Joining two addresses left each setting on the address it was written for, and a contact screen needed four places to read and write.
- Rejected: a contact-aware reader over the file (a merge still rewrites it, two edit places); moving domains and list rules (not correspondents); moving remote-image grants (a second home is the risk their module header names).

## Revisit when
- A setting must stay per address, so `arrival` needs an address scope.
