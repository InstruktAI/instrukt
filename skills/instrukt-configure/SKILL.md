---
name: instrukt-configure
description: Configure or repair instrukt on one computer or a cluster, settle the facts shared across computers, and verify each machine's enabled module infrastructure. Routes coordination through the configured runtime when instructions are available.
license: MIT
---

# Configure instrukt

Configure each participating computer for the developer's intended setup and
report its verified state. Use the installed CLI as the authority for command
availability and current settings. The complete skill directory can be read
from a checkout or supplied directly to an agent on each computer.

## Establish the facts

Read `instrukt -h`, relevant command `-h`,
`instrukt config get --global --json` and, in each participating project,
`instrukt config get --json`. Where instrukt's source specifications are
available, consult these contracts with `instrukt docs get`; on other
computers, use the installed command help for their supported behavior:

- project/instrukt/spec/feature/configuration/layered-configuration
- project/instrukt/spec/feature/configuration/configuration-editing
- project/instrukt/spec/feature/configuration/configuration-skill
- project/instrukt/spec/feature/configuration/agent-selection
- project/instrukt/spec/feature/work/github-apps
- project/instrukt/spec/feature/work/github-record
- project/instrukt/spec/feature/configuration/roles-and-capabilities
- project/instrukt/spec/feature/configuration/owned-files
- project/instrukt/spec/feature/runner/module-installation
- project/instrukt/spec/feature/runner/single-ingress-webhook-termination

If the binary or a required command is unavailable, report that prerequisite
for the affected work. Keep configuration success separate from
infrastructure readiness.

Keep the developer's existing choices unless their requested outcome changes
them. Ask only for unresolved facts or intent instrukt cannot supply. Settle
participating computers, each project's placement, intended module enablement (blueprint, todos, events and history),
the one ingress computer, the one computer that reacts to external facts, and
the operator-supplied cluster endpoint. Preserve an
existing runtime choice; an unset `runner.runtime` stays unset until the
developer chooses one for runtime-dependent work.

Offer history with its default-on setting like the other modules. Explain that
local history needs no Redis and merged search independently demands cluster
Redis, even when events are off. For an explicitly core-only setup, record
`modules.history: false`; existing history remains stored when disabled.

Prepare one brief for the participating agents containing the agreed facts and
each computer's intended result. Read each computer's machine ID, and its own
`ingress.computer` and `ingress.reactions`, from its `instrukt events status`,
and project paths from local project metadata; never invent an ID.

- `ingress.computer` is the same existing stable instrukt machine ID everywhere.
  Runtime environment identity and hostname are separate facts.
- `ingress.reactions` is the same machine ID everywhere for the one computer
  that reacts to external facts, or left out everywhere so the ingress reacts.
  instrukt does not detect computers that disagree, so compare what each
  computer's `instrukt events status` reads.
- `redis.cluster.url` reaches the same logical database and stream namespace on
  every events or merged-history participant. Equivalent URLs and machine-local credential
  references may differ; verify their common destination and required access.
- Record project identities and computer-local checkouts as placement facts.
  Ensure the ingress holds each source-declaring adopted project and company
  home's settings, with their signing values, for verification; it needs no
  runtime when another computer reacts. Ensure the reacting computer holds the
  project checkouts and company homes external procedures run in, with their
  published procedures and a chosen runtime. Existing todo claims retain their
  meaning.
- When the reacting computer is another computer than the ingress, configure
  and install it, with its consumer prepared, before the ingress starts
  receiving: a consumer prepared later starts at the stream's end and misses
  the facts published before it.
- Change `ingress.reactions` or `ingress.computer` only while nothing is in
  flight: no participating computer's procedure consumer has entries from
  before the change left to read or settle, and the computer that reacted so
  far has settled what it accepted. Stopping intake keeps new entries from
  arriving; it does not satisfy that alone.
- Align `ingress.port` with the ingress edge. Keep local Redis endpoints,
  runtime, credentials, ticks and project overrides specific to each computer.
  Record signing and Redis material through `instrukt secrets set` or
  `instrukt secrets generate` on the computer that uses it. References resolve
  only from that machine's owner-only `~/.instrukt/secrets.env`, including on
  ticks; keep secret values out of patches, briefs and reports.

Configuration management outside instrukt distributes these agreements. Each
machine's global layer remains its own; project placement uses existing homes
rather than an added routing setting. Carry credential references and masked
findings in briefs and reports, with secret values resolved only at their
declaring computer.

## Configure GitHub identities

For projects enabling todos, configure the machine's public `github.apps`
inventory and `github.project_app`, and record per-role `github_app` references
in the agents file with `instrukt config get --agents --revision` and
`instrukt config patch --agents --revision <rev>` under the GitHub Apps and
configuration-editing contracts. Reuse the project,
author and reviewer Apps already present on the installation; record their
observed App ids, slugs and stable bot account ids/node ids/logins.
For a new installation, guide the developer through creating and installing
three distinct Apps, using the actual assigned identities and the repository
and account permissions required by their responsibilities. The project App
maintains records, issues, bugs and the board; authoring and native review use
different Apps. Preserve the role's other capabilities and model choices.

Store each private key through `instrukt secrets set NAME`, with redirected
stdin or a protected input file, and put only `${NAME}` in the public profile's
`private_key`. Each computer keeps its own protected key material. Configuration
reports carry public identities and reference names. Verify the configured App
and stable bot identities, the repository installation, permission scopes and
credential issuance through the available App-authenticated operation. Report
missing command/API capabilities or credentials as incomplete setup with their
repair door; verification remains pending until observed. Board setup and repair
run automatically with reflection, including when a project has no `board`
reference yet. Preserve unrelated project fields and existing matching resources.

## Route the work

Read the coordinating computer's configured `runner.runtime`. Retrieve
`instrukt docs get general/procedure/ai-to-ai-operations --closure` for its
read-time configured environment discovery, connection and remote-task
instructions. Use those rendered operations to arrange the local work below;
each target retains its own runtime choice. Runtime execution wording lives in
that runtime's configuration entry.

When the runtime is unset, its configured route is unavailable, the procedure
cannot be retrieved or a target cannot be reached, give the developer that
computer's brief and this skill directory's location to run locally there.
Continue independent reachable work and leave that computer pending until its
report arrives. Core configuration can be completed without choosing a runtime.

## Configure agent profiles and readiness

Include this phase in every computer's local assignment. Read the effective
selection with `instrukt config get --agents [--json]` (and
`instrukt config show agents` for expanded grants) together with its revision
via `instrukt config get --agents --revision`, and show the current role
mappings and profile order before changing selection. Through the configured
runtime's available operations, discover ACP instances already configured in T3,
ask where the developer wants them in that order, and record profile, order,
and role changes with `instrukt config patch --agents --revision <rev>` under
the agent-selection and configuration-editing contracts. Each patch validates
the full effective selection and keeps only overrides of the shipped defaults;
on a stale revision, reread and recompute the intended difference so concurrent
choices survive. Preserve unrelated role capabilities, App references and model
choices, then read back the effective mapping with
`instrukt config get --agents --json`.

For a supported harness that is not ready, guide the developer through its login
or key setup in T3 and wait for observed readiness. T3 owns provider instances
and model-provider credentials. Report an unavailable discovery or readiness
surface, or unfinished setup, as an unmet prerequisite for that computer.

## Converge and verify locally

The local agent's outputs are the addressed computer's intended configuration,
its infrastructure verification and a report to the coordinating agent or
developer. Route prerequisites and observations outside that assignment back
in the report.

1. Read current global and project settings and use
   `instrukt config get --revision [--global]` for each addressed layer.
   Initialize a missing layer with `instrukt config init [--global]` as needed.
   Compare against this computer's brief and prepare only the required merge
   patch, keeping references unexpanded. Project module overrides determine
   demand independently of global defaults.
2. Apply each difference with
   `instrukt config patch --revision <rev> [--global] --from-file <file>`.
   On a revision conflict, reread and recompute the intended difference so
   concurrent settings survive. Validate the global layer with
   `instrukt config validate --global` and each addressed project with
   `instrukt config validate`; read back effective values. Keep already matching
   documents as they are.
3. For developer-enabled modules, use `instrukt install` to converge demand
   across adopted projects. `--module todos` or `--module events` or `--module history` can narrow
   the requested installation/check scope. Use full convergence for withdrawal
   after disable so remaining projects' demand is counted. An events-disable
   patch removes conflicting source declarations in the same valid patch.
   Let installation own resource reuse, repair, ingress placement and retained
   consumer readiness; report its named prerequisites for any incomplete setup.
4. Run `instrukt install --check` with the same scope. Inspect
   `instrukt ticks status` where todos or events need scheduling, and
   `instrukt events status` for event setup or retained event findings.
   Treat matching settings and passing checks as completed work. Reuse the
   same installation command for repair after resolving an unmet prerequisite.

## Report the result

For each computer, return its identity, addressed project paths, configuration
differences or already-matching state, validation and installation/check
results, agent profiles and role/profile order changes, observed harness
readiness, relevant status findings and remaining repair doors. Include only
masked endpoints and credential references. Existing consumer positions and
durable work are retained by the installation contract.

Compare participating reports for agreement on the ingress, the reacting
computer, the logical Redis store and project placement. Mark a computer complete only with its own verification;
name inaccessible computers and operator prerequisites as incomplete. Report
configuration-management distribution work separately from instrukt changes.
