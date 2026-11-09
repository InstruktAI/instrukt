---
description: Enabled adopted projects determine the shared wake-up, private local Redis and ingress receiver infrastructure installed, checked and repaired on each computer.
visibility: public
delivered_by: [instrukt-first-build]
---
# Module Installation — Spec

## What it is

Enabling a module authorizes its configuration agent to install that module's
required infrastructure on the appropriate computer. `instrukt install`
converges the infrastructure demanded by all adopted projects' effective module
settings. Core adoption installs none. Configuration writes and publication
record desired settings; installation is a separate explicit act followed by
checks. The runtime-neutral configuration skill consumes these commands; its
remote routing and user interview are outside this contract.

## Canonical fields

### Demand across adopted projects

Publication's adopted-project identities and registered checkout paths supply
the project set. Evaluate project-over-global module settings for each. A global
default without an adopted enabled project creates no infrastructure demand.
Writing installation reads the registered checkouts and retains their validated
project settings and resolved repository identities in this computer's own
installation state. Inspection reads the current repository and those retained
facts for other projects; it does not traverse other working checkouts. Missing
retained facts are findings with the `instrukt install` door. An unreadable or
invalid project's desired state is never proof that its shared resources are
unused; withdrawal waits for a complete demand read.

| Effective demand | Infrastructure and prerequisites |
| --- | --- |
| Core, with history explicitly off and no project with todos/events/history enabled | No periodic wake-up, Redis or receiver installation. Documentation/configuration remain usable without a runtime. |
| Blueprint enabled | Release-pinned Graphify in the instrukt home via `uv tool install graphifyy==<pin>`; uv is an unmet prerequisite when installation is needed and uv is absent. No wake-up, Redis or receiver solely for blueprint. |
| History enabled (default) | Cluster Redis checked for merged-search request/reply, independently of events. Local history commands require no Redis. No minute wake-up, local event Redis, event consumers or receiver solely for history. |
| Todos enabled | One shared minute wake-up unless `ticks: off`; selected capable runtime and existing GitHub repository/access/reflection prerequisites. No Redis or receiver solely for todos. |
| Events enabled, no project sources | Shared wake-up unless off; private local Redis setup or verified operator local Redis; cluster transport check; retained consumer coverage prepared before producers. No receiver. |
| Events enabled with nonempty valid project `webhooks:` | The events dependencies, plus receiver/socket service on the designated ingress computer only. Other computers retain consumers without listeners. |

The local database is created/migrated on commands that need it under its own
contract. Todos may record finalize-ready events locally while events are off;
this creates no Redis dependency or event activation. Blueprint's hook behavior
adds no wake-up/Redis/receiver demand.

`modules.events` is authoritative. Sources with events off are a configuration
finding, never an implicit enablement. Empty `webhooks: {}` creates no endpoint
or receiver demand. `runner.autostart: false` leaves the enabled module's demand
intact: deliberate todo pickup still has reflection and timing responsibilities.

### Installation and inspection commands

| Command | Result |
| --- | --- |
| `instrukt config init [--global]` | Initialize the addressed configuration without installing infrastructure. |
| `instrukt config get [<path>] [--global] [--json]` and `instrukt config get --revision [--global]` | Read settings and revision with credentials redacted or references preserved. |
| `instrukt config patch --revision <rev> [--global] --from-file <file>` | Validate and atomically record desired configuration; standard input and `--yaml` retain their existing alternatives. Installs nothing. |
| `instrukt config validate [--global]` | Validate layer, effective module/runtime prerequisites and source references; names findings without exposing secret values. |
| `instrukt install [--module todos\|events\|history\|blueprint]` | Read machine-wide demand, install/reuse/repair its resources and withdraw unneeded instrukt-owned units; report per-resource results and remaining findings. |
| `instrukt install --check [--module todos\|events\|history\|blueprint]` | Read-only comparison of desired and actual infrastructure, capabilities, ownership and retained-reader readiness; nonzero for unmet demand or unsafe drift. Older configuration is converted in memory and reported as migrating on the next writing command. Creates no file, including logs, SQLite sidecars, locks or defaults; takes no lock and mutates no package, service, configuration, group position or data. |
| `instrukt ticks status` | Show registration mode, last wake and stale/missing wake findings. |
| `instrukt events status` | Show enabled projects, scoped Redis reachability, per-consumer stream positions/lag/pending, activation/delivery failures and receiver placement/health; masks credentials. |
| `instrukt events dead-letters` | Inspect retained dead-letter identity, original transport coordinate and failure facts without arbitrary Redis operations. |
| `instrukt webhooks serve` | Internal receiver entry point for owned service definitions; refuse before binding on non-ingress or with no enabled valid source. |

The optional module filter limits which dependency set is converged or checked;
it never overrides disabled configuration, ingress placement or other projects'
demand. Full demand is always read before withdrawing a shared resource. Without
a filter, obsolete owned resources are included even when all modules are off.
`install` and `install --check` remain available with modules off. Event
inspection can report retained infrastructure/failures after disable and creates
no event workload. Commands' `-h` describes these boundaries and the repair door.

The configuration agent records enablement with revision-checked configuration
commands, calls `instrukt install`, then `instrukt install --check` and relevant
`ticks status`/`events status`. Repair uses the same `install` command after a
finding's named prerequisite is resolved. No separate repair installer exists.
Successful configuration is distinct from successful installation. An interrupted
or partially failed install reports its completed and remaining resources;
rerunning reconciles observed state rather than assuming the prior run completed.
The ownership journal retains both pre-write and desired definitions until the
transition completes, so each interrupted file-write prefix remains provably
owned and can be reconciled or withdrawn.
Reports name installed, reused, repaired, withdrawn, externally supplied or
unmet resources, their paths/units and demanding project identities, without
expanded credentials. A failed prerequisite never produces an overall success.

### Graphify installation

For blueprint demand, use the lifecycle and pin owned by
`project/instrukt/spec/feature/blueprint/graph-engine`. Installation reuses a
matching tool, installs a missing one and repairs a different or broken version;
it verifies the installed version before reporting success. uv's tool and binary
directories both remain inside the instrukt home. A missing uv or failed install
is an unmet prerequisite with its repair door, never a successful installation.
`install --check` detects a missing, broken or differently versioned home-owned
binary without installing, repairing, creating bytecode or changing files.
Disabling blueprint installs nothing and retains its existing tool; it creates
no background infrastructure demand.

### GitHub reflection readiness

For enabled todos, inspection consumes work's CLI-composed GitHub readiness
port and surfaces unmet findings with their configuration repair door and any
available retry boundary. Readiness follows the owning contract in
`project/instrukt/spec/feature/work/github-apps`.
Inspection writes no local state and reflects no pending work. Other projects
use only this machine's retained repository and board settings; their checkouts
are not read.

### Shared minute wake-up

One registration calls `instrukt tick` each minute for all enabled work. Use the
existing launchd, systemd-user timer or cron fallback appropriate to the computer;
there is no registration per task or project. `ticks: off` means the operator
provides scheduling and installation removes an unneeded owned registration.
It creates no replacement scheduler. Tick and explicit long-lived run retain
the same due-time and pending/new event-drain contracts.

`ticks status` distinguishes no module demand, installed native/fallback
registration, missing/broken registration and operator scheduling with ticks
off. It reports last invocation independently, including none ever recorded.
An enabled scheduled computer with no wake for five minutes is stale; no-demand
mode reports inactivity without treating it as broken required infrastructure.
One wake proves neither delivered activation nor completed task.

### Local and cluster Redis

Each module contributes its own Redis demand. History checks cluster-only
request/reply permissions (lists, transactions, expiry and readiness keys).
When history uses an existing instrukt-owned Redis as its cluster endpoint,
installation retains and maintains that service and its shared ACL even with
events off; it prepares no event consumers solely for history. Existing
stream-only owned ACL assets migrate into the resource transition journal,
which retains the before/after proof while the service restarts to load the
request/reply permissions. Read-only inspection reports the pending repair
without changing files or services. An unused local
event Redis is withdrawn when history points at an external cluster.

For events, reuse an explicitly configured working local Unix-socket Redis
without rewriting its config, credentials or service. Otherwise install the
Redis executable when absent using a supported platform provisioner, and
configure one instrukt-owned local instance. It has private configuration/data
under instrukt state, an owner-only Unix socket, no TCP listener, persistence,
default ACL user off and one instrukt ACL user with credentials in the one
machine-local owner-only `~/.instrukt/secrets.env`, written through the secrets
commands under `project/instrukt/spec/feature/configuration/owned-files`.
Only secret references enter configuration or output. Its owned launchd or
systemd service survives runner exit. Package provisioning must preserve foreign
service state; inability to do that is a prerequisite finding, never permission
to stop or reconfigure another Redis.

Redis 7.0 or later is required. Verify executable/version and required Streams,
groups, stale-pending reclaim, Lua and ACL operations using disposable diagnostic
keys. Read-only checks use `ACL DRYRUN` as their non-mutating permission proof.
The proof covers the actual publisher's `EVALSHA` and cache-miss `SCRIPT LOAD`,
as well as `XADD` on the production event/dead-letter stream keys; permission on
a diagnostic key alone is insufficient. Older servers are an unmet prerequisite,
with the Redis upgrade door. Authenticate and check
socket permissions before recording the new local endpoint. Check cluster endpoint/database, reachability and permissions separately.
The logical key namespace is the code-owned `instrukt:*`; there is no user namespace setting. A lone computer may
explicitly use the same instance for both scopes; identical connection identity
is read once. No cluster host is selected or provisioned on another computer.

If local setup fails before any local endpoint/progress is established, report
the cause and door. Unconfigured-local fallback may carry local events on the
verified cluster, visibly; a successfully checked fallback is reported as such,
never as installed local Redis. Missing/unusable cluster prerequisites remain
unmet. An established configured local endpoint that fails is not unconfigured;
installation never silently redirects its pending positions to cluster or
discards a group to make a check pass. Scope handover follows event-transport.

### Ingress receiver and coverage

Receiver demand exists only on the configured `ingress.computer`, identified
by instrukt's stable machine ID, and only while an adopted enabled project's
nonempty valid source map exists there. Missing declaring checkouts or signing
values fail with the project/reference named. Signing material resolves only
from the ingress computer's owner-only `~/.instrukt/secrets.env`; other
computers need no signing values. Ticks and interactive paths use that same
file for their required signing, Redis and other instrukt secrets.
All participants agree on ingress ID and cluster Redis endpoint plus database;
local sockets, runtime and tick policy remain local. Configuration management
distributes those facts; instrukt has no shared configuration layer.

On Linux use a systemd socket/service pair; on macOS use a launchd job with its
named listening socket. Both retain the loopback socket across receiver process
restarts and pass it to `instrukt webhooks serve`. Units execute the installed
instrukt binary with its configuration scope; there is no release pointer or
daemon fallback. Cron can supply ticks but cannot meet receiver socket custody.
Unsupported managers refuse receiver installation with the missing prerequisite.
Non-ingress installations skip receiver creation; direct serving there refuses.

The socket and receiver agree on loopback and configured port. A receiver exits
before serving after loss of ingress designation; a later convergence removes
only its obsolete owned units. Valid source edits are read per request without
restart. Invalid changes do not replace a working validated listener config.
Owned execution changes take effect for existing and new installations while
receiver ownership remains proven. Same-port restarts retain the registered
listening socket; health and inspection establish that the desired installed
executable and configuration are running. See the
[receiver activation implementation](../../../../../crates/instrukt-runner/src/installation/_launch.rs)
for the concrete mechanism. A port change
registers and checks the replacement listener before withdrawing the prior owned
units, and reports the edge-route change for operator alignment without waiting
for the operator. Interrupted replacement retains its recorded ownership for
reconciliation on the next installation run.

Prepare all new retained groups at the current tail before admitting relevant
producers or enabling intake. Existing groups keep their own progress; startup
never resets them. Installation verifies the local activation/relay consumers'
coverage before starting a new receiver. A receiver remains independent of
`run`/`tick` lifetime. Its health check establishes ownership, loopback, authenticated
Redis publication capability and prepared readers. Protocol verification also
exercises invalid-signature rejection and a valid diagnostic delivery through
the ordinary claim/append and downstream observation path in a disposable
environment with a test procedure; production checking does not fabricate a
business event or claim end-to-end activation from a socket check.

The operator owns TLS, edge filtering/abuse protection and provider registration.
Socket custody is bounded by the provider timeout; it is not a durable HTTP queue.
While cluster Redis cannot publish, requests fail rather than being accepted
into a new local store-and-forward service.

### Withdrawal and ownership

Disabling a module stops admission of its work on the next effective-config
evaluation; config edits themselves remove no units. A subsequent explicit
`instrukt install` withdraws only instrukt-owned resources with no remaining
enabled project demand. Shared dependency users are counted across modules and
projects, including explicitly configured references to an owned Redis instance.
Physical server identity protects Unix socket references regardless of URL database
qualification (`?db=0` or any other database).
Removing the last source withdraws the receiver while retaining event transport
for still-enabled events. Disabling events retains the wake-up if todos need it.

Ownership requires the installation's recorded resource identity and matching
owned definition. An unrelated unit, listener, socket, config or changed foreign
definition at the desired address is a conflict with a named repair door, never
overwritten or removed. Cron withdrawal compares the entire marked definition
against the retained owned line and preserves the whole table on conflict.
Withdrawal stops/unregisters owned units only; it never
deletes the database, Redis data, streams, pending delivery records, credentials
or foreign services, and never uninstalls a shared Redis executable. Re-enable
reuses retained resources and progress within transport retention. Pending
delivery is suspended while disabled, not silently settled.

## Use cases

### UC-MI1: Adopt core without background infrastructure

```gherkin
Given a developer adopts instrukt with todos, events and history off
When the configuration agent converges installation
Then no wake-up, Redis or webhook receiver is installed for that project
And documentation and configuration remain usable without a runtime
```

### UC-MI2: Install only the dependencies of enabled todos

```gherkin
Given a developer enables todos in a GitHub-backed project with a capable selected runtime
And no project enables events or history
When its configuration agent runs installation
Then one shared minute wake-up is registered unless ticks are off
And no Redis or receiver is installed solely for todos
```

### UC-MI3: Enable event responses without an inbound listener

```gherkin
Given a developer enables events with no webhook sources or an empty source map
When installation converges
Then local or explicit fallback Redis and cluster capabilities are checked with retained readers prepared
And the shared wake-up follows tick policy
And no receiver is installed
But when only history is enabled
Then installation checks cluster Redis without requiring events or preparing event consumers
And no wake-up or receiver is installed solely for history
```

### UC-MI4: Receive project webhooks only on the designated ingress

```gherkin
Given a developer enables events and configures a valid project source on the designated ingress
When the configuration agent installs its dependencies on participating computers
Then only ingress receives a loopback socket and receiver service with its declaring-project signing material
And other computers receive no listener
And sources with events off are reported without enabling events or installing their receiver
```

### UC-MI5: Reuse a working setup and repair only drift

```gherkin
Given a developer has working instrukt infrastructure and a configured operator-owned local Redis
When the configuration agent repeats installation
Then working units, credentials, data and consumer positions are reused
And the operator-owned Redis is unchanged
But when an instrukt-owned definition is missing or stale
Then installation repairs that difference and a subsequent check passes
```

### UC-MI6: Understand an unavailable local Redis prerequisite

```gherkin
Given local Redis setup fails before any local transport endpoint is established
When installation checks event dependencies
Then it names the cause and repair door
And it reports fallback only if the unconfigured-local cluster path actually works
But when an established local endpoint fails
Then its pending progress is retained and never silently rerouted
```

### UC-MI7: Keep infrastructure still needed by another project

```gherkin
Given two enabled projects share a wake-up or receiver
When one project disables the module and installation converges
Then resources still required by the other project remain available
And an unreadable project's demand prevents unsafe shared withdrawal
```

### UC-MI8: Keep intake available across receiver and runner restarts

```gherkin
Given an enabled ingress has its service-manager-owned loopback socket
When the receiver restarts while the runner is stopped
Then arriving connections wait on that socket within provider timeout
And the receiver resumes verified cluster publication independently of the runner
And existing consumer positions are not reset
```

### UC-MI9: See receiver placement and manager limits before binding

```gherkin
Given a computer is not ingress or lacks a supported socket-holding service manager
When receiver serving or required receiver installation is requested
Then it refuses the unsupported operation before binding and names the placement or manager prerequisite
And cron scheduling is not reported as receiver socket support
```

### UC-MI10: Supply wake-ups through an operator scheduler

```gherkin
Given a developer enables work and sets ticks off
When installation converges and the operator calls instrukt tick each minute
Then instrukt installs no periodic registration and status identifies operator scheduling
And normal task timing and event draining still run on those wake-ups
```

### UC-MI11: Withdraw unused units without losing retained work

```gherkin
Given a developer disables the last enabled user of instrukt-owned infrastructure
When installation converges the complete demand set
Then only unneeded instrukt-owned units are stopped and unregistered
And durable work, Redis data, credentials and pending activations remain
And foreign services and shared executables are unchanged
```

### UC-MI12: Converge the release-pinned blueprint engine without host dependence

```gherkin
Given a developer enables only blueprint for their adopted project
When the developer installs its dependencies with uv available
Then the pinned Graphify is installed with uv's tool and binary directories inside the instrukt home
And repeating installation reuses a matching version and repairs a missing, broken or drifted version
And no wake-up, Redis or receiver is installed
When installation is checked with a missing, broken or drifted Graphify
Then it reports unmet demand and changes no files
When Graphify needs installation and uv is absent or fails
Then installation reports the unmet prerequisite and its repair door
But with blueprint disabled
Then Graphify is neither installed nor required and an existing tool is retained
```

## Known caveats

- This contract supplies no always-on consumer daemon, runtime scheduling,
  remote installer, configuration-sharing service or configuration skill.
  Scheduled-only readers react on their next wake within stream retention.
- Native Redis provisioning, ACLs and socket activation need disposable macOS
  and Linux proof. A tick registrar or rendered template alone is not evidence
  that required infrastructure is installed and working.
- Runtime feasibility remains the procedure-event-delivery gate. Infrastructure
  health never proves agent hook delivery, revival or business completion.
- Package/privilege and unsupported service-manager prerequisites can block
  convergence; successful parts are reported separately and reruns reuse them.
- No release-pointer, daemon lifecycle, fleet update or legacy Redis config
  migration is imported from TeleClaude.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/task-timing](task-timing.md)
- [project/instrukt/spec/feature/runner/work-dispatch](work-dispatch.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](single-ingress-webhook-termination.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](../configuration/configuration-editing.md)
