# Upgrading instrukt

## 0.1.0

### This repository's commit check

For developers of instrukt itself. The pre-commit hook runs the checks the
staged files need, as `.pre-commit-config.yaml` assigns them, through the
pre-commit framework; the functional suite runs in CI. Install the pinned
framework once, `uv tool install pre-commit==<version>` with the version in
`scripts/test-tools.env`, and keep `make hooks` as the way to enable the hook:
the framework's own `install` refuses while `core.hooksPath` is set. A
`core.hooksPath` set to the relative `.githooks`, as `make hooks` sets it,
lets each linked worktree run its own checkout's hook.

### Agent instructions are committed, and reach the runtimes you configured

A repository's `AGENTS.md` files are now committed. instrukt's `pre-commit`
section renders them from what the commit holds and stages them into it; a
sync no longer writes them and a checkout no longer renders them. Each
rendering opens with a line naming it as generated, and a file beside a
master without that line is never overwritten. A committed rendering holds
only the repository's own content: a required read of doctrine another
repository publishes becomes a line naming `instrukt docs get <id>
--closure`, and a `@config` directive in a master or in a document a
rendering holds in full is a finding. In each adopted repository, in this
order:

1. Run `instrukt hooks install` (or add the line `instrukt hooks status`
   prints to the hook manager), so the `pre-commit` section renders.
2. Prepare every source before anything is deleted: for the root master and
   every nested master, remove each `@config`, `@exec` and `@include` from the
   master and from every document its rendering holds in full — the root
   `AGENTS.md`'s `expanded` snippets and their closure, and each nested
   master's required reads and their closure. Move such a snippet from
   `expanded` to `listed`, or drop the directive. Run
   `instrukt docs render --check` until it reports no finding; it reports the
   existing files as differing, which step 4 resolves.
3. Remove the `AGENTS.md` lines `instrukt init` wrote to `.gitignore`.
4. Delete the untracked `AGENTS.md` beside each master; earlier renderings
   carry no generated line and are not replaced. Commit through the hook: the
   commit carries the new renderings.
5. Add `instrukt docs render --check` to the repository's CI, so a commit made
   without the hook (a web editor, a cloud agent) fails naming the stale file;
   repair with `instrukt docs render` and a commit. Pin the instrukt release
   CI installs to the one the hooks run.

A company home is now the checkout of the company's repository, and its
`AGENTS.md` is committed there; the runner no longer renders it before a
start. A repository renders as a company home when its root `instrukt.yaml`
declares `company`, or when its root holds `AGENTS.master.md` and no
`instrukt.yaml` at all, as a clone of the company's repository does; a root
project layer always renders as a project. For an existing company home that
is a plain folder, in this order:

1. In the home, run `instrukt config init --company <name>`: it writes the
   home's ignore lines for the settings document, the secrets file,
   `projects/` and local state. Leave all of them where they are.
2. Put the company's knowledge in place in the home: the master at its root
   and the company's documents in its `docs/` folder. The master and the
   documents reference only the company's own documents, by relative path:
   remove every required read or reference to `general`, a domain or other
   doctrine, and every `@config`, `@exec` and `@include`. Run
   `instrukt config validate --company <name>` until it reports no finding.
3. Delete an `AGENTS.md` the runner rendered in the home before this release,
   or one another tool rendered: it carries no instrukt generated line and is
   not replaced.
4. Make the home folder, in place, the working tree of the company's
   repository, so that its main is checked out there. The local-only material
   stays in the folder, untracked and ignored; nothing of it is moved or
   committed. Where the repository already holds knowledge, the home's
   documents and the repository's are reconciled into one master and one
   `docs/`.
5. Confirm that the repository tracks none of the local-only material:
   `instrukt config validate --company <name>` reports none of it, and the
   repository's status lists none of it.
6. Run `instrukt docs render` in the home and commit the master, the
   documents, the ignore file and the rendered `AGENTS.md` together; deliver
   that commit to the repository's main.
7. Have the company repository's workflow run `instrukt docs render` on main
   and commit when the rendering changed.

The operator ends with: the home at its usual location, the checkout of the
company's repository on main, holding the master, the documents, the ignore
file and the rendered `AGENTS.md`; the settings document, the secrets file,
`projects/` and local state in the same folder and out of git. The company's
agent may then change the company's documents in its home and commit them.

The global agent files go to the agent runtimes the selected runtime reports
as configured, in each one's own home, instead of to every agent program
whose user directory exists. With `runner.runtime: t3` these are T3's enabled
provider instances; T3 must be running when the `docs/general/` owner syncs,
or the files stay as they were. The next such sync writes new files for the
newly supported runtimes and removes the file of a runtime that no enabled
instance uses any more, DeepSeek's among them unless the runtime reports it;
while an enabled instance of a runtime reports no home, that runtime's
earlier files are kept and `instrukt status` shows them as kept. An
existing file in a new destination that instrukt did not write stops that
sync: move its content into documentation or the file out of the way.
`agents` keys in `docs/general/index.yaml` name the runtimes instrukt deploys
to; `instrukt docs validate` names them when a key is not one.

### One computer reacts to external facts; watched refs are repository refs

`runner.upstream_changed` and `runner.main_failing` now name their ref with a
`repository_ref` subject; an older instrukt named it with the `inbound_source`
kind, which now marks an external fact and is selected only on the computer
that reacts to external facts. An older entry of either event that is still
unread when a computer is upgraded is therefore treated as an external fact:
on a computer that is not the reacting computer, or where no ingress is named,
it is settled without activating anything, and since the ref's observation is
already recorded, the same revision is not reported again; only the ref's next
change wakes the agent. On the reacting computer it can activate for a project
another computer emitted it for, and fails there when that project is absent.
An activation already accepted before the upgrade is delivered as before.

Upgrade every participating computer together, in these steps:

1. **End every older producer, a tick already running included.** On every
   computer stop what starts ticks and the runner, and keep it stopped until
   all computers are upgraded: on macOS
   `launchctl unload ~/Library/LaunchAgents/ai.instrukt.tick.plist` (and
   `ai.instrukt.runner.plist` where it exists), on Linux
   `systemctl --user stop instrukt-tick.timer instrukt-tick.service instrukt-runner.service`,
   or the `instrukt tick` line in the crontab where the wake-up is cron; also
   stop any `instrukt run` started by hand. Then run `instrukt tick` by hand:
   while a tick or run started earlier still holds the runner lock, it prints
   `the runner is busy; this wake-up was skipped` and does nothing. Repeat
   until it no longer prints that line; from then on, no earlier tick or run
   is still working, and ticks run only by hand.
2. **Let the consumers catch up.** The older build has no command that shows
   entries it has recorded but not yet published. Run `instrukt tick` by hand
   on every computer until `instrukt events status` shows no pending entries
   and no lag for every procedure-activation consumer. This reduces what is
   left; it does not prove that nothing is. An older entry still unpublished
   or unread at the upgrade is treated as described above, and the ref's next
   change raises a new one.
3. **Upgrade every computer** without running another tick in between, then
   run `instrukt install` on each; it registers the stopped wake-up and runner
   service again.

On Linux every owned unit that has to run without a login (the runner service,
the receiver's socket and service, the private local Redis service and the
minute timer) needs the user's service manager to start at boot.
`instrukt install --check` now reports one finding naming
`loginctl enable-linger` when it does not; instrukt never changes it. Run
`loginctl enable-linger` once for the user that runs instrukt, then
`instrukt install --check` again.

### T3 release contract and failure routing

Instrukt's own project enables events with an empty watch list and selects
both `renovate/` and `upstream/` dependency-branch prefixes. On a computer
running its failure responses, converge the existing event prerequisites with
`instrukt install --module events` and verify them with
`instrukt install --check --module events`. Publish the integrator subscription
and run the normal participant; CI detection alone cannot file assessed bug work.
The observation App retains its existing check/status/content read permissions.

Install the dispatch-enabled T3 contract workflow on the repository's default
branch before enabling hourly discovery. Its scheduler needs repository-token
contents and Actions write permissions to push `upstream/t3-nightly` and issue
`workflow_dispatch`; token-generated pushes do not trigger the check themselves.
A pin commit is not a successful check. Redispatch that branch's contract workflow
if dispatch was interrupted. Only current dependency heads are observed, so a
failure replaced before observation may be missed.

### Committed files change only in a commit

instrukt no longer rewrites a repository's committed `instrukt.yaml`, or the
editor modelines of its committed files, from sync, hooks after a commit or
checkout, installation, ticks or board reflection. It converts an older project
layer in memory, and its new `pre-commit` hook section converts the layer and
corrects the modelines of the files being committed, staging both into that
commit. A board instrukt set up or found is recorded on the computer; the
state database migrates to hold it. In each adopted repository run
`instrukt hooks install` (or add the line `instrukt hooks status` prints to the
hook manager) so the `pre-commit` section exists.

A commit made without the hook, from a web editor or a bot, keeps an older
project layer. `instrukt config validate` reports it, exiting non-zero; to catch
it, add that command to the repository's own CI, run inside the checkout with a
throwaway machine layer that makes the project's configuration valid, written
from outside the checkout (`scripts/check-committed-config.sh` is instrukt's
own). A repository whose project layer still names an older schema needs one
commit through the hook, or `instrukt config init`, before that check passes.

### Dependency-update CI observation

Configuration schema 5 adds the project-only `ci.dependency_branch_prefixes`
list. Existing configurations migrate without changing their settings; an absent
selection follows the configuration owner's default. Run the project's existing
runner with events enabled and publish the updated integrator subscription so
failed dependency check runs reach its ordinary activation path. The integrator
assesses bug evidence and deduplicates work; this installs no webhook bridge.

### Blueprint tool and checkout instructions

For projects with blueprint enabled, make `uv` available on the computer, then
run `instrukt install --module blueprint` followed by `instrukt install --check
--module blueprint`. The shipped adapter now uses the release-pinned Graphify
inside the selected instrukt home. A host `graphify` on PATH no longer supplies
the shipped adapter; existing configured `blueprint.engine` commands retain
their command behavior.

Run `instrukt hooks install` in each adopted repository so its sections are
current. A checkout renders no agent instructions: every checkout, linked
worktrees included, holds the `AGENTS.md` files committed at its revision
(see “Agent instructions are committed, and reach the runtimes you
configured” above).

### GitHub App identities

Existing installations using `GITHUB_TOKEN`, `GH_TOKEN` or `gh auth token`
need App setup before remote work reflection and CI observation:

1. Reuse existing TeleClaude project/author/reviewer Apps when available, or
   create three distinct Apps and install them for each intended repository.
   Keep the observed App ids, slugs and stable bot id/node-id/login tuples.
2. Grant the project's record App issues write and contents/pull-requests read;
   grant the author App contents/pull-requests write, and the reviewer App
   contents read/pull-requests write. Board maintenance also needs organization
   Projects write on the repository's installation; CI observation needs checks,
   statuses and contents read. Resolve missing installation/permission support
   before treating remote setup as complete.
3. On each computer, store keys with `instrukt secrets set NAME --from-file
   /protected/input-file` or redirected stdin. Keys live only in owner-only
   `~/.instrukt/secrets.env`; model-provider keys remain with T3.
4. Configure machine-global `github.apps` public profiles with their actual
   `app_id`, `slug`, `bot: {id, node_id, login}` and `private_key: ${NAME}`
   references; select `github.project_app`. Set `roles.<role>.github_app` in
   `agents.yaml`, using different author/reviewer identities. Omitted mappings
   use the project App. Preserve existing capabilities and model choices.
5. Validate global/project configuration and verify observed actor facts during
   setup. Confirm an authorized remote reflection and CI observation authenticate
   as the configured Apps with usable scoped credentials. Missing verification
   stays incomplete; local accepted work remains pending and retryable.
6. Retire the former token environment/gh credential integration for instrukt.
   It supplies no fallback. Raw runtime-owned git/gh use keeps its own credential
   integration. Local bot-attributed commits consume configured verified actor
   facts and require no installation access to the current repository.

### Conversations without a switch, people known from Slack

A company that declares a connector has conversations. The machine's
`modules.conversations` key no longer exists and is removed from older
settings, whatever its value: a company whose connector was declared while that
switch was off is served once the runner runs.

Nobody is listed for the agent any more. A full member of the company's Slack
workspace is answered; guests, people from other organizations, bots and
deactivated accounts are not. A company's `members` now hold only a person's
own tool logins and are found by email address. Before upgrading, give each
member entry that carries logins the person's `email`, and remove entries that
only named a Slack identity. An entry without an address is a validation
finding and its company is not served until the address is added.

An existing Slack app needs the permissions of the connector's manifest
(`docs/project/spec/feature/company/slack-connector.md`), reading users and
their email addresses among them. Add the missing ones in the app's settings
and install the app to the workspace again, before upgrading; Slack grants new
permissions only on a new installation. Store the bot token again when Slack
issues a new one.

Name the company's Slack app and its bot `agent`; the status line Slack shows
while the agent works starts with that name.

### Signing in to a company's MCP servers

A company home's MCP server can be declared with `speaker_login: sign-in`
(`docs/project/spec/feature/runner/mcp-access.md`). A declaration that names
`credential` keeps its stored logins and needs nothing.

Before declaring a server with sign-in:

- Set `ingress.url` in the global layer of every participating computer to
  the public `https` address at which the operator's edge forwards
  `/hooks/inbound` to the receiver, keep events enabled on the ingress
  computer, and keep cluster Redis reachable from the ingress computer and
  the company's computer. Then run `instrukt install` on both.
- For a server registered with a client beforehand (`sign_in.client`),
  register the return address `instrukt mcp list --company <company>` shows
  with the server's authorization server.

The Slack app needs no new permission: the sign-in link is posted to the
person's direct messages with the bot's existing `chat:write`.

### A company's agent starts from its procedure

A conversation's session now starts from the published procedure that
declares the role `company-agent`. instrukt ships no doctrine: the docs
repository the computer adopted publishes that procedure (instrukt's own
publishes it as `general/procedure/company-assistance`).

On a computer that serves conversations, in this order:

1. Bring the adopted docs repository to a commit that holds a delivered
   procedure declaring `company-agent`, and let its sync publish it
   (`instrukt sync` in that repository, or its hook when main moves).
2. Replace the program, then run `instrukt install` so the runner restarts on
   it.

Until a delivered procedure declares the role, a conversation's first message
waits: no session starts, nothing is replied, and `instrukt events status`
names the missing role. Once one is published the message starts without
being sent again. Sessions started earlier keep the opening they started with.

The agents file converts by itself: its `roles.conversation` entry becomes
`roles.company-agent` with its settings kept. A file holding both entries is
refused, and left as it is, until one of them is removed.

### Protected secrets and owned files

Existing development installations need machine-local secret setup before using
webhook intake or authenticated Redis with this build:

1. Store each referenced value with `instrukt secrets set NAME --from-file
   /protected/input-file`, or provide it through redirected standard input.
   `instrukt secrets generate NAME` creates a new random value when rotation is
   intended. These commands never print values; terminal input is refused to
   prevent terminal echo.
2. Keep `${NAME}` references in webhook and Redis configuration. Distinct values
   for different projects require distinct names. Project `.env` files and process
   environment variables no longer supply signing or Redis credentials. Coordinate
   signing-key rotation with the sender before replacing a current signing value.
3. Run `instrukt config validate --global` and validate each project on its ingress
   computer. Required material must be readable only by its owner in the normal
   machine home's `secrets.env`.

Configuration schemas 1 and 2 migrate to schema 3 before reads, validation, patches,
publication decisions, installation and ticks. Supported global model choices
move to `agents.yaml`; non-bot git authors remain in `instrukt.yaml`. If a
different `agents.yaml` already exists, conversion refuses rather than discarding
either choice. Reconcile those selections and retry. A failed conversion keeps
the source configuration.

Schema 3 removes the retired project-layer `actions` key while preserving supported
settings and the editor modeline. Procedures take project checks from project
documentation. A new `actions` declaration is an unknown-key finding.

Project documentation now lives in `~/.instrukt/docs/project/<slug>/`, copied from
committed canonical main. Index schema 1 is upcast to schema 2 on read; its former
checkout paths become store paths. For each previously published project, run
`instrukt sync --force` once after upgrading to populate its store even when main
has not changed. A missing store file reports the project and republish command;
retrieval never reads a checkout as a fallback. Source headers retain the real
checkout address for authoring.

Procedure frontmatter no longer accepts `dispatch`. Configure its role's order
and tier in `agents.yaml`. Model names are open; use `instrukt config show agents`
to inspect effective mappings and `instrukt agents availability` for local
capacity restrictions. T3 retains model-provider credentials.

Development of instrukt itself uses `scripts/dev-instrukt <arguments>` or
`make dev ARGS='<arguments>'`. The wrapper builds this checkout, copies its binary
under the shared build lock, then runs it from `.instrukt-home/bin/` with a private
`.instrukt-home`. First use initializes default configuration and publishes this
worktree's committed HEAD there. Subsequent configuration edits are retained;
`sync` through the wrapper refreshes from the development worktree's HEAD.
Generated user-scope files and logs remain within the development home. The
normal protected secrets file is shared; secrets commands still deliberately
write that one machine file.

Installed instrukt in ordinary project worktrees uses the shared machine home.
Entering a linked project worktree does not redirect claims or todos. This
repository has no worktree-removal hook: remove its ignored `.instrukt-home`
when cleaning up a development worktree (before removing that worktree). No
machine home needs removal or reseeding.

Unattended starts require the adapter's supported WebSocket protocol and
readiness response shapes. CLI/server release strings are diagnostic; a different
compatible release remains usable. Unknown protocols or response shapes refuse startup
with a repair reason. The adapter obtains and revokes a five-minute read-scoped
session for each new start's provider snapshot. Quota restrictions now come from
reported usage windows, with a recorded one-hour expiry when no reset is reported;
launch-error text supplies no quota or no-start evidence.

Agents schema 1 converts to schema 2 before selection/default replacement.
Every harness uses the same neutral `effort` input; only the adapter emits native
option identifiers. Existing effort settings are preserved. Conflicting retired
fields refuse conversion with the source intact until reconciled. Runtime start
requests migrate to the same neutral representation before current consumers load.
