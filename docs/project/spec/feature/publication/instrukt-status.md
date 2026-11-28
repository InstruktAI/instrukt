---
description: "`instrukt status` reports in one read where this computer and this repository stand with instrukt: the machine's configuration, the selected runtime adapter and its capabilities, the features that makes available, the modules, this repository's adoption and whether its code is on GitHub, the published domains and their owners, and the other adopted projects. It changes nothing."
---
# instrukt Status — Spec

## What it is

`instrukt status` answers, in one read, the questions an agent or developer
asks before changing anything about instrukt on a computer: is it configured,
which runtime does it call and what can that runtime carry, which modules are
on, is this repository adopted, which domains are published and by whom, and
which other projects publish here. The command inspects current state: it
takes no publication lock and starts no agent session. When general doctrine
is published, it makes one bounded configured-instance read through the
selected adapter. T3 may issue and revoke its existing short-lived read-scoped
credential for that read; no Instrukt file or persistent inventory is written.

The instrukt cockpit skill
(`project/instrukt/spec/feature/distribution/agent-skill`) opens
every visit with it.

## Canonical fields

### Invocation

- `instrukt status` prints the report as text; `instrukt status --json`
  prints the same report as one JSON object with one member per section
  below. The JSON member names and value forms are a published contract:
  a later version only adds members.
- It runs anywhere. Outside a git repository the repository section says so;
  on a computer without a global layer the machine section says so. Neither
  is an error.
- `instrukt status -h` says what the report is for and when to read it.

### Sections

| Section | Reports |
| --- | --- |
| `instrukt` | the running binary's version and the path it runs from |
| `machine` | whether the global layer `~/.instrukt/instrukt.yaml` exists, its `schema_version`, and its validation findings (`project/instrukt/spec/feature/configuration/layered-configuration`) |
| `runtime` | the selected runtime and whether `runner.runtime` chose it or it is the compiled default; the compiled adapter's capabilities and recognised caller identity when available; no verb templates or entry list; `agent_instructions` reports current per-instance native instruction targets and limitations |
| `modules` | each module (`blueprint`, `todos`, `events`, `history`; history defaults on) on or off in the effective configuration and in which layer that was decided; per runtime-dependent feature, whether the selected adapter supports its required capabilities, naming each missing one or the unknown runtime name |
| `backup` | runner's local-only home backup standing rendered verbatim from runner's standing query (`project/instrukt/spec/feature/runner/home-backup`) |
| `repository` | outside a repository: none. Inside one: the project layer's presence and `project` slug, whether instrukt's hook sections are installed (`instrukt hooks status`), whether its code is on GitHub (the GitHub repository it resolves to, or the remote that is not one; `project/instrukt/spec/feature/work/github-record`), the positions the working tree holds (`docs/general/`, each `docs/domain/<x>/`, `docs/project/`), the commit last published for it and whether its main is ahead of that commit |
| `published` | each published position on the computer (`general` and each domain): its owner project, the owner's checkout path, the commit published, and for a domain its `index.yaml` description or that it has none |
| `projects` | every other project publication state records: its slug, checkout path and published commit |

- The repository is **adopted** when its project layer names a `project` and
  its hook sections are installed; the report states adopted, partly adopted
  (naming what is missing, as `instrukt init` would complete it) or not
  adopted.
- For a position the repository holds that another project owns, the report
  names that owner: the conflict the repository's first sync would be
  refused on (`project/instrukt/spec/feature/repository/project-initialization`).
<!-- planned-change:thin-live-runtime-boundary -->
- Which capabilities a feature needs is the runtime adapter's contract
  (`project/instrukt/spec/feature/runner/session-continuity`); status
  reports that judgment and adds none of its own.
<!-- change:thin-live-runtime-boundary -->
- Which capabilities a feature needs is declared by each consuming context
  (`project/instrukt/spec/feature/runner/session-continuity`,
  `project/instrukt/spec/feature/company/conversations`) in the capability
  vocabulary of `project/instrukt/spec/feature/runtime/adapter`; status
  reports that judgment and adds none of its own.
<!-- /planned-change:thin-live-runtime-boundary -->

### Current instruction targets

`runtime.agent_instructions` reuses publication's target resolver. Without a
published general position it is `not_applicable`, with no provider query.
Otherwise it reports each enabled instance's runtime, instance, optional
registry identity, native mechanism and target where established. Outcomes are
`file_present`, `file_missing`, or `not_delivered` with
`mechanism_unestablished`, `home_unknown`, `native_override` or target conflict.
Defaulted homes carry their default-home limitation. Relocated Claude/fx output
reports `instruction_relocation_unverified`; the default Antigravity output
reports `runtime_home_differs`, separately from file availability. A present file establishes
only current path availability, never running-session receipt or current bytes.
An unavailable adapter reports `runtime_unreachable` and its cause rather than
an empty configured list. These answers do not fail status or unrelated sections.

### Backup status

The report carries an additive `backup` section in both text output and
`instrukt status --json`. It consumes runner's local-only, lock-free backup
standing query (`project/instrukt/spec/feature/runner/home-backup`), adds no
staleness or backup-state judgment of its own, reads no private runner state
file directly, and never contacts remote object storage or acquires backup
locks.

- It renders runner's standing payload verbatim without re-deriving fields or
  adding status-side judgment, and reports any unreadable-state error from
  runner's query in the section's `errors` list with path and reason.

### Reading and failure

- Except for the bounded instruction-target query, sections use local state
  only: the configuration layers, publication state, runner's local-only backup
  standing query, the repository's hook sections and its git remotes. Nothing
  is re-derived from other repositories, and neither GitHub nor remote backup
  storage is contacted.
- A part that cannot be read (an unparsable project layer, unreadable
  publication state, or unreadable backup standing reported by runner's query)
  is reported in its section with the path and the reason; the other sections
  are still reported, and the exit status is non-zero. A machine layer that
  cannot be loaded stops status as it stops every command
  (`project/instrukt/spec/feature/configuration/layered-configuration`).
- With every part read, the exit status is zero whatever the report says:
  an unconfigured computer, an unconfigured or never-run backup, or an
  unadopted repository is an answer, not a failure.

## Use cases

### UC-IS1: An agent learns where this computer and repository stand with instrukt in one read

```gherkin
Given a developer's computer is configured with a runtime, has the todos module on and publishes always-on doctrine and domains from two adopted projects
And the developer works in one of those projects
When an agent acting for them reads instrukt's status
Then it learns in one answer the instrukt version, that the machine is configured, which runtime adapter is selected and which capabilities it supports
And which modules are on and what the runtime lets them do
And that this repository is adopted, what it publishes and whether its last commit is published
And every published domain with its owner, and the other adopted project
And it sees current instruction-target availability and explicit non-delivery reasons for enabled instances
And no Instrukt file has changed
```

### UC-IS2: A developer on a fresh computer sees that nothing is set up yet

```gherkin
Given a developer has just installed instrukt on a computer with no instrukt configuration
And they stand in a git repository instrukt has never seen
When they read instrukt's status
Then it reports that the machine is not configured, that the runtime is the compiled default, that history defaults on, the other modules are off and that nothing is published
And that this repository is not adopted
And the command succeeds
```

### UC-IS3: A developer sees which modules their runtime can carry before switching one on

```gherkin
Given a developer's runtime adapter supports standing agents but lacks anomaly notification
When they read instrukt's status
Then the report shows standing agents available on that runtime
And shows anomaly notification unavailable, naming the missing capability
```

### UC-IS4: A developer sees that a repository would claim a domain another project owns

```gherkin
Given a developer's computer publishes a domain owned by one project
And they stand in another repository whose working tree holds a domain of the same name
When they read instrukt's status
Then the report names the project that owns that domain
And shows that this repository's publication of it would be refused
```

### UC-IS5: See home backup standing in status without contacting remote storage

```gherkin
Given an operator's computer has home backup configured and runner supplies a local backup standing answer
When the operator or an agent reads instrukt's status in text or JSON form
Then the report includes the additive backup section showing runner's standing, last timestamp and published generation without adding status-side judgment
And no remote object storage call is made and no home file is modified
But when home backup is unconfigured or has not yet run
Then the backup section reports that state clearly and status still succeeds
```

## Known caveats

- A repository adopted with `instrukt init` but never synced is unknown to
  publication state: it appears in `projects` only after its first sync.
  Inside that repository the repository section still reports it.
- Status reports configuration as written; declared capabilities do not establish runtime reachability. Operational
  failures are learned when the adapter is called.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md)
<!-- planned:thin-live-runtime-boundary -->
- [project/instrukt/spec/feature/runtime/adapter](../runtime/adapter.md)
<!-- /planned:thin-live-runtime-boundary -->

- [project/instrukt/spec/feature/distribution/agent-skill](../distribution/agent-skill.md)

- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/repository/project-initialization](../repository/project-initialization.md)
