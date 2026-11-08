---
description: Role-keyed provider-instance profiles and ordered model tiers, with readiness checks, open model names and user-owned defaults.
delivered_by: [instrukt-first-build]
---
# Agent Selection — Spec

## What it is

Model choice belongs to the computer's configuration, independently of procedure
content. `~/.instrukt/agents.yaml` supplies one role map and profiles of existing
T3 provider instances. A procedure's `role` selects its mapping; an unlisted
role uses the default. A per-task override takes precedence. Users can choose
reliable inexpensive or open models as well as frontier subscriptions, without
changing a procedure or recompiling instrukt.

## Canonical fields

### File and selection inputs

| Field | Meaning |
| --- | --- |
| `default: {class, can, order, tier}` | Fallback role entry; order and tier select models, class and grants follow roles-and-capabilities. |
| `roles: {<role>: <role-entry>}` | One entry combines class, scoped `can` grants, model order and tier, and optional GitHub App reference. |
| `bundles: {<name>: <grants>}` | Named scoped capability bundles, owned by roles-and-capabilities. |
| `profiles: {<name>: <profile>}` | User-visible profile names; orders reference these names. |
| Profile `instance` | The existing T3 provider-instance id this profile addresses. |
| Profile `kind` | A known harness kind selecting that profile's allowed effort values and typed extra settings. |
| Profile tier chains | Nonempty ordered entries for `light`, `standard`, `strong` and `premium`, each holding a model name, one runtime-neutral `effort` field validated per kind, and extra harness settings beside it. |

Several profiles can share a kind, including separate accounts of one harness.
Profile names, provider-instance ids and kinds are distinct identities. The
file, rather than `instrukt.yaml`, `t3.yaml` or procedure frontmatter, owns
orders, role mappings and chains. Role class and grants share that same map under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`; App references
select the bot identity under `project/instrukt/spec/feature/work/github-apps`.
Non-bot agent git-author settings remain separate in `instrukt.yaml`. The
selected runtime's adapter translates kinds, instance ids and native option names; it supplies no compiled model catalogue.

A task override wins over its role's selection; omitted inputs inherit the role
mapping, then `default`. Selection walks the effective profile order
sequentially. In the first ready profile it walks the selected tier's chain;
only after that chain is exhausted does it try the next profile. Capacity
restrictions and reported unavailable models are skipped with their reasons.
Both runner startup evidence and native worker assignments retain the resolved
profile, kind, instance, model and harness-native effort/options.

### Shipped defaults and upgrades

instrukt deploys the file once and then treats it as the user's own. It ships
default profiles for every harness T3 has built in. The role map preserves the
cross-model pattern: authors prefer Claude, reviewers prefer Codex. These are
heuristics, not validation rules, and DeepSeek has no special default position.
Shipped effort is `high`, never `xhigh` or `max`; each `premium` chain ends
with the strong model. Exact models and chains belong to the shipped data,
not a price promise or a rule enforced against user edits.

On upgrade, an unedited deployed file is replaced with the new defaults. An
edited file is retained and instrukt reports that new defaults exist. Whether
it was edited is established against the deployed version, never guessed from
its age. Owned-shape migrations remain distinct from default replacement and
follow `project/instrukt/spec/feature/configuration/owned-files`.

### Validation and inspection

Shape is strict: known kinds, the four tier names, declared profile references,
nonempty orders/chains and the kind's typed fields and allowed effort values.
Effort remains a closed semantic vocabulary through per-kind validation; the
selected runtime adapter maps it to native option identifiers. Model names are
open strings. Unknown names are neither rejected nor tested
against a catalogue. A hand-maintained schema in instrukt's repository offers
known model names as editor suggestions, without restricting other names.
This is separate from the generated repository YAML schemas.

Validation rejects unknown action/bundle names and malformed role entries under
roles-and-capabilities, and reports configured roles no installed procedure
declares, apart from reserved fallback entries. `instrukt config show agents`
prints effective role classes, expanded scoped grants, App references, model
mappings and profiles.
It exposes configuration, never provider credentials. Changing selection does
not change an existing conversation.

### Readiness, skips and quota

The runner supplies readiness, capacity and selection-realization outcomes under
`project/instrukt/spec/feature/runner/session-continuity`. Selection consumes
those outcomes before a new start, preserving the configured profile order and
per-tier chains. A failed or incomplete readiness probe cannot admit a profile;
source-supported configurations that require no provider login remain usable.
Native default/automatic picks follow the selected adapter's declared behavior.
Skips and exhaustion retain their reasons and leave work pending through the
existing anomaly route.

`instrukt agents availability` and `instrukt agents status` inspect and change
runner-owned capacity restrictions. Their expiry and observed-quota policy is
owned by the runner contract, independently of configuration shape validation.

Ambiguous startup retains its key and selected inputs for recovery. Chain
fallback after invocation requires rejection proven before creation or explicit
no-start resolution under `project/instrukt/spec/feature/runner/session-continuity`.
Native delegation uses the same resolved selection through configured runtime
instructions where its native operation supports the options.

### Configuration skill and ACP

The configuration skill discovers ACP instances the user already has in T3,
shows the current order, asks where to place them, and writes the corresponding
profiles and order changes. For a supported harness that is not ready, it guides
login or key setup and waits for readiness. instrukt creates no provider
instances and carries no ACP implementation. Model-provider keys remain with
T3, outside instrukt's secrets file.

## Use cases

### UC-AS1: Change model choice without editing a procedure

```gherkin
Given a developer has two profiles of the same harness kind and a procedure with a role
When they change that role's order and tier in agents.yaml
Then later selections follow those profiles and tier without changing the procedure
And a role absent from roles uses default
And a per-task override takes precedence
```

### UC-AS2: Exhaust one ready profile before trying the next (retired)

Retired: sequential selection is part of the prose contract above.

### UC-AS3: Configure a new model name within the harness shape

```gherkin
Given a developer enters a model name absent from the editor's suggestions
When its profile uses a known kind and valid typed effort and option fields
Then validation accepts that model name
But malformed fields or an invalid effort value produce shape findings
And configured roles with no installed procedure are reported
```

### UC-AS4: Preserve user edits during a defaults upgrade

```gherkin
Given a newer instrukt ships changed agents defaults
When its deployed file is unchanged from the prior defaults
Then the file receives the new defaults
But an edited deployed file is kept and the new defaults are announced
```

### UC-AS5: Check a harness before creating a thread and contain quota marking (retired)

Retired: readiness and quota are parts of the prose contract above.

## Known caveats

- `t3 thread start` has no effort/options flag. Until upstream supports it,
  instrukt's own starts receive effort only from the T3 project default.
  Native delegation and the T3 server can carry options; instrukt builds no
  workaround for the host CLI. The upstream request is
  [pingdotgg/t3code#29863](https://github.com/pingdotgg/t3code/issues/29863).
- Mandatory conversion of a version-one agents file preserves user choices and
  refuses conflicting retired effort fields with the source intact until they
  are reconciled.
- Editor model suggestions are manually maintained and neither tested nor
  used to reject unknown names. Readiness reporting and actual provider
  acceptance remain separate from configuration shape validation.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](roles-and-capabilities.md)
- [project/instrukt/spec/feature/configuration/owned-files](owned-files.md)
- [project/instrukt/spec/feature/configuration/configuration-skill](configuration-skill.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
