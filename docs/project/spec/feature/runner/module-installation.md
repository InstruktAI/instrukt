---
description: Enabled adopted projects and company homes determine the shared wake-up, private local Redis, ingress receiver and runner service infrastructure installed, checked and repaired on each computer.
visibility: public
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

### Demand across adopted projects and company homes

Publication's adopted-project identities and registered checkout paths supply
the project set. Evaluate project-over-global module settings for each. A global
default without an adopted enabled project creates no infrastructure demand,
except through a company home, which follows the machine's global switches
(`project/instrukt/spec/feature/configuration/company-homes`): a nonempty valid
source map there demands events infrastructure and the receiver while events is
on, a declared connector demands the runner service, and an MCP server declared
with sign-in demands cluster Redis and a set `ingress.url` on the computer that
holds that home. The ingress computer serves the sign-in return whenever events
is on there and `ingress.url` is set, whichever homes it holds.
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
| A company home's declared connector | A selected capable runtime and the owned runner service that keeps the long-lived `instrukt run` running. No Redis or receiver solely for conversations. |
| Todos enabled | One shared minute wake-up unless `ticks: off`; selected capable runtime and existing GitHub repository/access/reflection prerequisites. No Redis or receiver solely for todos. |
| Events enabled, no sources | Shared wake-up unless off; private local Redis setup or verified operator local Redis; cluster transport check; retained consumer coverage prepared before producers. No receiver. |
| Events enabled with nonempty valid `webhooks:` in a project or a company home | The events dependencies, plus receiver/socket service on the designated ingress computer only. Other computers, the reacting computer included, retain consumers without listeners. |
| A company home's MCP server declared with sign-in (`project/instrukt/spec/feature/runner/mcp-access`) | On the computer that holds the home: cluster Redis reachable and `ingress.url` set. Without them it is an unmet finding naming the company and the server, with its door: set the missing settings, then `instrukt install`. No receiver on that computer solely for sign-in. |
| The ingress computer with events enabled and `ingress.url` set | The events dependencies and the receiver/socket service, which serves the sign-in return whether or not a source is declared. |

The local database is created/migrated on commands that need it under its own
contract. Todos may record finalize-ready events locally while events are off;
this creates no Redis dependency or event activation. Blueprint's hook behavior
adds no wake-up/Redis/receiver demand.

`modules.events` is authoritative. Sources with events off are a configuration
finding, never an implicit enablement. Empty `webhooks: {}` creates no endpoint
or receiver demand. `runner.autostart: false` leaves the enabled module's demand
intact: deliberate todo pickup still has reflection and timing responsibilities.

### Runtime projects of execution roots

A session starts in its execution root only where the selected runtime holds a
project rooted exactly at that root
(`project/instrukt/spec/feature/runner/session-continuity`, `execution_root`).
Every execution root whose sessions start on this computer demands one, under
one rule whatever the root is:

| Sessions start in | Module whose dependency set holds the runtime project |
| --- | --- |
| A company home with a declared connector | `conversations` |
| A company home where this computer activates standing procedures: events on, and not a computer that only receives | `events` |
| An adopted project's registered checkout with todos effective | `todos` |
| An adopted project's registered checkout with events effective, where this computer activates its procedures | `events` |

- `instrukt install` reuses each demanded root's runtime project and creates a
  missing one, once per canonical root however many rows demand it.
  `install --check` reports a missing one as unmet, naming the root and the
  door `instrukt install`, and creates nothing. An unreachable runtime leaves
  the resource unmet with its cause, and the next `install` completes it.
- A runtime project is never withdrawn, whatever the demand: the sessions it
  holds stay with it.
- Unfiltered installation and its check cover every row; a module filter
  converges or checks only the rows of its module.
- Installing ahead is not a precondition for work: a launch that finds its root
  without a runtime project adds it itself
  (`project/instrukt/spec/feature/runner/session-continuity`).

### Installation and inspection commands

| Command | Result |
| --- | --- |
| `instrukt config init [--global]` | Initialize the addressed configuration without installing infrastructure. |
| `instrukt config get [<path>] [--global] [--json]` and `instrukt config get --revision [--global]` | Read settings and revision with credentials redacted or references preserved. |
| `instrukt config patch --revision <rev> [--global] --from-file <file>` | Validate and atomically record desired configuration; standard input and `--yaml` retain their existing alternatives. Installs nothing. |
| `instrukt config validate [--global]` | Validate layer, effective module/runtime prerequisites and source references; names findings without exposing secret values. |
| `instrukt install [--module todos\|events\|history\|blueprint\|conversations]` | Read machine-wide demand, install/reuse/repair its resources and withdraw unneeded instrukt-owned units; report per-resource results and remaining findings. |
| `instrukt install --check [--module todos\|events\|history\|blueprint\|conversations]` | Read-only comparison of desired and actual infrastructure, capabilities, ownership and retained-reader readiness; nonzero for unmet demand or unsafe drift. Older configuration is converted in memory and reported as migrating on the next writing command. Creates no file, including logs, SQLite sidecars, locks or defaults; takes no lock and mutates no package, service, configuration, group position or data. |
| `instrukt ticks status` | Show registration mode, last wake and stale/missing wake findings. |
| `instrukt events status` | Show this computer's machine ID and its `ingress.computer` and `ingress.reactions` as it reads them, enabled projects, scoped Redis reachability, per-consumer stream positions/lag/pending, activation/delivery failures, undeliverable requested-review verdict notices with their count and the most recent ones, receiver placement/health, the runner service's state, the last skipped wake with its time and cause until a wake that is not skipped, and each connector's connection; masks credentials. |
| `instrukt events dead-letters` | Inspect retained dead-letter identity, original transport coordinate and failure facts without arbitrary Redis operations. |
| `instrukt webhooks serve` | Internal receiver entry point for owned service definitions; refuse before binding on non-ingress, or with neither an enabled valid source nor `ingress.url` set with events enabled. |

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
An interrupted install is completed by the next one: every definition is written
again whole under its name without checking proof of earlier writes.
Reports name installed, reused, repaired, withdrawn, externally supplied or
unmet resources, their paths/units and demanding project identities, without
expanded credentials. A failed prerequisite never produces an overall success.
Installation and `install --check` also report the configuration findings that
withhold a responsibility on this computer: event responses switched on without
`redis.cluster.url`, and a company connector without its published procedure,
its secret values or its provider permissions
(`project/instrukt/spec/feature/configuration/company-homes`).

They report in the same way a project whose declaration of regenerable output
is not admitted: reclaiming that output from its idle worktrees is withheld
for that project on this computer, while a delivered todo's worktree is still
released (`project/instrukt/spec/feature/configuration/layered-configuration`).

They report in the same way a retention window the selected runtime cannot
carry: thread retention is withheld on this computer while the runner's other
work runs (`project/instrukt/spec/feature/configuration/layered-configuration`).
The window adds no wake-up demand.

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

### Runner service

A connector is served only by a running `instrukt run`
(`project/instrukt/spec/feature/company/conversations`). While a company home
on the computer declares a valid connector, installation keeps one owned
service that runs
`instrukt run`: a launchd job on macOS, a systemd user service on Linux. The
service manager starts it at login or boot and restarts it when it exits. The
unit executes the installed instrukt binary with its configuration scope. Cron
cannot keep a process running; a computer with no supported service manager
has an unmet prerequisite naming it. On Linux it needs the user's service
manager to start at boot, as every owned unit does (Owned units without a
login, below).

The service's run is an ordinary `instrukt run`: it performs every enabled
module's work, and the runner lock still admits one runner per machine. While
it runs, the minute tick exits at once and a second `instrukt run` is refused
naming the service. While a run started by hand holds the lock, the service's
process waits for the lock for at most `RUN_LOCK_WAIT`, measured in
monotonic elapsed time. If the wait expires, it exits nonzero naming the
holder when known, and its manager starts it again. An acquired lock is kept
even when the wait bound has elapsed. Installation writes the service's
definition and restarts the service every time it runs, so the installed
executable and configuration are the ones running; it keeps no record of what the
running process executes, rewriting instrukt's own unit whole without consulting
any prior write record. `install --check` reports
a missing, stopped or unloaded service as unmet demand without changing it.

`instrukt events status` reports the service as `runner_service` in one of
four states, with the cause where there is one:

| State | Meaning |
| --- | --- |
| `running` | The service is loaded and its process is alive. It may be waiting for the runner lock. |
| `unmet` | There is demand, and its service is missing, stopped, unloaded or cannot be inspected. |
| `obsolete` | An owned service remains and there is no demand. |
| `no_demand` | There is no demand and no owned service. |

A company home that cannot be read leaves the demand unresolved. Nothing is
withdrawn on the strength of it, and where the state would otherwise be
`obsolete` or `no_demand` it is `unmet`, with that home as the cause.
Companies whose homes can be read are served as before.

Removing the last declared connector ends the demand; the next `instrukt install` withdraws the owned service under the
withdrawal and ownership rules below. A run started by hand is never the
service and is never stopped by withdrawal.

### Local and cluster Redis

Each module contributes its own Redis demand. History checks cluster-only
request/reply permissions (lists, transactions, expiry and readiness keys).
When history uses an existing instrukt-owned Redis as its cluster endpoint,
installation retains and maintains that service and its shared ACL even with
events off; it prepares no event consumers solely for history. The private
Redis's configuration and ACL are written from the stored secret on every
install and the service is restarted when they change; an existing secret is
reused and a missing one is generated, and the data Redis keeps stays reachable
with it, rewriting instrukt's own Redis files whole without checking proof of
earlier writes. Installation checks a Redis it started or restarted only
once it answers and has finished loading its data, waiting a bounded time for
both; past the bound the check reports what Redis answers. Read-only inspection reports a pending repair without changing
files or services. An unused local
event Redis is withdrawn when history points at an external cluster.

Where `redis.cluster.url` is configured and the computer has any demand —
history, events, an MCP server with sign-in, the minute wake-up or the runner
service — installation and `install --check` also check the fleet query's
permissions on the cluster user (`project/instrukt/spec/domain/runner`): setting
this computer's entry on the fleet membership record and reading the record,
and the evaluation kind's request/reply (lists, transactions and expiry). A
computer whose only runner is the minute `instrukt tick` is checked like one
that runs `instrukt run`. The proof is `ACL DRYRUN` on the production key
forms, as for history. When the cluster store is instrukt's own Redis, the ACL
installation writes for it grants these permissions, so convergence meets them.
For an operator-supplied cluster store a denial is an unmet finding with its
door: grant the fleet query's permissions to the cluster Redis user, then
`instrukt install`.
A computer without a cluster connection is not checked: it keeps no
membership entry and answers no fleet request. A module filter checks these
permissions when it covers one of those demands.

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
or an enabled company home's nonempty valid source map exists there, or events
is enabled there and `ingress.url` is set. The sign-in return needs no company
home on the ingress computer: the receiver knows no company, checks only the
return's form and path, and writes its short-lived entry
(`project/instrukt/spec/feature/runner/mcp-access`). Missing
declaring homes or signing values fail with the home/reference named. Signing
material resolves only on the ingress computer, from its owner-only
`~/.instrukt/secrets.env` for a project source and from the company home's own
secrets file for a company source; other computers need no signing values. Ticks and interactive paths use that same
file for their required signing, Redis and other instrukt secrets.
All participants agree on ingress ID, reacting computer ID and cluster Redis
endpoint plus database; local sockets, runtime and tick policy remain local.
Configuration management distributes those facts; instrukt has no shared
configuration layer. The reacting computer, when it is not the ingress,
installs no receiver and needs no signing values; procedures for external facts
need the homes they run in and the chosen runtime there, and the ingress needs
neither (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`).

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
The receiver's invocation is written on every install and the receiver is
restarted; the receiver validates the invocation it reads at its boundary.
Same-port restarts retain the registered listening socket; health establishes
that the receiver rejects an unsigned request. See the
[receiver activation implementation](../../../../../crates/instrukt-runner/src/installation/_launch.rs)
for the concrete mechanism. A port change
registers and checks the replacement listener before withdrawing the prior owned
units, and reports the edge-route change for operator alignment without waiting
for the operator. An interrupted replacement is completed by the next
installation run.

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

### Owned units without a login

On Linux every owned unit is a systemd user unit that has to run without a
login: the runner service, the receiver's socket and service, the private local
Redis service and the minute timer. They need the user's service manager to
start at boot. Where it does not, installation and `install --check` report
one unmet prerequisite naming `loginctl enable-linger` and the owned units that
need it, and installation does not change it. A wake-up registered with cron
needs no such setting.

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

A unit, definition file, crontab line or private Redis file under instrukt's own
name for this home is instrukt's: it is written, reloaded or withdrawn by that
name, under whichever service manager present on the computer holds it. A home
other than `~/.instrukt` carries a suffix in those names, from its canonical path;
the default home keeps today's names. An earlier unit without the suffix that names
this non-default home in its environment is withdrawn by this home's install; the
default home never touches a unit naming another home. Anything under another name
is never read or touched; an incomplete demand still withdraws nothing
(instrukt writes and withdraws only its own named places without consulting
earlier write records). Cron withdrawal replaces or removes only the line marked with this home's name.
Withdrawal stops/unregisters owned units only; it never
deletes the database, Redis data, streams, pending delivery records, credentials
or foreign services, and never uninstalls a shared Redis executable. Re-enable
reuses retained resources and progress within transport retention. Pending
delivery is suspended while disabled, not silently settled.

### Scheduler locations and generated environments

One runner-owned `SchedulerLocations` resolves all scheduler writes, reads,
inspection and withdrawal. Systemd unit files use the first explicitly supplied
absolute entry of native `SYSTEMD_UNIT_PATH`; remaining entries and trailing
colon retain native search semantics. Malformed explicit paths fail rather than
falling back. Without an override, use normal XDG user configuration. Before
registration, the running manager's decoded `UnitPath` must include the selected
directory; installer environment alone does not reconfigure the manager.
`INSTRUKT_LAUNCH_AGENTS_DIR` selects an absolute plist directory, defaulting to
inherited HOME's `Library/LaunchAgents`. Alternate files are loaded explicitly
and are not automatically scanned at login.
`INSTRUKT_CRONTAB_FILE` selects an absolute, atomically written table file;
missing means empty, other errors fail. File mode preserves unrelated lines,
never invokes crontab and reports `file_only`, not active scheduling. Unset uses
the native executing-account table through crontab, never a guessed spool path.
Generated tick/runner/receiver environments retain inherited HOME and propagate
INSTRUKT_HOME, T3CODE_HOME, selected native runtime variables and all three
scheduler-location inputs when present. Optional values stay absent when unset.

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
And other computers, the reacting computer included, receive no listener
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
But when a Linux computer's user service manager does not start at boot
Then installation and its check report that prerequisite naming loginctl enable-linger for each owned unit that needs it
And installation does not change it
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

### UC-MI14: Serve sign-in returns without a webhook source

```gherkin
Given a developer declares a company's MCP server with sign-in and no webhook source anywhere
And the ingress computer has events enabled and the ingress address set, and holds no company home
When the configuration agent installs the demanded infrastructure on the ingress computer and the company's computer
Then the ingress computer runs the receiver that takes sign-in returns
But when the company's computer has no ingress address or cannot reach cluster Redis
Then installation reports the company and the server with the setting to supply, and claims no success
```

### UC-MI13: Keep the runner running for a company's conversations

```gherkin
Given a developer has a company home that declares a connector
When they install the demanded infrastructure and the computer later restarts
Then an owned service has the runner running again without anyone starting it
But when the computer has no service manager that can keep a process running
Then installation reports that prerequisite and claims no success
When a run started by hand keeps the runner lock past the service's wait bound
Then the service invocation exits nonzero naming that holder
But if the service acquires the lock when it next checks, it runs even when the wait bound has elapsed
```

### UC-MI15: Install over definitions instrukt wrote without its records

```gherkin
Given instrukt's runner service definition exists from an earlier install, has been changed, and instrukt's records of it are gone
When the developer installs
Then the service is written again, restarted and running
And a definition under another name is left as it was
And a second instrukt home on the computer installs under its own names without touching the first
```

## Known caveats

- Outside conversation demand this contract keeps no process running:
  scheduled-only readers react on their next wake within stream retention. It
  supplies no runtime scheduling, remote installer, configuration-sharing
  service or configuration skill.
- Native Redis provisioning, ACLs and socket activation need disposable macOS
  and Linux proof. A tick registrar or rendered template alone is not evidence
  that required infrastructure is installed and working.
- Runtime feasibility remains the procedure-event-delivery gate. Infrastructure
  health never proves agent hook delivery, revival or business completion.
- Package/privilege and unsupported service-manager prerequisites can block
  convergence; successful parts are reported separately and reruns reuse them.
- There is no release-pointer, fleet update or legacy Redis config migration,
  and no daemon lifecycle beyond the owned units this contract installs and
  withdraws.
- Installation and `install --check` judge this computer's own settings
  only: computers that disagree on the ingress or the reacting computer each
  pass, and what follows is the owner's caveat
  (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`).

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/task-timing](task-timing.md)
- [project/instrukt/spec/feature/runner/work-dispatch](work-dispatch.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](single-ingress-webhook-termination.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](../configuration/configuration-editing.md)
