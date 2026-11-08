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
| Profile `kind` | A known harness kind selecting that profile's typed shape and native option fields. |
| Profile tier chains | Nonempty ordered entries for `light`, `standard`, `strong` and `premium`, each holding a model name and that harness's own effort/option fields. |

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
Model names are open strings. Unknown names are neither rejected nor tested
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

Before each instrukt-owned start the runner checks that the selected harness
is installed, its instance enabled and logged in, and its models reported.
This check precedes invoking startup, which can create a T3 thread before
finding a missing harness. Selection reports each skipped profile/model and
why, including readiness and local capacity restrictions. Exhaustion leaves
the work pending and uses the existing anomaly route.

`instrukt agents availability` reads local expiring capacity marks;
`instrukt agents status` changes them. Profile marks apply to that profile;
model marks apply to the selected model within its profile. Existing
available/unavailable/degraded semantics remain: automatic starts skip
unavailable and degraded capacity, expiry lapses at the command clock, and
available clears the restriction. A quota rejection marks the selected model
itself unavailable with expiry, without disabling its other models or another
profile. A capacity mark never proves that startup created no thread.

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

### UC-AS2: Exhaust one ready profile before trying the next

```gherkin
Given a role orders two ready profiles with ordered chains for its tier
When selection skips an unavailable model in the first profile
Then it tries the remaining entries there before the second profile
And reports each skipped profile or model and its reason
But when an invoked start has an uncertain outcome
Then that assignment remains under recovery rather than falling through to another start
```

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

### UC-AS5: Check a harness before creating a thread and contain quota marking

```gherkin
Given an instrukt-owned start needs a profile
When that harness is not installed, enabled, logged in or reporting models
Then it is skipped with a reason before thread startup is invoked
But when a selected model is rejected for quota
Then that model receives an expiring unavailable mark without marking the whole profile
```

## Known caveats

- `t3 thread start` has no effort/options flag. Until upstream supports it,
  instrukt's own starts receive effort only from the T3 project default.
  Native delegation and the T3 server can carry options; instrukt builds no
  workaround for the host CLI. The upstream request is
  [pingdotgg/t3code#29863](https://github.com/pingdotgg/t3code/issues/29863).
- Editor model suggestions are manually maintained and neither tested nor
  used to reject unknown names. Readiness reporting and actual provider
  acceptance remain separate from configuration shape validation.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](roles-and-capabilities.md)
- [project/instrukt/spec/feature/configuration/owned-files](owned-files.md)
- [project/instrukt/spec/feature/configuration/configuration-skill](configuration-skill.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
