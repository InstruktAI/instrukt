---
description: '`instrukt.yaml` in a global and a project layer, and a company home''s own document — which key lives where, how the layers combine, validation, schema versions and migration.'
---

# Layered Configuration — Spec

## What it is

`instrukt.yaml` uses one schema read in two layers,
like git's user and repository configuration. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and what only the
machine may decide; the project layer (`instrukt.yaml` at the repository root)
holds what the project declares, and overrides the global layer where a key
may live in both. Validation knows, per key, which layer it may live in.
Each layer carries the `schema_version` of its own kind's schema, and older
documents are migrated.
Conversion retains supported existing settings. Omitted dependency-update branch selection follows
`DEFAULT_DEPENDENCY_BRANCH_PREFIX` in configuration's schema declaration
(`crates/instrukt-core/src/config/_schema.rs`).

## Canonical fields

`runner.runtime` chooses a compiled adapter. Dispatch preferences are checked
against the selected runtime's native and adapter capabilities, never executable
configuration. Agent-native instructions are configuration prose rendered at read time;
they are not adapter command templates.

### History enablement

`modules.history` is a boolean allowed in both layers and defaults on; an
explicit project value overrides the global value. Operational enablement is
owned by `project/instrukt/spec/feature/runner/history`; infrastructure demand
is owned by `project/instrukt/spec/feature/runner/module-installation`.

### Event enablement

The effective `modules.events` switch is authoritative. Subscriptions remain in
published documentation while off and produce no activation. A nonempty valid
`webhooks:` map in a project layer or a company home adds receiver demand only
while events are enabled for it (the effective switch for a project, the
machine's global switch for a company home); a declaration with events off
produces a plain validation finding naming that switch. An empty map adds no receiver demand. Source declarations never silently
enable events. Installation and withdrawal are owned by
`project/instrukt/spec/feature/runner/module-installation`; these schema rules install nothing.

### Automatic review enablement

`review.automatic` is a boolean allowed in the project layer only and defaults
off. On, it opts that project in to automatic independent review: the runner
observes its pull requests and the integration candidate carries the review
status (`project/instrukt/spec/feature/runner/pull-request-review`,
`project/instrukt/spec/feature/workflow/integration-gate`). A global value is a
finding: no machine setting opts a repository in. The setting is independent of
`modules.todos`.

Each of these is a finding at `review.automatic`, in plain words naming what is
missing:

- the project's effective `modules.events` is off, since a review request is
  an event response;
- the project's code is not on GitHub
  (`project/instrukt/spec/feature/work/github-record`);
- the active published set for the project does not route
  `runner.review_needed` to `sdlc/procedure/lifecycle/work/review-commits` and
  to nothing else: that procedure is active in the set, subscribes by that
  exact type and declares the `commit-reviewer` role, the role whose App
  publishes the verdict, and it is the only procedure subscribed to the type
  (`project/instrukt/spec/feature/runner/procedure-subscription-routing`,
  `project/instrukt/spec/feature/work/github-apps`). A pattern that merely
  matches the type, another procedure in its place whatever role it declares,
  that procedure under another role, and a second subscribed procedure each
  leave the finding standing;
- work's readiness for automatic review reports an unmet finding for the
  project identity or the reviewer identity on the repository
  (`project/instrukt/spec/feature/work/github-apps`).

`config patch` refuses a write that would switch it on with such a finding,
under its revision check and whole-document validation
(`project/instrukt/spec/feature/configuration/configuration-editing`).
`config validate` and `install --check` report it without changing anything
(`project/instrukt/spec/feature/runner/module-installation`). A project layer
that arrives with it through a commit is not admitted for automatic review on
that computer; its other capabilities are unaffected
(`project/instrukt/policy/checks-and-guards`). The readiness findings need
GitHub; while GitHub cannot be asked, admission waits and retries rather than
refusing or admitting.

### Regenerable output declaration

`regenerable` is a list of paths allowed in the project layer only; absent or
empty, the project declares nothing. Each entry names one location inside a
checkout of the project, relative to the checkout's root, that the project
rebuilds from its committed sources. Worktree reclamation is its only reader
and owns what a removal does
(`project/instrukt/spec/feature/runner/worktree-reclamation`). The key holds
locations and never a command: a project layer decides nothing that runs on
the machine.

An entry is a finding at `regenerable`, naming the entry, when it is empty or
absolute, steps to a parent directory, names the checkout's root itself, or
names the repository's `.git` or a location under it. A global value is a
finding.

`config patch` refuses a write that would introduce such an entry, under its
revision check and whole-document validation
(`project/instrukt/spec/feature/configuration/configuration-editing`).
`config validate` reports it without changing anything. A project layer that
arrives with it through a commit is not admitted for reclaiming declared
output on that computer; the project's other capabilities are unaffected
(`project/instrukt/policy/checks-and-guards`). Installation inspection reports
the withheld reclamation
(`project/instrukt/spec/feature/runner/module-installation`), and no declared
output is removed without an admitted declaration; the release of a delivered
todo's worktree does not depend on it
(`project/instrukt/spec/feature/runner/worktree-reclamation`).

### Layer rules

| Key | Layer | Meaning owned by |
| --- | --- | --- |
| `schema_version` | each document | configuration |
| `agents.<name>.git_author` | global only | commit composition |
| `modules: {blueprint, todos, events, history}` (booleans; history defaults on, others off; core has no switch) | both (project overrides global) | each module |
| `ci.dependency_branch_prefixes` (list of nonempty strings; `[]` disables dependency-branch observation) | project only | runner CI observation |
| `events.watch: [{remote, ref}]` | project only | runner |
| `webhooks.<domain>` | project layer or company home; required `secret`, optional `previous_secret` references, `verify` (native default, github, hmac-sha256), generic `header`, exclusive `name_from` or `name` | runner intake |
| `company` | company home only; equals the home folder's name | configuration |
| `members.<member>` (`name`, `email`, `credentials.<tool>` references) | company home only | configuration stores and validates; runner resolves a speaker's tool logins |
| `profiles` (ordered list of profile names the machine's `agents.yaml` declares) | company home only | configuration validates the names; agent selection uses the order for sessions in that home |
| `connectors.<name>` (optional `kind` defaulting to `<name>`, optional `role` defaulting to `company-agent`, optional `admit` (`members` \| `admins_only`), optional `session_scope` (`conversation` \| `company`), optional `reply_mode` (`turn_output` \| `explicit`), plus the provider fields that `kind` declares; `slack`'s are in `project/instrukt/spec/feature/company/slack-connector`) | company home only | company conversations |
| `mcp_clients.<name>` (required `issuer` authorization server URL, required `id` and optional `secret` references into `~/.instrukt/secrets.env`, optional `description`) | global only; reusable host-level OAuth clients pinned to one authorization server issuer that companies reference via `sign_in.client_ref` | runner MCP access |
| `mcp.<name>` (`url`, `description`; in a company home `speaker_login` (`stored` with `credential`, or `sign-in` with optional `sign_in` mode, scopes, host `client_ref` or local `client` references, and identity request) and a `login` reference; in the global layer a `login` reference) | global layer or company home; a company home's declaration replaces the machine-wide one of the same name for that company | runner MCP access |
| `issuer` (`enabled` boolean, optional `signing_key` `${NAME}` reference into the company home's `secrets.env`) | company home only; lightweight built-in per-company OIDC issuer for MCP Enterprise-Managed Authorization (`jwt-bearer`) | runner MCP access |
| `ingress.computer`, `ingress.port`, `ingress.reactions`, `ingress.url` | global only; stable existing machine ID of the ingress, loopback port, stable existing machine ID of the computer that reacts to external facts (default: the ingress), and the public `https` address at which the operator's edge forwards the receiver's paths | runner intake and MCP sign-in |
| `redis.cluster`, `redis.local` | global only; `url`, optional `username`/`password` references and connection limits/timeouts | runner transport |
| `ticks` | global only | the runner |
| `blueprint.engine` | both; the project overrides, but may only name a shipped adapter; the command form is global only | the `blueprint` module |
| `runner.notify` | global only | the runner |
| `runner.runtime` (compiled adapter name; unset, the one compiled adapter, initially `t3`) | global only | runner |
| `runner.archive_finished_after` (retention window) | global only | runner thread retention |
| `runtimes.<name>` (agent-action instruction values) | global only; complete value set below | configuration and the agent consuming the rendered instruction |
| `runner.autostart` (boolean or map `{<kind>: bool}`) | both; the project overrides | runner |
| `project` | project only | docs and publication |
| `requires` (modules the flavor needs) | project only | publication |
| `board` (GitHub project `<owner>/<number>`; automatically set up and repaired with views) | project only | the `todos` module |
| `github.apps` (App/bot public identities and protected private-key references), `github.project_app` (fallback App reference) | global only | work GitHub boundary; configuration stores and validates declarations |
| `github.approvers` (GitHub logins) | project only | work |
| `github.repository` (`owner/repo`; default derived from the remote the default branch tracks) | project only | work, runner |
| `github.api_url` (default `https://api.github.com`) | global only | work, workflow, runner |
| `approvals.prd`, `approvals.task` (`human` or `peer`, each default `human`) | project only | workflow |
| `workflow.domain` (published domain name, default `sdlc`; a domain published from some project's `docs/domain/<x>/`) | project only | workflow |

Runtime choice is global-only: a cloned repository does not decide which runtime
executes on the machine. Procedures run checks as the project documents them;
there is no project-layer `actions` key. CI required checks remain the merge gate.

### Event configuration values and cross-computer agreement

`redis.cluster.url` is required where event responses are on and identifies one shared
logical database/stream namespace, not a sharded Redis Cluster client. Local URL
is Unix-socket-only; absence uses cluster visibly. `max_connections` and
`socket_timeout` follow the port's connection settings, with timeout outlasting
the blocking read. Credentials remain references and expanded values are masked
in reads and findings. An unavailable configured local endpoint is not absence.

Where the effective configuration turns event responses on and the machine layer
has no `redis.cluster.url`, that is a finding at `modules.events`: `config patch`
refuses it, `config validate` and `install --check` report it, and a project layer
that arrives with it through a commit is not admitted for event responses on that
computer; its todos and history are unaffected
(`project/instrukt/policy/checks-and-guards`: a prerequisite is checked where it
enters, never discovered by the running runner).

`ingress.computer` is the existing stable instrukt machine ID. `ingress.reactions`
names, by the same kind of ID, the one computer that activates procedures for
external facts; left out, the ingress reacts (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`). All participating
machine global layers agree on both IDs and the logical cluster Redis/namespace;
URL spellings and ACL credentials may differ when they reach the same logical
store with required permissions. `ingress.port` is a valid TCP port shared only
by the ingress receiver and local edge. The bind host is fixed loopback.
`ingress.url` is an `https` address with a host, an optional port and no
credentials, path, query or fragment; the edge forwards `/hooks/inbound` below it to the
receiver, and a sign-in's return address is built from it
(`project/instrukt/spec/feature/runner/mcp-access`). All participating
computers agree on it as on `ingress.computer`. Local
Redis socket paths, local credentials, runtime and ticks settings stay local.
`instrukt events status` shows each computer's own machine ID and both IDs as
that computer reads them (`project/instrukt/spec/feature/runner/module-installation`);
agreement is established when the computers are configured
(`project/instrukt/spec/feature/configuration/configuration-skill`), and a
computer validates only its own settings. No shared
configuration layer exists, and no key routes facts per project, per home or to
a project's owner computer.

Project identities and checkouts and company homes needed for activation must
exist on the activating computer: the emitting computer for its own events, the
reacting computer for external facts. The ingress needs each declaring home's
source declaration; it needs a home's procedures only when it is also the
reacting computer. A project source's signing values needed on ingress resolve
from the one owner-only machine-local
`~/.instrukt/secrets.env`; a company source's resolve from its home's own
secrets file (`project/instrukt/spec/feature/configuration/company-homes`). Source secret fields
are exact raw `${NAME}` references; the same file also supplies Redis and all
other instrukt secrets on every entry path. Project `.env` and process
environment supply no fallback. The protected read/write contract is
`project/instrukt/spec/feature/configuration/owned-files`. A declared previous secret follows the same
rules as current. Configuration reads preserve references and never expand
source signing material. Source edits apply at the next request. The domain key selects
the declaring home's `/hooks/inbound/<kind>/<slug>/<domain>` and the emitted type prefix; it is
never taken from a body field. Sources use native `{name, payload}`, a payload
field through `name_from`, fixed `name`, or GitHub normalization through
`verify: github`, as defined by the inbound framework.

Invalid domains, unknown verification schemes, conflicting name sources, misplaced keys, malformed references, unreadable ingress
values and configured sources with effective events off are findings. Unknown
keys remain findings; no direction or `sources` subkey is accepted. The typed
configuration also generates editor schemas. Schema validation itself starts
no receiver, Redis or scheduler; infrastructure convergence belongs to
`project/instrukt/spec/feature/runner/module-installation`.

### Enablement and installation demand

What the enabled settings of adopted projects and company homes install on a
computer is owned by
`project/instrukt/spec/feature/runner/module-installation`. `ticks: on` permits
the shared minute wake-up only while that demand needs it.

A revision-checked patch records desired enablement, never successful setup.
The configuration agent follows it with `instrukt install` to converge dependencies
and `instrukt install --check` to verify them. Disable stops workload admission;
explicit convergence removes only owned units with no remaining enabled demand,
retaining data and foreign services. Disabling events with sources declared
requires removing those source declarations in the same valid patch. No new
installer toggle, shared configuration layer or runtime detection is added.

### Runtime choice

The developer may choose global `runner.runtime` by compiled adapter name. `t3`
selects the shipped T3 implementation; an unknown name is a validation finding.
An unset value selects the one runtime compiled into this instrukt. No runtime is
detected from installed tools or from a caller inside a runtime's session.

`runtimes` is a map keyed by runtime name. Each entry holds the named nonempty
agent-action instruction values below. `runner.runtime` selects that entry;
`runtime.<name>` resolves its value at read time. Names and inputs are
runtime-neutral. The entry supplies either its native instruction or an instrukt
route for a capability with no native counterpart. The consuming doc, action,
template or skill references only `@config: runtime.<name>`; it embeds neither
native tool calls nor fallback commands. Instrukt executes none of this prose.
No command/agent verb kind or agent-action exit-status router exists.

### Agent-action values

<!-- planned-change:delegator-reply-checks -->
| Value under `runtimes.<runtime>` | What its instruction conveys |
| --- | --- |
| `dispatch` | Delegate this agent's worker using the supplied brief, role, attempt, semantic key, display title and preferences with shared isolation in the parent's recorded workspace. Return native child identity and matching workspace/branch/head evidence; retain native parent linkage. |
| `message_child` | Send a bounded follow-up, findings, review continuation or recovery to this parent's identified child within its outstanding attempt, or the brief of a newly reserved attempt already bound to that child, using the native continuation behavior when idle. |
| `message_peer` | Send the supplied substantive text to an addressed peer/coordinator conversation, preserving the target and source attribution. |
| `read_thread` | Read an identified conversation's relevant messages and attribution; state extent, pagination and unavailable evidence. This is not history search. |
| `inspect_thread` | Inspect an identified thread or own child/task's state/result, distinguishing unavailable observation, input wait, failure and confirmed absence. |
| `look_around` | Discover current conversations in the supplied project/scope, including native current-thread metadata search and settled threads where supported. It releases no archive/history search service. |
| `steer` | Deliver current direction at the target's supported safe input boundary while its turn and children continue; state queued/entry degradation when native interjection is absent. |
| `discuss` | Open, participate in and close a runner-owned conversation link between two peer threads; the person participates in either thread and the runner relays finished turns with attribution. |
| `start_thread` | Start a separately addressed conversation with its supplied project, semantic key, display title, initialized brief and placement. Repository todos use a single `<slug>/work` worktree; tasks create no git worktree. A worker uses `dispatch`. |
| `cancel_child` | Stop this parent's identified child under its existing mandate, retaining its work/evidence and distinguishing accepted cancellation from completion. |
| `finish_thread` | Dormantly settle an addressed conversation, or this agent's own conversation where the runtime supports it, so it stays wakeable in place with full context: a resting conversation, or the orchestrator thread of a todo that was delivered or withdrawn. Retain open obligations, forbid external agents and runner automation from settling a standing agent (a standing procedure may self-settle when parked dormant), and apply self-settling at its supported turn boundary. |
| `discover_environments` | Discover connected computers/environments, their project locations, access and task capabilities; keep runtime addresses distinct from instrukt machine IDs. |
| `connect_environment` | Request the runtime's native connection/authentication flow for a named missing computer; report unresolved access as a prerequisite without inventing an address. |
| `dispatch_remote` | Assign a local brief and report destination to an explicitly addressed connected environment, under its project and permissions; follow native completion evidence and retain partial results. |
<!-- change:delegator-reply-checks -->
| Value under `runtimes.<runtime>` | What its instruction conveys |
| --- | --- |
| `dispatch` | Delegate this agent's worker using the supplied brief, role, attempt, semantic key, display title and preferences with shared isolation in the parent's recorded workspace. Return native child identity and matching workspace/branch/head evidence; retain native parent linkage. |
| `message_child` | Send a bounded follow-up, findings, review continuation or recovery to this parent's identified child within its outstanding attempt, or the brief of a newly reserved attempt already bound to that child, using the native continuation behavior when idle. |
| `message_peer` | Send the supplied substantive text to an addressed peer/coordinator conversation, preserving the target and source attribution. |
| `read_thread` | Read an identified conversation's relevant messages and attribution; state extent, pagination and unavailable evidence. This is not history search. |
| `inspect_thread` | Inspect an identified thread or own child/task's state/result, distinguishing unavailable observation, input wait, failure and confirmed absence. A scheduled reply check is permitted inspection of a running child. |
| `schedule_check` | Set, update or clear this agent's one scheduled reply check (`general/procedure/ai-to-ai-operations`) through the runtime's scheduler, at the interval the agent supplies, carrying its own identity so a later run can clear it. |
| `look_around` | Discover current conversations in the supplied project/scope, including native current-thread metadata search and settled threads where supported. It releases no archive/history search service. |
| `steer` | Deliver current direction at the target's supported safe input boundary while its turn and children continue; state queued/entry degradation when native interjection is absent. |
| `discuss` | Open, participate in and close a runner-owned conversation link between two peer threads; the person participates in either thread and the runner relays finished turns with attribution. |
| `start_thread` | Start a separately addressed conversation with its supplied project, semantic key, display title, initialized brief and placement. Repository todos use a single `<slug>/work` worktree; tasks create no git worktree. A worker uses `dispatch`. |
| `cancel_child` | Stop this parent's identified child under its existing mandate, retaining its work/evidence and distinguishing accepted cancellation from completion. |
| `finish_thread` | Dormantly settle an addressed conversation, or this agent's own conversation where the runtime supports it, so it stays wakeable in place with full context: a resting conversation, or the orchestrator thread of a todo that was delivered or withdrawn. Retain open obligations, forbid external agents and runner automation from settling a standing agent (a standing procedure may self-settle when parked dormant), and apply self-settling at its supported turn boundary. |
| `discover_environments` | Discover connected computers/environments, their project locations, access and task capabilities; keep runtime addresses distinct from instrukt machine IDs. |
| `connect_environment` | Request the runtime's native connection/authentication flow for a named missing computer; report unresolved access as a prerequisite without inventing an address. |
| `dispatch_remote` | Assign a local brief and report destination to an explicitly addressed connected environment, under its project and permissions; follow native completion evidence and retain partial results. |
<!-- /planned-change:delegator-reply-checks -->

Each entry supplies the complete set, including a clear unsupported/prerequisite
instruction where neither a native counterpart nor an implemented instrukt route
exists. An unsupported instruction grants no capability. Unknown names, empty
values and project-layer entries are findings. A missing selected entry or value
fails `@config` lookup visibly. Native-capability availability and unattended
adapter availability remain distinct typed facts; no code infers them from prose.
All agent-action instructions, including fallback routes and runtime-specific
worktree constraints, are configuration data. The shipped entry is a data
resource parsed by configuration, not Rust string constants or tool-selection
branches. Changing one of these values changes what a future read renders;
it never reconfigures an existing initialized standing assignment.

### Shipped T3 entry values

The following table specifies the instruction text carried by the `t3` entry.
Only the configuration entry carries the runtime-specific execution wording;
agent-facing consumers retrieve the value through its alias.

<!-- planned-change:delegator-reply-checks -->
| Value | T3 entry instruction |
| --- | --- |
| `dispatch` | Use native delegated-task dispatch with shared isolation from this conversation, preserving its recorded workspace. A repository todo shares one `<slug>/work` worktree; all repository children inherit `todo-orchestrator`'s workspace (`kind: task` creates no git worktree). Use the supplied brief, semantic key, title and role-attempt facts. Include the role/attempt identity in the child brief and require it to await confirmed instrukt binding before workspace changes, record writes or stamps. Retain the native child/task id and matching workspace/branch/head evidence for instrukt binding. Follow native completion delivery. |
| `message_child` | Message or continue the identified child natively, as the `message_child` contract above requires: a follow-up within an outstanding attempt stays under that role/attempt, and a newly reserved attempt is bound to the child through instrukt before its brief is sent. The native wording is owned by the shipped entry, `crates/instrukt-core/src/config/shipped/t3.yaml`. |
| `message_peer` | Use native thread messaging for the supplied peer/coordinator thread id and text. Preserve its identity and the message's attribution. |
| `read_thread` | Read the identified thread with native thread-read operations, including the relevant message range and attribution. Report pagination or unavailable evidence; read no archive/history service. |
| `inspect_thread` | Read the identified native thread or own delegated task through its native status/result operation. Follow native completion/wake instructions; inspect a running task only when those instructions permit it. |
| `look_around` | Use native project thread listing and available current-thread metadata filters. Select the requested project and include settled threads when needed. Retain the runtime's visibility limits for children; listing absence proves no missing child. |
| `steer` | Use native thread/child input with its supported steering mode. Preserve the running turn and children. For a target that queues rather than interjects, hand over now for its next supported turn. |
| `discuss` | When asked to open a discussion, use `instrukt discuss open <thread> [--message <text>]`; the native caller is the other member. Use `instrukt discuss close [--message <text>]` to deliver an optional final contribution and close for both members. The running instrukt runner shares finished turns automatically under project/instrukt/spec/feature/runner/conversation-links. |
| `start_thread` | Use supplied placement.selection profile/kind/instance/model/effort/options for native startup where supported. The host CLI passes --provider and --model and records its effort/options flag gap. For a repository todo, start `t3 thread start <project> - --worktree origin/main --branch <slug>/work --title <display-title>` (on another computer use `--worktree origin/<slug>/work --branch <slug>/work`; legacy un-integrated `<slug>/prepare` branches are carried forward automatically). instrukt derives these names; creation refuses an existing local branch. Native shared children inherit `<slug>/work`. After deliberate native startup, enter `instrukt todo work <slug> --placement-evidence <file\|->` containing native session, key, title, recorded worktree, branch, head and source; shell cwd alone is not placement evidence. Wake a settled retained orchestrator in place. The CLI cannot attach an existing worktree to a replacement thread; resolve that placement or use native MCP existing-worktree start. Ad hoc work uses its own one-off worktree; only the integrator writes main. |
| `cancel_child` | Use native delegated-task cancellation for the supplied own-child task id, preserving its branch, workspace and unfinished evidence. Read the returned cancellation state before deciding its remaining work. |
| `finish_thread` | Preserve unfinished obligations. External agents and runner automation never settle a standing agent; a standing procedure may self-settle when parked dormant with no pending work, and other threads, including a resting conversation and a delivered or withdrawn todo's orchestrator thread, use native settling, which applies when the current turn ends. A settled standing thread stays reachable and wakes in place on a message. |
| `discover_environments` | Use the connection's native linked-environment discovery. Match participating computers to observed environment ids and registered project paths, keeping instrukt machine IDs distinct. Report missing mappings and access. |
| `connect_environment` | Use native environment linking with the named computer as its hint and follow the connection's authentication flow. Report any unresolved operator prerequisite. |
| `dispatch_remote` | Use the connection's native task operation explicitly targeting the identified linked environment and registered project. Supply the local configuration brief, skill package location/text and report destination; preserve local permission limits. Follow its completion instructions and compare returned verification across computers, retaining partial success. |
<!-- change:delegator-reply-checks -->
| Value | T3 entry instruction |
| --- | --- |
| `dispatch` | Use native delegated-task dispatch with shared isolation from this conversation, preserving its recorded workspace. A repository todo shares one `<slug>/work` worktree; all repository children inherit `todo-orchestrator`'s workspace (`kind: task` creates no git worktree). Use the supplied brief, semantic key, title and role-attempt facts. Include the role/attempt identity in the child brief and require it to await confirmed instrukt binding before workspace changes, record writes or stamps. Retain the native child/task id and matching workspace/branch/head evidence for instrukt binding. Follow native completion delivery. |
| `message_child` | Message or continue the identified child natively, as the `message_child` contract above requires: a follow-up within an outstanding attempt stays under that role/attempt, and a newly reserved attempt is bound to the child through instrukt before its brief is sent. The native wording is owned by the shipped entry, `crates/instrukt-core/src/config/shipped/t3.yaml`. |
| `message_peer` | Use native thread messaging for the supplied peer/coordinator thread id and text. Preserve its identity and the message's attribution. |
| `read_thread` | Read the identified thread with native thread-read operations, including the relevant message range and attribution. Report pagination or unavailable evidence; read no archive/history service. |
| `inspect_thread` | Read the identified native thread or own delegated task through its native status/result operation. Follow native completion/wake instructions; inspect a running task only when those instructions permit it or your scheduled reply check fires. |
| `schedule_check` | Set, update or clear the check with native scheduled tasks bound to this thread and running in its own session, as the `schedule_check` contract above requires. The native wording is owned by the shipped entry, `crates/instrukt-core/src/config/shipped/t3.yaml`. |
| `look_around` | Use native project thread listing and available current-thread metadata filters. Select the requested project and include settled threads when needed. Retain the runtime's visibility limits for children; listing absence proves no missing child. |
| `steer` | Use native thread/child input with its supported steering mode. Preserve the running turn and children. For a target that queues rather than interjects, hand over now for its next supported turn. |
| `discuss` | When asked to open a discussion, use `instrukt discuss open <thread> [--message <text>]`; the native caller is the other member. Use `instrukt discuss close [--message <text>]` to deliver an optional final contribution and close for both members. The running instrukt runner shares finished turns automatically under project/instrukt/spec/feature/runner/conversation-links. |
| `start_thread` | Use supplied placement.selection profile/kind/instance/model/effort/options for native startup where supported. The host CLI passes --provider and --model and records its effort/options flag gap. For a repository todo, start `t3 thread start <project> - --worktree origin/main --branch <slug>/work --title <display-title>` (on another computer use `--worktree origin/<slug>/work --branch <slug>/work`; legacy un-integrated `<slug>/prepare` branches are carried forward automatically). instrukt derives these names; creation refuses an existing local branch. Native shared children inherit `<slug>/work`. After deliberate native startup, enter `instrukt todo work <slug> --placement-evidence <file\|->` containing native session, key, title, recorded worktree, branch, head and source; shell cwd alone is not placement evidence. Wake a settled retained orchestrator in place. The CLI cannot attach an existing worktree to a replacement thread; resolve that placement or use native MCP existing-worktree start. Ad hoc work uses its own one-off worktree; only the integrator writes main. |
| `cancel_child` | Use native delegated-task cancellation for the supplied own-child task id, preserving its branch, workspace and unfinished evidence. Read the returned cancellation state before deciding its remaining work. |
| `finish_thread` | Preserve unfinished obligations. External agents and runner automation never settle a standing agent; a standing procedure may self-settle when parked dormant with no pending work, and other threads, including a resting conversation and a delivered or withdrawn todo's orchestrator thread, use native settling, which applies when the current turn ends. A settled standing thread stays reachable and wakes in place on a message. |
| `discover_environments` | Use the connection's native linked-environment discovery. Match participating computers to observed environment ids and registered project paths, keeping instrukt machine IDs distinct. Report missing mappings and access. |
| `connect_environment` | Use native environment linking with the named computer as its hint and follow the connection's authentication flow. Report any unresolved operator prerequisite. |
| `dispatch_remote` | Use the connection's native task operation explicitly targeting the identified linked environment and registered project. Supply the local configuration brief, skill package location/text and report destination; preserve local permission limits. Follow its completion instructions and compare returned verification across computers, retaining partial success. |
<!-- /planned-change:delegator-reply-checks -->

Selection and entry values stay global; a project cannot replace either. A
shipped entry never chooses the runtime on the developer's behalf. T3's host CLI
continues serving unattended adapter starts independently of these agent values;
caller-identity recognition remains an environment-only code operation.

Feature enablement is checked against the selected runtime's native/adapter capabilities
(`project/instrukt/spec/feature/runner/session-continuity`). Without a selected
runtime, enabling runtime-dependent features is refused with that reason. A
missing capability refuses and names the runtime and capability. Configuration
reads and documentation remain usable without a runtime.

### Agent preferences and availability

Model selection is role-keyed configuration in `~/.instrukt/agents.yaml`,
independent of these layers and of procedure content. A procedure's `role`
selects a role mapping, falling back to `default`; per-task overrides win.
The full profile, tier-chain, readiness, availability and upgrade contracts are
`project/instrukt/spec/feature/configuration/agent-selection`. That same role map
carries `class`, scoped `can` grants with named `bundles`, and `github_app` under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. The
machine-global App inventory and fallback identity follow
`project/instrukt/spec/feature/work/github-apps`; keys resolve only through the
protected secrets file. Neither model preferences nor project configuration
selects a bot actor.

### Locating the layers

- Installed instrukt uses `~/.instrukt/instrukt.yaml` for the global layer, including
  in ordinary project worktrees. The repository development wrapper uses its
  isolated home under `project/instrukt/spec/feature/configuration/owned-files`.
- The project layer is the `instrukt.yaml` at the root of the repository
  containing the working directory. Outside a repository, or in one without
  that file, there is no project layer.
- A company home's document is the `instrukt.yaml` in
  `~/.instrukt/companies/<name>/`, addressed with `--company <name>`.

### Company home documents

A company home's `instrukt.yaml` is a document of its own beside the two
layers (`project/instrukt/spec/feature/configuration/company-homes`). It
carries `schema_version`, `company` and the keys the layer rules permit in a
company home; any other key there is a finding naming the key. It combines with
no layer: a read with `--company <name>` answers from that document alone. It
is validated, patched, revision-checked and migrated like a layer document, and
as a machine-owned file its conversion is persisted by writing paths
(`project/instrukt/spec/feature/configuration/owned-files`).

### Effective configuration

A read without `--global` answers from the effective configuration: the
global layer with the project layer's keys applied over it, key by key. A read
with `--global` answers from the global layer alone. Writes address one layer
(`project/instrukt/spec/feature/configuration/configuration-editing`).

### Validation

`instrukt config validate` checks the project layer, or the global layer with
`--global`. Findings:

- a key in a layer that does not permit it, naming the key and its layer;
- a value of the wrong type or outside its allowed values;
- a missing `schema_version`;
- `ingress.reactions` set while `ingress.computer` is not;
- a feature switched on whose required capabilities the chosen adapter lacks,
  or switched on while `runner.runtime` names no compiled adapter, naming each
  missing capability and the selected runtime
  (`project/instrukt/spec/feature/runner/session-continuity`). A write
  that would switch such a feature on is refused with the same naming. Events
  on a computer that is the ingress and not the reacting computer need no
  runtime: receiving and publishing need no agent;
- in a project, `modules.todos` on in the effective configuration while the
  project's code is not on GitHub, saying in plain words that todos need the
  project's code on GitHub
  (`project/instrukt/spec/feature/work/github-record`). A write to the
  project layer that would switch todos on there is refused with the same
  reason.
- `runner.archive_finished_after` set while the selected runtime cannot
  archive threads, naming the runtime and the missing capability
  (`project/instrukt/spec/feature/runner/session-continuity`). A write that
  would set it so, or choose such a runtime while it is set, is refused with
  the same naming.

That capability finding withholds only thread retention: wherever the
machine layer is admitted, a layer whose only findings are this one is
admitted with the window off, the finding is reported with the others, and
everything else the layer admits runs unchanged. Every other finding at
`runner.archive_finished_after`, a value outside its shape among them, is a
finding of the machine layer like any other.

Every finding names the document and key; the exit status is non-zero when
there is at least one finding. Validation judges the document as written;
references are checked for unresolved keys and cycles without executing commands.

### A machine layer that cannot be loaded

The machine layer cannot be loaded when it cannot be read, is not YAML, is not
a mapping, or its `schema_version` is missing or newer than this instrukt
knows. Then every command fails with exit status 1 and one line naming the
file, the reason and `instrukt config validate --global`; that command fails
with the file's own problem and where it is. A layer that loads is never
judged again by the commands that read it: its validation findings, an
unknown key among them, are `instrukt config validate`'s, and stop no other
command.

### Schema versions and migration

- Every document carries `schema_version`. The global layer, the project layer
  and a company home's document each version their own shape: each kind has
  its own current schema version and its own forward conversion, and `init`
  writes its kind's current version. A change to one kind's shape raises only
  that kind's version, so documents of the other kinds stay current as written
  (`sdlc/policy/backward-compatibility-hygiene`: a shape read across a
  multi-version fleet moves once its slowest reader has caught up).
- Every configuration path converts older configuration before schema refusal
  or skip, preserving its settings. Which path persists a conversion, the
  machine's layer on writing paths and a committed project layer only in a
  commit, follows `project/instrukt/spec/feature/configuration/owned-files`.
- A document already at its kind's current version is left untouched.
- Conversion removes a machine's `modules.conversations` key, whatever its
  value, and each company member's connector identity key. A member entry left
  without an email address is the validation finding of
  `project/instrukt/spec/feature/configuration/company-homes` until the
  address is added; conversion derives none.
- The global layer's conversion removes the `retire_thread` value from each
  runtime entry of its `runtimes`, keeping every other agent-action value. A
  runtime entry that this removal leaves empty is removed, and so is a
  `runtimes` map it leaves empty.

### Environment

`INSTRUKT_NOW=<RFC 3339>` overrides the clock, for verification and
diagnosis. While it is set, every command takes it as the current time in
place of the system clock — for retention windows, schedules and due times,
and the times it records. It is read once, where the command constructs its
clock. It is never silent: while it is set, every command prints
`instrukt: clock overridden to <value>` to stderr. A value that is not RFC 3339
refuses the command; it never falls back to the system clock.

`INSTRUKT_NOW=@<path>` takes the time from a file instead. The file holds one
RFC 3339 time and is read again each time the command asks for the time, so a
verification can move the clock while a command runs. A file that is missing
or holds no RFC 3339 time when the command starts
refuses the command. One that becomes unreadable later keeps time running from the
last valid reading, advanced by elapsed monotonic time since that reading.
File-clock readings never move backward during the invocation.
When a valid file reading resumes, the clock uses the later of that value and
the last observed time. The invocation warns once when the file becomes
unreadable. Stderr failure does not interrupt a clock reading. Both fixed and
file overrides affect domain timestamps, retention windows, schedule eligibility
and due times only. All live subprocess, pipe/I/O and lock elapsed-time limits
use monotonic real time regardless of the override.

### External state declarations

External names default to occurrence. A `webhooks.<domain>` source may
set `state: [<name>, …]` and `subject_from: <dotted-payload-field>`. Names omit
the domain prefix and follow the ordinary event-name grammar. State names
require a nonempty subject field declaration. After signature verification,
the field is read from the normalized payload and must be a nonempty string
or number. Its UTF-8 percent-encoded value qualifies the source's ordinary
`inbound_source:<domain>:<kind>:<slug>` subject. Missing or invalid values
produce a client error with no relay. Other names remain occurrences. Ingress
records `inbound_delivery: state` as producer metadata independently of payload
text. Pending-state replacement is owned by procedure-event-delivery.

## Use cases

### UC-CL1: A developer overrides a machine default for one project

```gherkin
Given a developer's machine sets a default for a setting that projects may override
And one of their projects sets its own value for that setting
When an agent acting for the developer reads the setting inside that project
Then it receives the project's value
When it reads the setting inside another project that sets nothing
Then it receives the machine's default
```

### UC-CL2: A developer clones a project that tries to decide what runs on their machine

```gherkin
Given a developer clones a project whose configuration chooses the machine runtime
When the developer validates the project's configuration
Then the declaration is reported as not permitted in a project
And reading the runtime choice inside that project still answers the machine's own
```

### UC-CL3: A developer keeps working after upgrading instrukt

```gherkin
Given a developer's machine and project configuration were written by an older instrukt
When they upgrade instrukt and read or validate configuration or run a scheduled tick
Then their configuration is brought to the current schema with every setting kept
And the commands that read it work as before
And validating the project's configuration passes while noting that its committed file is still to be converted
```

### UC-CL4: An older instrukt refuses configuration written by a newer one

```gherkin
Given a developer's configuration was written by a newer instrukt than the one they run
When they read, change or validate that configuration
Then each command refuses and says how to bring instrukt up to date
And the configuration is unchanged
```

### Configuration value shapes

| Key | Type / allowed values | Default |
| --- | --- | --- |
| `ticks` | `on` or `off` | `on` (installation registers one wake-up only with enabled project demand; `off` selects operator scheduling). |
| `runner.runtime` | compiled runtime adapter name (`t3`) | absent; the one compiled adapter |
| `runner.notify` | command string or `off` | the selected runtime’s notification capability when unset |
| `runner.autostart` | boolean or map of kind to boolean | `false` (manual); in a map, an omitted kind is `false`. |
| `runner.archive_finished_after` | a positive duration instrukt can represent, in the grammar the published global schema states (`instrukt config schema --global`), or `off` | the published global schema declares it; under it no thread is archived until a person sets a window (`project/instrukt/spec/feature/runner/thread-retention`). |
| `agents.<name>.git_author` | git author identity string (name and email) | absent; an agent commit without it is refused |
| `board` | GitHub project reference `<owner>/<number>` | automatically set up and repaired with its views when todos is enabled; absence triggers setup rather than opt-out. |
| `blueprint.engine` | shipped adapter name (either layer) or command string (global layer only; in the project layer a finding, ignored like project-layer `runner.runtime`) | the shipped adapter |
| `schema_version` | schema version number written by init | current schema version; the first is `1`. |

### UC-CL5: Configure external state in its owning source

```gherkin
Given a developer configures external state names for one project source
When their declaration supplies the subject field and valid names
Then configuration retains that source declaration
But a state declaration without its subject field is refused with a finding
```

### UC-CL6: Obtain agent actions from the chosen runtime's configuration

```gherkin
Given a developer chose T3 and customized its agent-action values globally
When an agent retrieves a procedure using those instructions
Then its dispatch, child follow-up, peer message, conversation read, status inspection, listing and steering instructions come from that entry
And instrukt executes none of that prose
But a cloned project cannot replace the machine's agent-action values
```

### UC-CL7: Keep a runner operating when its clock file disappears

### UC-CL8: Obtain the thread finishing instruction from the machine's runtime entry

```gherkin
Given a developer chose T3 as this machine's runtime
When an agent retrieves a procedure that renders the thread finishing action
Then the finishing action resolves from the machine's T3 entry as native settling, for a resting conversation and for a finished todo's orchestrator alike
And a global override replaces the machine's finishing instruction while a project-layer override is refused
And a global override of an action the runtime entry does not define is refused
And reading those directives starts or modifies no runtime session
```

### UC-CL9: A project's declared regenerable output cannot reach outside its checkout

```gherkin
Given a developer's project declares the output it rebuilds from its sources
When the developer validates the project's configuration
Then the declaration is accepted
But when the declaration names a location outside the project's checkout
Then validation reports that entry
And the project is not admitted for reclaiming that output on the computer
```

## Known caveats

- An unknown key is a validation finding. There is no `publishes` key: what a
  project publishes follows from its `docs/general/` and `docs/domain/<x>/`
  positions, and a leftover `publishes` is an unknown key, never migrated.
- The project layer carries the `yaml-language-server` modeline naming its
  editor schema; it is a comment and never a setting
  (`project/instrukt/spec/feature/publication/editor-schemas`).
- Every entry path migrates older configuration before refusal or skip. All
  commands refuse a newer schema version. For a project's layer they print the
  command that installs the current instrukt,
  `curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh`.
  A machine layer of a newer schema cannot be loaded: every command names
  `instrukt config validate --global`, which prints the same installer command.
- Unresolved and cyclic references are validation findings.
- `requires` naming a switched-off module is a validation finding.
- Runtime capability metadata belongs to the shipped integration; adapter
  behavior requires code. Agent instruction prose remains configuration.
  Arbitrary executable runtime command templates are not supported.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](configuration-editing.md)
- [project/instrukt/spec/feature/configuration/interpolation](interpolation.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
