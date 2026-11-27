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

A company's home is generated. An operator declares once, for the computer,
the GitHub organisation that owns company homes; from then on an agent asked
to add a company creates the company's private repository in that
organisation with the one company skeleton every company receives, makes the
company's GitHub administrator an administrator of it, and checks it out as the
company's home. A company whose repository already exists there is one by
contract: adding it to another computer checks that repository out as its home
and changes nothing on GitHub. Nothing about a company is customised: two
companies' homes differ only where each company's identity enters them. A
company's repository is also a project like any other adopted on the computer,
one whose documentation is the company's knowledge alone, so everything an
adopted project receives works for it from the moment its home exists. Where
the runtime is T3, the home's runtime project is created titled with the
repository's name, `company-<name>`, while the folder keeps the company's name
(`project/instrukt/spec/feature/runner/t3-contract`).

## Canonical fields

### The home

| Part | Where | Contract |
| --- | --- | --- |
| Home folder | `~/.instrukt/companies/<name>/` | `<name>` is the company's identity on this computer and follows the grammar of a project slug. |
| Settings document | `company.yaml` in the home folder | Carries `schema_version`, `company: <name>` equal to the folder name, and the company-home keys of `project/instrukt/spec/feature/configuration/layered-configuration`. |
| Project layer | `instrukt.yaml` at the root of the home's repository, committed | Part of the company skeleton (Generation): the home's repository is an adopted project like any other, named `company-<name>`, declaring itself a company's repository and the company identity with `company_name` (`project/instrukt/spec/domain/configuration`, The company identity), with todos off and automatic review on, under the project layer's rules (`project/instrukt/spec/feature/configuration/layered-configuration`). |
| Secrets file | `secrets.env` in the home folder | The company's own protected secrets, under the protection, write and disclosure rules of `project/instrukt/spec/feature/configuration/owned-files`. |
| Company knowledge | The company's documents in the home's `docs/` folder, and the index `docs/index.yaml` that selects from them | Optional. What every agent working for the company starts with, rendered into the home's committed `AGENTS.md` and, on request, into the company's skill, which also holds a document's `visibility` to its rules. It references only the company's own documents (`project/instrukt/spec/feature/publication/agent-entrypoints`, `project/instrukt/spec/feature/publication/company-plugin`), and it is never published to the computer's documentation (`project/instrukt/spec/feature/publication/project-publication`). |
| Ignore file | `.gitignore` in the home folder | Keeps the settings document, the secrets file, `projects/` and the local state instrukt keeps in the home out of the repository. |

- A folder under `companies/` without a valid settings document is not a
  company: nothing is served or run for it, and validation names the folder
  and the finding. A settings document whose `company` differs from its folder
  name is such a finding.
- The settings document is read on its own. It is no layer of a project's
  effective configuration, and no project layer selects a company's settings
  document or home.
- What the settings document declares follows the machine's global layer's
  module switches; the settings document carries no module switches. The
  home's project layer switches the repository's modules as every project
  layer does.
- **The settings document has its own name.** A home whose settings are still
  the root `instrukt.yaml` declaring `company` has that file converted to
  `company.yaml` with every setting it holds, as every owned shape is
  converted (`project/instrukt/spec/feature/configuration/owned-files`). Where
  the home holds both, neither is changed and validation names both.
- instrukt never deletes a company home.



### Generation

| Part | Contract |
| --- | --- |
| Owning organisation | `github.org` in the machine's global layer names the GitHub organisation that owns every company home repository on this computer. instrukt names no owner of its own. |
| Repository | `company-<name>` in the owning organisation, private, where `<name>` is the company's key. |
| Home | The repository's checkout at `~/.instrukt/companies/<name>/`. |
| Administrator | The company's GitHub administrator, named when the repository is created, is made an administrator of it. |
| Identity | The company's key, the name people write the company by, and its GitHub administrator. Generation takes nothing else. The key and the name enter the skeleton's project layer as the company identity (`project/instrukt/spec/domain/configuration`, The company identity). |

- **The repository by contract.** Where `company-<name>` does not exist in
  the owning organisation, it is created with the skeleton and the
  administrator. Where it exists, it is a company home repository by
  contract: its content is not inspected, nothing on GitHub is changed, and it
  is checked out as the home.
- **An existing home is never overwritten.** Where the home folder exists,
  adding the company is refused, naming the folder; nothing in it, and nothing
  on GitHub, is changed.

- **One skeleton.** The repository is created holding the company skeleton:
  the home's committed project layer and the ignore file of this spec (The
  home), the company's knowledge index with its documents folder, the
  committed `AGENTS.md` rendered from them
  (`project/instrukt/spec/feature/publication/agent-entrypoints`), and the
  company's skill with the catalog that lists it, rendered from them
  (`project/instrukt/spec/feature/publication/company-plugin`). The skeleton
  comes from one source and is the same for every company except where the
  company's identity enters it. It holds nothing that writes the repository's
  default branch on its own: that branch has one writer
  (`general/policy/commits`).
- **Ready when it ends.** Whether the repository was created or checked out
  by contract, the home holds a valid settings document naming the company
  and instrukt's hook sections, and the computer has published it as the
  project its project layer names. It is then a company on the computer and an
  adopted project like any other: a commit made in it carries its rendered
  `AGENTS.md`, and its changes are integrated as every adopted project's
  are. A computer that meets automatic review's prerequisites admits it for
  the home out of the box; on one that does not, the home is added all the
  same, automatic review is withheld there and reported as every project
  layer's is (`project/instrukt/spec/feature/configuration/layered-configuration`,
  Automatic review enablement;
  `project/instrukt/spec/feature/runner/pull-request-review`).
- **The company's knowledge stays the company's.** Publishing the home
  publishes none of the company's documents to the computer's documentation,
  so no agent working elsewhere on the computer reads them
  (`project/instrukt/spec/feature/publication/project-publication`).
- **From anywhere.** The operator, and an agent session acting for the
  operator whose role grants adding a company, add one with the same result
  from any working directory on the computer, a company's home included; the
  working directory alone refuses no one. A session's role and a session's
  binding to a company conversation still decide what it may do
  (`project/instrukt/spec/feature/configuration/roles-and-capabilities`,
  `project/instrukt/spec/feature/configuration/configuration-editing`).
- **The organisation is a prerequisite where it is used.** `github.org` is
  optional: a computer that adds no companies is valid without it. Setting it
  to a value that names no GitHub organisation — a personal account, or an
  empty value — is refused, naming the key and the prerequisite, and the
  settings stay as they were. Adding a company on a computer whose `github.org`
  is not declared, or names no organisation, is refused, naming the key and the
  prerequisite; no repository and no home is created.

### What stays out of the repository

The home also holds what belongs to one computer and is never committed: the
secrets file, the `projects/` folder with the checkouts of other
repositories, and the local state instrukt keeps in the home. The settings
document stays out of the repository too: it holds the members' addresses
and the company's live settings, and it is kept on the computer.

- `instrukt config init --company <name>` writes the home's ignore lines for
  them, appending each missing line and keeping every other line, also in a
  home that already exists. A `.gitignore` that is a symbolic link is
  refused, naming it, before anything is read or written; it is replaced by
  the file itself in the company's repository, and the command run again.
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

### What a company's connector needs

A connector a company declares is admitted on a computer only with what it needs
there. A missing prerequisite withholds the connector and is reported to the
operator. `instrukt config validate --company <name>` reports local
prerequisites. `instrukt events status` reports the runner's
admission findings, including provider permission checks
(`sdlc/principle/design-fundamentals`, Fail Fast: prerequisites are checked at
entry rather than discovered by the running runner):

- a valid `role` (default `company-agent`) that is configured in `agents.yaml`
  or declared by a delivered published procedure on that computer, together with
  a delivered published procedure declaring that role (or declaring
  `company-agent` when the role is configured in `agents.yaml` without a
  dedicated procedure of its own) — publishing it is part of setting the company
  up;
- a value in the home's secrets file for each secret the declaration references,
  not blank;
- at admission, the provider permissions the connector requires, as the provider
  reports them for the connector's own token
  (`project/instrukt/spec/feature/company/slack-connector`).

A token the provider refuses after admission is the provider's availability, not
a missing prerequisite: the connector retries it and reports it to the operator.

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

### UC-CH5: Add a company whose home is generated the same as every other

```gherkin
Given an operator has declared the GitHub organisation that owns company homes on their computer
When their agent adds a new company, given its key, its name and its GitHub administrator
Then the organisation holds the company's private repository, named after its key, holding the company skeleton
And the company's GitHub administrator is an administrator of that repository
And the computer holds the company's home as the checkout of that repository, ready as a project like any other with no step taken by hand
And the company's home holds the same skeleton as every other company added this way
But when no organisation is declared, or the declared owner is a personal account
Then no repository and no home is created, and the operator is told the organisation is a prerequisite
But when the company already has a home on the computer
Then nothing of it is changed, and the operator is told the home exists
```

### UC-CH6: Give another computer an existing company's home

```gherkin
Given a company's repository exists in the organisation that owns company homes
And the operator's computer holds no home for that company
When their agent adds the company by its key
Then the computer holds the company's home as the checkout of that repository, ready as a project like any other with no step taken by hand
And nothing of the repository on GitHub is changed
```

## Known caveats

- A company home publishes no documentation of its own. An agent working
  there starts with the global agent files, the home's rendered `AGENTS.md`
  and the assignment it is given.
- instrukt reads a company home as it stands. A company's repository is
  created, and its home checked out, when the company is added (Generation);
  from then on the home is kept current as every adopted project's checkout
  is.
- A home folder that exists is never adopted: a home made by hand, or one that
  is not a checkout, stays as it is.
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
