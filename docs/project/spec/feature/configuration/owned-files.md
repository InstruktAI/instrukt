---
description: Machine-local secrets for every instrukt entry path, versioned owned shapes and migration before configuration refusal or skip.
delivered_by: [instrukt-first-build]
---
# Owned Files — Spec

## What it is

instrukt keeps secret material on the computer that uses it and evolves its
owned file shapes through versioned migrations. Interactive commands, hooks,
receiver requests and scheduled ticks use the same protected secret source and
migration rules. A valid older configuration remains usable after an upgrade.

## Canonical fields

### Machine-local secrets

`~/.instrukt/secrets.env` is the one machine-local file for every secret
instrukt uses, including webhook signing material, Redis credentials and GitHub
App private keys. App profiles carry protected references; installation tokens
are issued in memory under `project/instrukt/spec/feature/work/github-apps`, not
saved as long-lived secrets.
Model-provider keys remain with T3. Configuration holds `${NAME}` references,
never secret values. Every instrukt path resolves such secrets from this file,
including noninteractive ticks; a process environment or project `.env` cannot
supply or override them. Names are machine-wide, so projects requiring distinct
values use distinct reference names.

The file is owner-only and permissions are checked on read. Missing, blank,
unreadable or insufficiently protected required material refuses the dependent
operation with the reference and cause, never its value. An operation needing
no secret does not require unrelated values to be present. Only
`instrukt secrets set` and `instrukt secrets generate` write the file. Writes
are atomic, and values are never echoed or included in diagnostics. Reading,
validation and masked configuration output never disclose them. Ingress still
resolves current and previous signing references at each request for rotation.

### Owned-shape migration

Owned file shapes are versioned and migrated when they change. Configuration
migration runs before refusal or skip on every path. Read-only inspection commands
convert older configuration in memory, leave files and locks untouched, and report
that the file migrates on the next writing command. Writing commands, publication,
installation convergence and ticks persist the conversion before proceeding; a skip
due to unchanged publication is considered only after migration. A conversion
preserves supported user settings and writes atomically; a failed conversion leaves its source intact
and reports the cause. Existing rejection of a configuration version newer
than the binary remains the upgrade door. A revision read during inspection names
the bytes on disk. A revision-checked writing patch verifies those bytes and
performs conversion under the same configuration write lock. Receiver inspection
and signature rejection use the in-memory view; a verified valid delivery persists
conversion before event publication.

Configuration schema version 4 adds `modules.history`. Existing documents
without the key retain the default-on behavior, while explicit false survives
migration. Migration does not insert a project override that would mask a global
choice. Inspection remains in-memory; writing paths persist version 4 atomically.

Other owned files gain a version when their shape first changes. Their readers
use the owned migration path rather than treating prior data as malformed or
silently replacing it. `agents.yaml` default replacement and preservation of
user edits follow `project/instrukt/spec/feature/configuration/agent-selection`;
a mandatory shape conversion is distinct from replacing default choices.

### Worktree isolation

Worktree isolation serves development of instrukt itself. The repository's
`scripts/dev-instrukt` (also exposed by `make dev`) builds that worktree's binary
and runs its private copy with `INSTRUKT_HOME=<worktree>/.instrukt-home`. The home
is gitignored. First use seeds default configuration and publication from that
worktree's committed HEAD, keeping development state and user-scope outputs out
of the normal machine home. Later development syncs use that same worktree
source, and first-use initialization does not overwrite later configuration edits.
Secrets continue resolving from the one normal machine protected file.

An installed instrukt in ordinary project worktrees retains the machine home;
claims and todos remain shared there. Merely entering a linked worktree redirects
nothing. The repository's worktree cleanup integration removes the development
home where such a hook exists; otherwise its cleanup is documented. The home
contents are indexed in `project/instrukt/design/architecture/overview`.

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

### UC-OF2: Migrate before an unchanged-publication skip

```gherkin
Given the machine and project configurations use an older owned schema
And canonical main equals the last published commit
When sync runs after an instrukt upgrade
Then configuration migrates before the unchanged-publication decision
And ordinary reads inspect the converted configuration without writing it and report the pending migration
And ticks persist migration before schema-based refusal or skip
And a failed conversion preserves the source and reports the failure
```

### UC-OF3: Keep worktree homes separate

```gherkin
Given a developer runs scripts/dev-instrukt in two instrukt development worktrees
And each script has seeded its isolated home from that worktree
When one changes its owned publication or operational state
Then the other's home and the normal machine home are unchanged
And both resolve required secrets from the one machine-local protected file
```

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/configuration/agent-selection](agent-selection.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](../runner/inbound-webhook-endpoint-framework.md)
- [project/instrukt/design/architecture/overview](../../../design/architecture/overview.md)
