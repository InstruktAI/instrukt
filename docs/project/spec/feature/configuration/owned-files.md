---
description: Machine-local secrets for every instrukt entry path, versioned owned shapes and migration before configuration refusal or skip.
---
# Owned Files — Spec

## What it is

instrukt keeps secret material on the computer that uses it and evolves its
owned file shapes through versioned migrations. Interactive commands, hooks,
receiver requests and scheduled ticks use the same protected secret source and
migration rules. A valid older configuration remains usable after an upgrade.

## Canonical fields

### Machine-local secrets

`<selected INSTRUKT_HOME>/secrets.env` (normally `~/.instrukt/secrets.env`) is the one machine-local file for every secret
instrukt uses, including webhook signing material, Redis credentials and GitHub
App private keys. App profiles carry protected references; installation tokens
are issued in memory under `project/instrukt/spec/feature/work/github-apps`, not
saved as long-lived secrets.
Model-provider keys remain with T3. Configuration holds `${NAME}` references,
never secret values. Every instrukt path resolves such secrets from this file,
including noninteractive ticks; a process environment or project `.env` cannot
supply or override them. Names are shared within the selected home, so projects requiring distinct
values there use distinct reference names.

A company home has its own secrets file, `~/.instrukt/companies/<name>/secrets.env`,
under the same protection, write and disclosure rules. References in that
home's settings resolve only from it
(`project/instrukt/spec/feature/configuration/company-homes`), and
`instrukt secrets set|generate --company <name>` write it.

The file is owner-only and permissions are checked on read. Missing, blank,
unreadable or insufficiently protected required material refuses the dependent
operation with the reference and cause, never its value. An operation needing
no secret does not require unrelated values to be present. Only
`instrukt secrets set` and `instrukt secrets generate` write the file. At a
terminal, `secrets set` asks for the value with echo off; from redirected input
or `--from-file` it reads that. Writes
are atomic, and values are never echoed or included in diagnostics. Reading,
validation and masked configuration output never disclose them. Ingress still
resolves current and previous signing references at each request for rotation.

### Owned-shape migration

Owned file shapes are versioned and migrated when they change. Configuration
migration runs before refusal or skip on every path. Read-only inspection commands
convert older configuration in memory, leave files and locks untouched, and report
that the file migrates on the next writing command. For the files instrukt owns on
the machine, the global layer and company home documents among them, writing commands, publication,
installation convergence and ticks persist the conversion before proceeding; a sync
compares its outputs only after migration, whether or not main has moved.

A project layer is committed to its repository, so it changes only in a commit,
as do the editor modelines and the repository's generated root `AGENTS.md`
(`project/instrukt/spec/feature/publication/agent-entrypoints`).
instrukt's pre-commit hook section converts an older project layer and stages it
into the commit being made, and the `instrukt config` writing verbs a developer
runs convert it in their checkout. Every other path, checkout rendering, the hooks
that run after a commit, publication, installation convergence and ticks among
them, converts it in memory and leaves the file as committed.
`instrukt config validate` judges a committed project layer older than the
binary's project schema as converted in memory and reports its pending
conversion as a notice, never a finding, so a release that knows a newer
project schema validates a repository that has not converted yet
(`sdlc/policy/backward-compatibility-hygiene`: a shape read across a
multi-version fleet moves once its slowest reader has caught up).
Every write to a configuration layer, a conversion or an
`instrukt config` writing verb, keeps the document's text outside the top-level
keys it changes, comments and formatting included. A conversion
preserves supported user settings and writes atomically; a failed conversion leaves its source intact
and reports the cause. Existing rejection of a configuration version newer
than the binary remains the upgrade door. A revision read during inspection names
the bytes on disk. A revision-checked writing patch verifies those bytes and
performs conversion under the same configuration write lock. Receiver inspection
and signature rejection use the in-memory view; a verified valid delivery persists
conversion before event publication.

Migration preserves explicit module settings and configuration inheritance.
An absent history setting follows the configuration owner
(`project/instrukt/spec/feature/configuration/layered-configuration`); migration
inserts no project override that would mask a global choice. Inspection remains
in-memory, and writing paths persist the converted document atomically.

Other owned files gain a version when their shape first changes. Their readers
use the owned migration path rather than treating prior data as malformed or
silently replacing it. `agents.yaml` holds the user's overrides of the shipped
defaults (`project/instrukt/spec/feature/configuration/agent-selection`);
a mandatory shape conversion is distinct from the defaults.

Hook-run records carry a shape version and an optional checkout or staging
outcome. Older records convert in memory with no invented checkout or staging
result; a record in a shape this binary does not know, a newer one included,
reads as no record and is never refused, and the
next recorded hook run persists the versioned shape
(derived hook-run records rebuild on the next run, `sdlc/policy/state-storage`). Inspection never rewrites these records. A record belongs to
the repository, keyed by a digest of its git common directory and naming the
repository's path, so it is found without reading the repository's
configuration (`sdlc/principle/design-fundamentals`: an outcome is never lost
to a file that cannot be read). Records kept under the project-keyed folder
`state/hooks/` are not converted: the first hook run that finds that folder
removes it, and the next run of each hook writes its outcome under the
repository's key (`sdlc/policy/state-storage`: an entry retires with its
scope).

Publication keeps no record of the files a sync wrote outside the home. The
record an earlier release kept under `state/publication/outputs/` is not
read: the first sync that publishes deletes it, whatever its shape, and
touches none of the files it lists
(`general/principle/trust-and-guardrails`: a record kept to authorize a later
overwrite or removal is a guard on what every sync rebuilds).

### Blueprint tool ownership

Blueprint tool environments and executables are confined to the selected
instrukt home, including an isolated development home. The lifecycle and pin
belong to `project/instrukt/spec/feature/blueprint/graph-engine`; installation,
repair and inspection belong to
`project/instrukt/spec/feature/runner/module-installation`.

### Worktree isolation

Worktree isolation serves development of instrukt itself. The repository's
`scripts/dev-instrukt` (also exposed by `make dev`) builds that worktree's binary
and runs its private copy with `INSTRUKT_HOME=<worktree>/.instrukt-home`. The home
is gitignored. Each use repairs what the development home lacks — its global
layer and its published index — seeded from that worktree's committed HEAD,
keeping development configuration, publication, state and secrets in the selected root;
no marker stands in for the state itself. Later development syncs use that same worktree
source, and first-use initialization does not overwrite later configuration edits.
Secrets resolve only from the selected root. Runtime instruction homes and
scheduler files follow their separate native/location-specific inputs, not
the development root. The wrapper selects its app log directory through
`INSTRUKT_LOG_DIR` without moving HOME or XDG bases.

An installed instrukt in ordinary project worktrees retains the machine home;
claims and todos remain shared there. Merely entering a linked worktree redirects
nothing. The repository's worktree cleanup integration removes the development
home where such a hook exists; otherwise its cleanup is documented. The home
contents are indexed in `project/instrukt/design/architecture/overview`.

### Explicit roots and external locations

An explicit Instrukt root owns its secrets with no fallback, copy or deletion of
the normal machine secret file. Existing explicit-root installations provision
required secrets in their selected root through the existing secrets commands.
Default installations retain their protected path. Runtime defaults use inherited
HOME independently. External scheduler files use the scheduler-location contract
in `project/instrukt/spec/feature/runner/module-installation`, not an implicit
Instrukt-private user directory.

## Use cases

### UC-OF1: Resolve the same secret interactively and on a tick

```gherkin
Given a developer records a signing or Redis secret through instrukt's secrets commands
When an interactive command and a scheduled invocation require that reference
Then both read the same owner-only machine-local secret
And neither project environment files nor process environment override it
And no output discloses its value
But an unreadable or insufficiently protected required file refuses the dependent operation
```

### UC-OF2: Migrate before publication, even when main has not moved

```gherkin
Given the machine and project configurations use an older owned schema
And canonical main equals the last published commit
When sync runs after an instrukt upgrade
Then configuration migrates before anything is published
And the machine configuration is written at the current schema while the project configuration stays as committed
And ordinary reads inspect the converted configuration without writing it and report the pending migration
And ticks persist the machine migration before schema-based refusal or skip
And a failed conversion preserves the source and reports the failure
When the developer next commits in that repository
Then the commit carries the project configuration at the current schema
```

### UC-OF3: Keep worktree homes separate

```gherkin
Given a developer runs scripts/dev-instrukt in two instrukt development worktrees
And each script has seeded its isolated home from that worktree
When one changes its owned publication or operational state
Then the other's home and the normal machine home are unchanged
And each resolves required secrets only from its selected Instrukt root
```

## Known caveats

- A write that changes a key's nested value renders that key anew, so comments
  and formatting inside it are lost; the rest of the document keeps its text.
- A layer written in flow style or with quoted keys is rendered whole and
  loses its comments and formatting; the write logs `config_rendered_whole`.
  A file holding several YAML documents is refused when it is read, as any
  unreadable layer is, and is never rewritten.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/configuration/agent-selection](agent-selection.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](../runner/inbound-webhook-endpoint-framework.md)
- [project/instrukt/design/architecture/overview](../../../design/architecture/overview.md)
