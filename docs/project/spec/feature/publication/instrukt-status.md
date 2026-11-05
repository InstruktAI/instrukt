---
description: "`instrukt status` reports in one read where this computer and this repository stand with instrukt: the machine's configuration, the runtime and which of its verbs are filled, the features that makes available, the modules, this repository's adoption and whether its code is on GitHub, the published domains and their owners, and the other adopted projects. It changes nothing."
delivered_by: [instrukt-first-build]
---
# instrukt Status — Spec

## What it is

`instrukt status` answers, in one read, the questions an agent or developer
asks before changing anything about instrukt on a computer: is it configured,
which runtime does it call and what can that runtime carry, which modules are
on, is this repository adopted, which domains are published and by whom, and
which other projects publish here. The instrukt cockpit skill
(`project/instrukt/spec/feature/distribution/claude-plugin`) opens
every visit with it. The command only reads: it writes no file, takes no
lock, reaches no network and starts no session.

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
| `runtime` | each runtime-mapping entry by name, which entry an OS-started runner uses (`runner.runtime`), the active runtime of the caller when there is one, and, per entry, each verb the runtime mapping defines, marked empty or filled with its kind (`command` or `agent`) |
| `modules` | each module (`blueprint`, `todos`, `events`) on or off in the effective configuration and in which layer that was decided; per feature the runtime mapping gates, whether the selected runtime fills every verb it needs, naming each missing one |
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
- Which verbs a feature needs is the runtime mapping's contract
  (`project/instrukt/spec/feature/runner/session-continuity`); status
  reports that judgment and adds none of its own.

### Reading and failure

- Every section is computed from what instrukt already records: the
  configuration layers, publication state, the repository's hook sections
  and its git remotes. Nothing is re-derived from other repositories, and
  GitHub is not contacted.
- A part that cannot be read (an unparsable layer, unreadable publication
  state) is reported in its section with the path and the reason; the other
  sections are still reported, and the exit status is non-zero.
- With every part read, the exit status is zero whatever the report says:
  an unconfigured computer or an unadopted repository is an answer, not a
  failure.

### Use cases

#### UC-IS1: An agent learns where this computer and repository stand with instrukt in one read

```gherkin
Given a developer's computer is configured with a runtime, has the todos module on and publishes domains from two adopted projects
And the developer works in one of those projects
When an agent acting for them reads instrukt's status
Then it learns in one answer the instrukt version, that the machine is configured, which runtime is used and which of its verbs are filled
And which modules are on and what the runtime lets them do
And that this repository is adopted, what it publishes and whether its last commit is published
And every published domain with its owner, and the other adopted project
And nothing on the computer has changed
```

#### UC-IS2: A developer on a fresh computer sees that nothing is set up yet

```gherkin
Given a developer has just installed instrukt on a computer with no instrukt configuration
And they stand in a git repository instrukt has never seen
When they read instrukt's status
Then it reports that the machine is not configured, that no runtime is mapped, that every module is off and that nothing is published
And that this repository is not adopted
And the command succeeds
```

#### UC-IS3: A developer sees which modules their runtime can carry before switching one on

```gherkin
Given a developer's runtime mapping fills the verbs standing agents need but not the one anomaly notification needs
When they read instrukt's status
Then the report shows standing agents available on that runtime
And shows anomaly notification unavailable, naming the verb that is empty
```

#### UC-IS4: A developer sees that a repository would claim a domain another project owns

```gherkin
Given a developer's computer publishes a domain owned by one project
And they stand in another repository whose working tree holds a domain of the same name
When they read instrukt's status
Then the report names the project that owns that domain
And shows that this repository's publication of it would be refused
```

## Known caveats

- A repository adopted with `instrukt init` but never synced is unknown to
  publication state: it appears in `projects` only after its first sync.
  Inside that repository the repository section still reports it.
- Status reports configuration as written; whether a filled verb works
  is learned only by running it.

## See Also

- project/instrukt/spec/domain/publication
- project/instrukt/spec/feature/distribution/claude-plugin
- project/instrukt/spec/feature/configuration/layered-configuration
- project/instrukt/spec/feature/repository/project-initialization
