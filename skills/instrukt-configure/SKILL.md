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
- project/instrukt/spec/feature/configuration/owned-files
- project/instrukt/spec/feature/runner/module-installation
- project/instrukt/spec/feature/runner/single-ingress-webhook-termination

If the binary or a required command is unavailable, report that prerequisite
for the affected work. Keep configuration success separate from
infrastructure readiness.

Keep the developer's existing choices unless their requested outcome changes
them. Ask only for unresolved facts or intent instrukt cannot supply. Settle
participating computers, each project's placement, intended module enablement,
the one ingress computer and operator-supplied cluster endpoint. Preserve an
existing runtime choice; an unset `runner.runtime` stays unset until the
developer chooses one for runtime-dependent work.

Prepare one brief for the participating agents containing the agreed facts and
each computer's intended result. Read existing identities and project paths
from available instrukt reports and local project metadata; report a missing
identity discovery surface as a prerequisite instead of inventing an ID.

- `ingress.computer` is the same existing stable instrukt machine ID everywhere.
  Runtime environment identity and hostname are separate facts.
- `redis.cluster.url` reaches the same logical database and stream namespace on
  every event participant. Equivalent URLs and machine-local credential
  references may differ; verify their common destination and required access.
- Record project identities and computer-local checkouts as placement facts.
  Ensure ingress holds each source-declaring adopted project, since external
  procedure activation occurs there. Existing todo claims retain their meaning.
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
   across adopted projects. `--module todos` or `--module events` can narrow
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
results, relevant status findings and remaining repair doors. Include only
masked endpoints and credential references. Existing consumer positions and
durable work are retained by the installation contract.

Compare participating reports for agreement on ingress, logical Redis store
and project placement. Mark a computer complete only with its own verification;
name inaccessible computers and operator prerequisites as incomplete. Report
configuration-management distribution work separately from instrukt changes.
