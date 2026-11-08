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
migration runs before refusal or skip on every path, including ordinary reads,
patches, validation, publication, installation and ticks. A skip due to unchanged
publication is considered only after migration. A conversion preserves supported user
settings and writes atomically; a failed conversion leaves its source intact
and reports the cause. Existing rejection of a configuration version newer
than the binary remains the upgrade door.

Other owned files gain a version when their shape first changes. Their readers
use the owned migration path rather than treating prior data as malformed or
silently replacing it. `agents.yaml` default replacement and preservation of
user edits follow `project/instrukt/spec/feature/configuration/agent-selection`;
a mandatory shape conversion is distinct from replacing default choices.

### Worktree isolation

Worktree-scoped execution uses an isolated instrukt home per worktree, keeping
that worktree's owned publication and operational state separate from other
worktrees and the normal machine home. It reads the same machine-local secret source instead of copying
secrets into worktrees. The home contents are indexed in
`project/instrukt/design/architecture/overview`.

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
And ordinary reads and ticks also migrate before schema-based refusal or skip
And a failed conversion preserves the source and reports the failure
```

### UC-OF3: Keep worktree homes separate

```gherkin
Given two worktrees use isolated instrukt homes
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
