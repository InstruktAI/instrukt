---
description: Company homes — a company exists on a computer as the folder `~/.instrukt/companies/<name>/`, the checkout of the company's repository, with its own settings document, its members, its knowledge and its own protected secrets kept out of the repository, addressed with `--company <name>`; no registry, no project ownership and no roles.
---
# Company Homes — Spec

## What it is

A company is a home on a computer: the place where work for that company runs
and where the company's own settings, knowledge and secrets live. The home is
the checkout of the company's repository, at the same location on a computer
and in a container, so the company's documents are in the folder its agents
work in. One computer holds any
number of companies. The folder is the registration: a company exists where
its home folder holds a valid settings document, and it is listed nowhere
else. A project belongs to no company, and a company home carries no roles and
no authorization.

## Canonical fields

### The home

| Part | Where | Contract |
| --- | --- | --- |
| Home folder | `~/.instrukt/companies/<name>/` | `<name>` is the company's identity on this computer and follows the grammar of a project slug. |
| Settings document | `instrukt.yaml` in the home folder | Carries `schema_version`, `company: <name>` equal to the folder name, and the company-home keys of `project/instrukt/spec/feature/configuration/layered-configuration`. |
| Secrets file | `secrets.env` in the home folder | The company's own protected secrets, under the protection, write and disclosure rules of `project/instrukt/spec/feature/configuration/owned-files`. |
| Company knowledge | `AGENTS.master.md` in the home folder, and the company's documents in its `docs/` folder | Optional. What every agent working for the company starts with, rendered into the home's committed `AGENTS.md`. It references only the company's own documents (`project/instrukt/spec/feature/publication/agent-entrypoints`). |
| Ignore file | `.gitignore` in the home folder | Keeps the settings document, the secrets file, `projects/` and the local state instrukt keeps in the home out of the repository. |

- A folder under `companies/` without a valid settings document is not a
  company: nothing is served or run for it, and validation names the folder
  and the finding. A settings document whose `company` differs from its folder
  name is such a finding.
- The settings document is read on its own. It is no layer of a project's
  effective configuration, and no project layer names a company.
- Module enablement for a company home follows the machine's global layer; a
  company home carries no module switches.
- instrukt never deletes a company home.

### What stays out of the repository

The home also holds what belongs to one computer and is never committed: the
secrets file, the `projects/` folder with the checkouts of other
repositories, and the local state instrukt keeps in the home. The settings
document stays out of the repository too: it holds the members' addresses
and the company's live settings, and it is kept on the computer.

- `instrukt config init --company <name>` writes the home's ignore lines for
  them, appending each missing line and keeping every other line, also in a
  home that already exists.
- `instrukt config validate --company <name>` reports as a finding, naming
  the path, a settings document, a secrets file, a `projects/` folder or
  local state that the home's repository tracks or does not ignore.

### Addressing

`instrukt config init|get|patch|validate|schema --company <name>` address a
company home's settings document with the verbs of
`project/instrukt/spec/feature/configuration/configuration-editing`.
`init --company <name>` creates the home folder and its settings document and
leaves an existing home as it is. `schema --company <name>` shows every key
this document can declare — connectors with each kind's setup, MCP servers,
members, profiles — with its contract. `instrukt secrets set|generate --company
<name>` write that home's secrets file.

### Secrets

A `${NAME}` reference in a company home's settings resolves only from that
home's `secrets.env`. The machine's secrets file, another company's file, the
process environment and any `.env` supply no fallback, so two companies use
the same reference name for different values.

### Members

```yaml
members:
  alex:
    name: Alex Example
    email: alex@example.com
    credentials:
      erpnext: ${ALEX_ERPNEXT_LOGIN}
```

A company's people reach its agent through its connectors, which know them
from their providers (`project/instrukt/spec/feature/company/conversations`).
`members` in the company home's settings holds what a provider cannot supply:
a person's own stored logins for the company's tools. A person working for two
companies is a member entry in each, and nothing links the two. A tool whose
server the company declares with sign-in needs no member entry: the person
signs in themselves, and that login is kept in the computer's state database,
never in the home (`project/instrukt/spec/feature/runner/mcp-access`).

| Field | Contract |
| --- | --- |
| `members.<member>` | The member's key, unique within the company, following the grammar of a project slug. |
| `name` | The person's display name; the key when omitted. |
| `email` | The person's email address. Required. An address is listed by at most one member of the company. |
| `credentials.<tool>` | A `${NAME}` reference into the home's secrets file: the person's own login for that tool. Never a literal value. |

- Two addresses that differ only in case are the same address. An address
  listed by two members, a missing address, a literal credential and an
  unknown field in a member entry are validation findings naming the member
  and the field.
- A member entry carries no role and grants nothing. It does not decide whom
  the agent answers; it says which logins are a person's own. How a speaker's
  entry is found is `project/instrukt/spec/feature/runner/mcp-access`.

### Agent profiles

```yaml
profiles: [sunfield-claude, sunfield-codex]
```

`profiles` is an ordered list of profile names the machine's `agents.yaml`
declares. Every session that runs in the company's home is started on that
order in place of the machine-wide one, under the selection rule of
`project/instrukt/spec/feature/configuration/agent-selection`. A company home
declares no profile, provider instance or credential of its own. A name the
machine's `agents.yaml` does not declare is a validation finding naming the
company and the profile; an empty list is a finding naming the company. A home
without `profiles` uses the machine-wide mapping.

## Use cases

### UC-CH1: Add a company to a computer

```gherkin
Given a developer runs work for companies on one computer
When they create a home for another company
Then that company's settings can be read, changed and validated by its name
And the companies already on the computer are unchanged
But a folder whose settings name a different company is reported and is not a company
```

### UC-CH2: Keep each company's secrets in its own home

```gherkin
Given a developer has two companies on one computer whose settings use the same secret name
When each company's settings are resolved
Then each resolves the value recorded in its own home
And neither the machine's secrets nor the other company's supply it
And no output discloses a value
```

### UC-CH3: Give a company's people their own tool logins

```gherkin
Given a developer lists a company's members with their email addresses and their own tool logins
When the company's settings are validated and read
Then each email address belongs to exactly one member
And each member's logins appear as references, never as values
But an address listed by two members is refused naming both
```

### UC-CH4: Keep a company's secrets out of its repository

```gherkin
Given an operator's company home is the checkout of the company's repository
When the operator sets the home up
Then the home's settings, its secrets, its other checkouts and instrukt's local state are ignored by the repository
When a commit tracks the secrets file anyway
Then validating the company reports the file
```

## Known caveats

- A company home publishes no documentation of its own. An agent working
  there starts with the global agent files, the home's rendered `AGENTS.md`
  and the assignment it is given.
- instrukt reads a company home as it stands. Obtaining the home — the first
  checkout, made by an agent at the request of someone with access to the
  company's repository — and keeping it current are the operator's, outside
  this contract.
- Company homes on one computer are separate folders of one operating-system
  user. What keeps a session working for one company from reading another
  company's home is that company's container
  (`project/instrukt/spec/feature/runner/company-containers`), not the
  folders.
- There are no person homes.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/configuration/owned-files](owned-files.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](../runner/inbound-webhook-endpoint-framework.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
