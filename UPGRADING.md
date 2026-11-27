# Upgrading instrukt

## 0.1.0

### Ephemeral project working directories move under `.instrukt/`

All instrukt-owned ephemeral project working directories now live under the
single gitignored `.instrukt/` namespace instead of top-level repository
folders:

- task workspaces: `.instrukt/workspace/<slug>/`
- project scratchpads and default `instrukt eval report` output:
  `.instrukt/scratchpad/` (`.instrukt/scratchpad/eval/index.html`)
- persistent integration worktree: `.instrukt/trees/_integration/`

Adopting repositories require only `.instrukt/` in `.gitignore`, which
`instrukt init` already writes. Any in-flight `withdrawal_intents` row
recorded before upgrading finishes against its stored `workspace_path` without
manual intervention.

For an existing checkout that still holds untracked instrukt-created root
directories, move them in this order (run `git worktree list` to check
whether an integration worktree exists):

1. Verify each candidate root directory is not tracked by git before moving it:
   `git ls-files <dir>` must print nothing. Leave any tracked project directory
   untouched.
2. Relocate an existing integration worktree:
   `mkdir -p .instrukt/trees && git worktree move trees/_integration .instrukt/trees/_integration && rmdir trees`.
   If left in place, the standing integrator also relocates an existing
   `integrate` worktree with `git worktree move` on its next wake.
3. Relocate existing task workspaces so recurring tasks retain checkpoints and
   ranked lists:
   `mkdir -p .instrukt/workspace && mv workspace/* .instrukt/workspace/ && rmdir workspace`.
4. Relocate any active shared scratchpads:
   `mkdir -p .instrukt/scratchpad && mv scratchpad/* .instrukt/scratchpad/ && rmdir scratchpad`
   (or remove disposable `scratchpad/eval/index.html` and `rmdir` the empty
   directories).
5. Once every checkout on the machine has moved its root directories, remove
   any manual `/workspace/`, `/scratchpad/`, or `/trees/` lines added to
   `.gitignore`, keeping `.instrukt/`.

### Retired discussion links, control wrappers, bare NOOP suppression, and proxy review recording

The following legacy surfaces have been retired:

- **`instrukt discuss` and `runtime.discuss`:** Peer thread conversation links
  and the `instrukt discuss` subcommand have been removed. Company-connected
  conversation relay (Slack connectors) continues unchanged. Existing
  `instrukt.yaml` machine overrides carrying `runtimes.t3.discuss` and existing
  `agents.yaml` files carrying the `conversation-link` action continue to parse
  and admit without failing startup; remove those retired keys when you next
  edit those files. Existing `state/instrukt.db` databases open without manual
  migration (historical schema migrations are preserved; any prior
  `conversation_links` rows remain dormant and unread).
- **Global `[Instrukt Assignment]` and `[Instrukt Notification]` wrappers:**
  Runner deliveries and agent launch briefs now arrive as plain structured text
  without `[Instrukt Assignment]` / `[Instrukt Notification]` wrappers. Only
  company connected-conversation messages retain `[Member (... · boundary)]`
  and `[Instrukt Conversation Context (boundary)]` framing. In-flight
  conversation rollovers, unaccepted requested-review retries, and open manual
  standing reservations stored before upgrade continue to match their legacy
  wrapped payloads until they complete.
- **Bare `NOOP` suppression:** Connected company conversations no longer drop a
  turn whose output is literally `NOOP`; it is posted as ordinary turn output.
  Empty/whitespace turns remain unposted.
- **`instrukt review record --reviewer`:** Proxy verdict entry on behalf of
  another session has been removed so every standing verdict is recorded
  directly by the independent reviewing session that produced it. Historical
  verdicts that recorded an `entered_by` session remain readable and exportable.

**Transition & rollback:** Upgrading requires no database reset or config edit
for existing `runtimes.t3.discuss` / `conversation-link` keys, and in-flight
standing/review/conversation records finish cleanly. Scripts or agent
instructions invoking `instrukt discuss`, `instrukt config get runtime.discuss`,
or `instrukt review record --reviewer` must be updated to native runtime
delegation/messaging and direct reviewer-session `instrukt review record`
calls. To roll back, let any open new-format manual standing reservations
complete or clear them before restoring the previous `instrukt` binary:
although this release performs no destructive SQLite schema migration, manual
standing reservations opened on this release store plain (unwrapped) briefs
that older binaries cannot resume because older builds require exact matches
against the legacy `[Instrukt Assignment]` wrapper.

### A company repository declares its company identity

A company repository's committed project layer now declares which company it
is: the key its `project` carries after `company-`, and the name people write
the company by in the new project-only key `company_name`. `instrukt init
--company <key> --name <name>` writes it into a new company's skeleton. An
existing company repository has no `company_name` yet: add it with `instrukt
config patch` on its project layer, through a reviewed change like any other
change to the repository. Until then `instrukt config validate` reports the
missing identity against `modules.events`, the home's event responses are
withheld, other patches of the layer are refused, and its plugin does not
render. No migration can supply the name, since it cannot be derived.

`instrukt docs plugin` no longer takes `--key` or `--name`: it reads the
company identity from the repository's committed project layer, so a home, a
clone and an integration candidate render the same bytes. A script that still
passes them gets a usage error; drop both flags. A repository without a
company identity is refused with that finding and nothing is written.
`instrukt docs render --plugin` renders the repository's whole agent material
as one operation, the entrypoints and in a company repository the company's
plugin, and `instrukt docs render --plugin --check` checks it.

### A company's settings and skill move

A company home keeps its settings in `company.yaml`, beside the project
layer its repository commits as `instrukt.yaml`. A home whose settings are
still its root `instrukt.yaml` declaring `company` needs nothing done: it is
read as it stands, and the next writing command (`instrukt config patch
--company`, `instrukt config init --company`) adds the `/company.yaml` ignore
line and renames the file to `company.yaml` with every setting it holds. A
home that holds both files is reported naming both and left as it is; keep the
one that holds the settings and remove the other.

`instrukt docs plugin` now renders the company's skill at `skills/<key>/` and
makes its entry in `.claude-plugin/marketplace.json` the plugin's manifest,
creating the catalog where the repository holds none. It no longer renders a
`plugins/<key>/` folder or a `plugin.json`, and it never writes or removes
one. A repository it rendered before keeps its release series: the first
rendering reads the version of `plugins/<key>/.claude-plugin/plugin.json` and
carries it, raised by one, into the catalog entry. Once `skills/<key>/` is
rendered and committed, remove `plugins/<key>/` from the company's repository
unless another publisher still renders it.

New companies are added with `instrukt init --company <key>`, after declaring
the owning GitHub organisation once with `instrukt config patch --global` and
`github.org`.

### This repository's commit check

For developers of instrukt itself. The pre-commit hook runs the checks the
staged files need, as `.pre-commit-config.yaml` assigns them, through the
pre-commit framework; the functional suite runs in CI. Install the pinned
framework once, `uv tool install pre-commit==<version>` with the version in
`scripts/test-tools.env`, and keep `make hooks` as the way to enable the hook:
the framework's own `install` refuses while `core.hooksPath` is set. A
`core.hooksPath` set to the relative `.githooks`, as `make hooks` sets it,
lets each linked worktree run its own checkout's hook. The Python lint and
type check are the framework's own hooks for ruff and mypy at pinned commits.
`make hooks` installs them into `~/.cache/pre-commit`, which needs network
access. A commit made before that, or after a pinned tool changes, installs
what it needs itself. A newly installed tool's first run is slower too, once
per installation and not once per checkout, so a commit takes longer that one
time. A clone that enabled the hook before this change needs nothing done. A
`v*` tag builds and publishes release binaries only after the checks `main`
requires pass on the tagged revision.

The hook directory also holds instrukt's own sections
(`project/instrukt/spec/feature/repository/hook-sections`): in `pre-commit`
before the framework call, and as `post-commit`, `post-merge`, `post-rewrite`
and `post-checkout`, so that documents are published from main without a
command. They run the installed `instrukt` from the path; without it a commit
goes through unchanged. An existing checkout has nothing to install for them
beyond `instrukt` itself. Publication they start needs this computer's global
agent files to be instrukt's; until then `instrukt hooks status` reports the
refused sync, and git's result is unchanged.

### Agent instructions are committed, and reach the runtimes you configured

A repository's `AGENTS.md` files are now committed. instrukt's `pre-commit`
section renders them from what the commit holds and stages them into it; a
sync no longer writes them and a checkout no longer renders them. Each
rendering opens with a line naming it as generated, and is written over
whatever the `AGENTS.md` beside its master held. A committed rendering holds
only the repository's own content: a required read of doctrine another
repository publishes becomes a line naming `instrukt docs get <id>
--closure`, and a `@config` directive in a master or in a document a
rendering holds in full is a finding. In each adopted repository, in this
order:

1. Run `instrukt hooks install` (or add the line `instrukt hooks status`
   prints to the hook manager), so the `pre-commit` section renders.
2. Clear the Codex override. instrukt no longer writes Codex's
   `AGENTS.override.md`, which Codex reads in place of `AGENTS.md`, so a copy
   left behind would hide every committed update; instrukt deletes none, and
   the hook and `instrukt docs render --check` report one beside a root
   master as a finding. Delete each copy an earlier release rendered, beside
   the primary checkout's root master and in each linked worktree, and the
   `instrukt-entrypoints.json` checkout rendering kept in each worktree's git
   directory (`git rev-parse --git-dir`). If an `AGENTS.override.md` holds
   instructions you wrote yourself, move them into the project's
   documentation first (a project snippet with its entry in
   `docs/project/index.yaml`), then delete the file.
3. Prepare every source: for the root master and
   every nested master, remove each `@config`, `@exec`, `@include` and each
   reference left for an agent program to resolve from the master and from
   every document its rendering holds in full — the root `AGENTS.md`'s
   `expanded` snippets and their closure, and each nested master's required
   reads and their closure. Move such a snippet from `expanded` to `listed`,
   or drop the directive. Run `instrukt docs render --check` until it reports
   no finding; it reports the existing files as differing, which step 5
   resolves.
4. Remove the `AGENTS.md` lines `instrukt init` wrote to `.gitignore`.
5. Commit through the hook: the commit carries the new renderings, written
   over the untracked `AGENTS.md` beside each master.
6. Add `instrukt docs render --check` to the repository's CI, so a commit made
   without the hook (a web editor, a cloud agent) fails naming the stale file;
   repair with `instrukt docs render` and a commit. Pin the instrukt release
   CI installs to the one the hooks run.

A company home is now the checkout of the company's repository, and its
`AGENTS.md` is committed there; the runner no longer renders it before a
start. A repository renders as a company home when its root `instrukt.yaml`
declares `company`, or when its root holds `AGENTS.master.md` and no
`instrukt.yaml` at all, as a clone of the company's repository does, or when
its root project layer declares `company_repository: true`, as a company's
repository generated by `instrukt init --company` does; any other root
project layer renders as a project. For an existing company home that
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
3. Make the home folder, in place, the working tree of the company's
   repository, so that its main is checked out there. The local-only material
   stays in the folder, untracked and ignored; nothing of it is moved or
   committed. Where the repository already holds knowledge, the home's
   documents and the repository's are reconciled into one master and one
   `docs/`.
4. Confirm that the repository tracks none of the local-only material:
   `instrukt config validate --company <name>` reports none of it, and the
   repository's status lists none of it.
5. Run `instrukt docs render` in the home, which writes the rendering over
   any `AGENTS.md` the home held, and commit the master, the documents, the
   ignore file and the rendered `AGENTS.md` together; deliver that commit to
   the repository's main.
6. Have the company repository's workflow run `instrukt docs render` on main
   and commit when the rendering changed.

The operator ends with: the home at its usual location, the checkout of the
company's repository on main, holding the master, the documents, the ignore
file and the rendered `AGENTS.md`; the settings document, the secrets file,
`projects/` and local state in the same folder and out of git. The company's
agent may then change the company's documents in its home and commit them.

The global agent files go to the agent runtimes the selected runtime reports
as configured, in each one's own home, instead of to every agent program
whose user directory exists. With `runner.runtime: t3`, or with it unset,
which selects T3, these are T3's enabled provider instances; T3 must be
running when the `docs/general/` owner syncs, or that sync writes no global
file and says so. Antigravity's file moves from
`~/.gemini/antigravity-cli/AGENTS.md` to `~/.gemini/AGENTS.md`, which
Antigravity reads without a settings key; the old file and the key instrukt
set in `~/.gemini/antigravity-cli/settings.json` stay until you delete them. Each file is written whole by every sync
that publishes it, over whatever the destination holds, a file you or
another program wrote included: move instructions you want to keep into
documentation before the first sync of this release that publishes. instrukt
removes no global file: the file of a runtime no enabled instance uses any more,
DeepSeek's among them unless the runtime reports it, stays until you delete
it. The record of written files an earlier release kept under
`~/.instrukt/state/publication/outputs/` is deleted by the first sync that
publishes.

Every sync now converges: it writes what is missing or different whether or
not main has moved, so a file deleted or edited by hand comes back at the next
sync. `instrukt sync` no longer takes `--force`; remove it from scripts and
aliases. A symbolic link at one of instrukt's destinations, such as a global
agent file linked into a dotfiles repository, is replaced by the file and the
sync names it; the file the link pointed to is left as it was. Keep what that
file held in documentation.
`agents` keys in `docs/general/index.yaml` name the runtimes instrukt deploys
to; `instrukt docs validate` names them when a key is not one. Two keys are
retired. In a `docs/general/index.yaml` you publish, rename `agents: gemini`
to `antigravity` (its file was Antigravity's, so the same sessions read it)
and remove `agents: deepseek` or move its snippets to a runtime that remains;
then validate and sync. Until then `instrukt docs validate` and the sync
report `index-agent-unsupported` for each of the two keys, naming the runtimes
instrukt deploys to, and the sync publishes nothing.

Hook-run outcomes recorded before this release are not carried over: the
first hook run removes `~/.instrukt/state/hooks/`, and `instrukt hooks status`
shows each hook's outcome again after its next run, now for the repository
and all its worktrees.

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
project layer. `instrukt config validate` judges it as converted in memory and
notes its pending conversion on stderr without failing; the next commit through
the hook, or `instrukt config init`, converts the file. To catch a project
layer that is invalid, names a newer schema or names none, add that command to
the repository's own CI, run inside the checkout with a throwaway machine layer
that makes the project's configuration valid, written from outside the checkout
(`scripts/check-committed-config.sh` is instrukt's own).

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

1. Reuse existing project, author and reviewer Apps when available, or
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

### Automatic review of pull requests and integration candidates

Nothing changes until a project opts in: `review.automatic` is absent, so its
pull requests are not observed, no review is asked for and nothing is
published. To switch it on for a project, in this order:

1. Install this release on the computer that serves the project.
2. On the repository's App installations, grant the reviewer App (the one the
   `commit-reviewer` role selects) write access to pull requests and to commit
   statuses, and the project App read access to pull requests. No other
   credential changes.
3. Publish the documentation that holds the delivered
   `sdlc/procedure/lifecycle/work/review-commits`, which subscribes to
   `runner.review_needed` under the `commit-reviewer` role and is the only
   procedure subscribed to it. Admission requires that route.
4. Record the approval of every head reviewed before the cutover that is not
   on the canonical branch yet from its reviewing session with `instrukt review
   record`. A head whose approval is not recorded is reviewed as part of its
   candidate.
5. Set `review: {automatic: true}` in the project layer through a reviewed
   commit. `instrukt config patch` refuses it while a finding stands, and
   `instrukt install --check` reports any readiness finding of the two
   identities.
6. Observe that an integration candidate carries the reviewer bot's
   `instrukt/review` status.
7. Only then require `instrukt/review` on the canonical branch, with the
   reviewer App as its expected source. Requiring it earlier blocks the
   integrator's fast-forward until candidates carry the status.

Review evidence is kept on the serving computer in `state/review.db`. When
another computer starts serving the project, move it with `instrukt review
export --to <file>` there and `instrukt review import <file>` here; a computer
that starts without an import recognises what the reviewer bot already
published and asks for every other owed head.

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

### The project coordinator stands and is woken by fleet attention

The project's coordinator is now `sdlc-coordinator`, a standing role woken by
`runner.attention.*` events, and agents can raise a condition about their
project's active todos with `instrukt events raise`.

The agents file converts by itself: its `roles.coordinator` entry becomes
`roles.sdlc-coordinator` with its settings kept. A file holding both entries
is refused, and left as it is, until one of them is removed.

The shipped `technical-coordinator` role now selects at `tier: strong`. A kept
agents file that sets its own tier for that role keeps it.

The shipped `orchestrate` bundle grants `attention-raise` at `own`. A kept
agents file that defines its own `orchestrate` bundle replaces the shipped one
whole; add `{action: attention-raise, scope: own}` to it so orchestrating
roles can raise.

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
`instrukt sync` once after upgrading to populate its store, whether or not main
has changed. A missing store file reports the project and republish command;
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

The provider snapshot unattended starts select from requires the adapter's
supported WebSocket protocol and readiness response shapes. CLI/server release
strings are diagnostic; a different compatible release remains usable. The runner
obtains one read-scoped session when it starts, renews it before it expires, and
refreshes the snapshot in the background; a start uses the last known snapshot.
An unknown protocol or response shape, or a T3 CLI that is slow or fails, makes
a refresh fail, shown in `instrukt events status`; it never refuses a start. Quota restrictions now come from
reported usage windows, with a recorded one-hour expiry when no reset is reported;
launch-error text supplies no quota or no-start evidence.

Agents schema 1 converts to schema 2 before selection/default replacement.
Every harness uses the same neutral `effort` input; only the adapter emits native
option identifiers. Existing effort settings are preserved. Conflicting retired
fields refuse conversion with the source intact until reconciled. Runtime start
requests migrate to the same neutral representation before current consumers load.

### Prerequisites are checked where they enter; outside failures are retried

In this order:

1. Update each company's Slack app from the manifest in
   `docs/project/spec/feature/company/slack-connector.md`: it now also requires
   `channels:read`, `groups:read` and `im:read`, which reading back missed messages
   needs. Reinstall the app to its workspace so its bot token carries them; a
   connector missing a required scope is withheld with a finding naming it.
2. Make sure every secret a company's connector references has a value in that
   home's `secrets.env` (`instrukt secrets set NAME --company <name>`, which now
   asks at a terminal with echo off). A missing or blank one now withholds that
   connector.
3. Make sure the procedure declaring the role `company-agent` is published on each
   computer that serves a company; without it the company's connector is withheld
   with a finding.
4. Install this release and run `instrukt install` once. It writes every service
   definition under instrukt's names and restarts the services, and deletes
   `~/.instrukt/state/installation/owned.json` and the runner service's identity
   record. A second instrukt home on the computer (an `INSTRUKT_HOME` other than
   `~/.instrukt`) now has units with its own suffix: run `instrukt install` from
   that home too, which withdraws its earlier units that carried no suffix.

Then:

- Switching on event responses now needs `redis.cluster.url` in the machine
  layer: `config patch` refuses it without, and a project whose committed layer
  switches them on is not admitted for event responses on a computer without it
  (its todos and history run as before). Set the URL and run `instrukt install`.
- `agents.yaml` now holds only your overrides; the defaults are in the program.
  An unedited copy an earlier release wrote is removed by the first writing
  command. An edited one is kept and overrides everything it names: delete the
  entries you never changed to receive new defaults.
- `runner.runtime` may be left unset: it is then the one runtime compiled in.
- A home or `state/` folder reached through a symbolic link works. A link at
  `state/instrukt.db` itself is refused; link the home or `state/` instead.
- `instrukt sessions resolve-start` stays the operator's way to settle a start
  whose confirmation was lost; such starts, key conflicts and archived threads now
  also appear in `instrukt events status`.
- Messages written to a company's agent while it was offline are read back and
  answered when they are less than a day old; older ones are listed in
  `instrukt events status` for the operator.
- A later release stops converting configuration and agents files written before
  their current schemas. Run a writing command of this release once on every
  computer, and commit each adopted project once through its hooks, so every file
  is stored at the current schema before that release.

Defects of the system an agent works through are reported through one front
door, `instrukt bugs report '<description>'`: without `--approved` it files
nothing and prints the questions to face; with `--approved` it files a bug in
the owning project, named with `--project <slug>`. `instrukt todo create
--kind bug` is refused and names that door. The capability catalog gains the
action `bug-report`: the `record` bundle carries it at `own`, the new `report`
bundle at `any`, and `company-agent` holds `read`, `tools` and `report`. An
`~/.instrukt/agents.yaml` that an earlier release wrote and its deployment
record recognises as unedited is removed by the first writing command, and
every role then takes these defaults. Any other file is kept as your
overrides: every role, bundle and field it leaves out takes these defaults,
and what it names stays as written. A `company-agent` whose `can` the file
lists holds the union of the scoped grants that list expands to, across its
explicit grants and the bundles it names. As a standing role it still sees
`instrukt bugs report` in command help; its usual-capabilities summary and
conversation briefing offer cross-project defect reporting (`bug-report:any`)
only when those grants supply it, for example by naming the shipped `report`
bundle where the file does not redefine it. Focused roles using a `record`
bundle the file defines report into their own project once you add
`{action: bug-report, scope: own}` to it. A role naming `report` in a file
that defines no such bundle receives the shipped one.

### One root AGENTS.md, rendered from an index.yaml

No entrypoint is rendered from an `AGENTS.master.md` any more. A repository
carries one rendered instruction file, the `AGENTS.md` at its root, rendered
from its entry index: `docs/project/index.yaml` in a project,
`docs/index.yaml` in a company home. Nothing below the root is rendered. This
replaces every step above that puts content in a master. A former master is
an `AGENTS.master.md` kept at a repository's root or in a subfolder as the
source of the `AGENTS.md` beside it; a document under `docs/` that the entry
index selects is not one, whatever its name, and is left where it is. In this
order:

1. Before the first commit through the new hook, in each project, move the
   root master's purpose into a project snippet (a `concept` is the usual
   type) and name it first in `expanded` in `docs/project/index.yaml`; move
   each nested master's content into project snippets the index selects. What
   stays in a master reaches no agent once the new release renders, and
   instrukt never deletes a master, so nothing is lost if this step comes
   later — but the commit made before it carries a root `AGENTS.md` without
   that content.
2. Install the release. The next commit through the hook carries the one root
   `AGENTS.md`, opening with the new generated line, and removes every
   `AGENTS.md` an earlier release rendered in a subfolder; an `AGENTS.md`
   there that you wrote stays. A project without `docs/project/index.yaml`
   has no rendering, and an earlier rendering at its root is removed. Once
   nothing reads a former master as a source, delete it.
3. In each company repository, create `docs/index.yaml` with `expanded` and
   `listed`, each a list of document paths relative to `docs/` ending in
   `.md`, and move the master's own text into a document it selects. A
   `listed` document needs a `description` in its frontmatter: the rendering
   names it by one line with that description. Run
   `instrukt config validate --company <name>` until it reports no finding.
   The company's workflow publishes its `AGENTS.md` and skill with the shared
   workflows repository's publisher (`InstruktAI/workflows`,
   `scripts/company-skill/render.py`), which still reads `AGENTS.master.md`
   at its released version: in the same change, take a publisher release
   that reads `docs/index.yaml` (`expanded` in full with their required
   reads, `listed` as one line with the description) and delete the master.
   Until that release exists, keep the master for the publisher; it means
   nothing to instrukt.
4. The editor schema for a company index is `company-index.schema.json` in
   any project's `.instrukt/schemas/`. Nothing writes schemas or a modeline in
   a company home; point an editor there at it by hand. The domain index
   schema now requires `description`, as validation already did.

### History keeps only what was said

History now retains, of each conversation, its identity, title, parent,
archive aliases and the runtime's record of it, and its messages with their
roles, times and text, each message text stored once
(`project/instrukt/spec/feature/runner/history`). It no longer retains what a
conversation's tools ran and returned, the files it changed, its reasoning
traces, its runs or its subagents; that text is no longer found by
`instrukt history search`, and `instrukt history show` of a runtime
conversation prints its `thread` and `messages` without the `runs`,
`subagents` and `turnItems` collections. Conversations an earlier import
retained keep their document and are found by the same text as before.

The first history operation after upgrading, on each computer, converts
`state/history.db` in place and then rewrites it to free the space the
dropped records held, holding the history lock throughout; on a large store
each step takes a while. While the conversion runs, every history command
fails once its wait for the lock times out. Once the conversion has
committed, and provided this computer had already finished moving its earlier
history out of `state/instrukt.db`, `history show` and the search's own read
work again while the rewrite continues; `history extract` still times out,
and `history search` returns what is retained and reports its catch-up as
failed. If that earlier move had not finished, every history command,
`history show` and `history search`
included, keeps waiting for the lock and can time out until the rewrite and
the move complete. The runner retries its extraction throughout. What the
store held of tool records, runs and subagents is dropped by the conversion
and nothing restores it. To keep a copy, take one before installing the new
build, while the store is still in its earlier shape, with SQLite's online
backup, which is safe while instrukt uses the store:

    sqlite3 ~/.instrukt/state/history.db ".backup '/path/to/history-before-upgrade.db'"

### Worktree reclamation: declared regenerable output

instrukt now reclaims the disk todo worktrees hold itself: the runner removes
a project's declared regenerable output from idle todo worktrees and releases
a delivered todo's worktree and work branch, and `instrukt repo reclaim`
reclaims a working session's own worktree. What may be removed from a todo
worktree that stays is only what a project declares under the project-layer
key `regenerable`; a project that declares nothing loses no files from it. A
delivered todo's clean worktree is removed whole, with everything git ignores
in it, whatever the project declares.

This repository declares `regenerable` in its `instrukt.yaml`. On a computer
whose installed instrukt predates this release, that key is an unknown key in
this repository's layer: the layer does not validate there, which withholds
history extraction for the project on that computer until instrukt is
upgraded. Upgrade instrukt on each computer that serves this repository.

### One thread-finishing action: `retire_thread` is removed

The runtime action `retire_thread` is gone; `finish_thread` is now the only
action that ends a thread, for a resting conversation and for the orchestrator
of a delivered or withdrawn todo alike.

The machine layer, `~/.instrukt/instrukt.yaml`, moves to global schema 7. The
first writing command of this release on a computer converts it: any
`runtimes.<runtime>.retire_thread` override is dropped, a runtime entry or a
`runtimes` map left empty by that goes with it, and every other setting is
kept. Read-only commands convert it in memory until then. Nothing needs doing
by hand.

Project layers and company homes keep their schema version, so no
repository, this one included, needs a commit for this change.

Once an instrukt of this release has written a computer's machine layer, an
instrukt from before it on the same computer refuses that layer as newer
than it supports. Upgrade every instrukt on a computer together.

### The work record lives on this computer only

GitHub no longer carries a copy of the work record: the issue, its labels,
the board, the native artifacts, the pull request and its reviews show the
todo one way, and the state database holds the only copy. `instrukt todo
refresh` is gone, and a computer no longer lists, picks up or reads todos
another computer created. The state database migrates forward on its own
(Version 40); existing record comments on issues are left where they are.

A todo this computer holds can carry artifact versions whose content only
GitHub has: versions another computer recorded, taken in by an earlier
build's pickup listing without their bodies. The new build reads artifacts
from the local record alone, so it cannot fetch them later. Before installing
it, restore them with the build you have. List the versions without content:

    sqlite3 ~/.instrukt/state/instrukt.db "SELECT t.project, t.slug || '@' || substr(t.id, 1, 8), v.name, v.digest FROM artifact_versions v JOIN todos t ON t.id = v.todo_id WHERE v.content IS NULL"

The list covers every todo this computer retains — live, delivered and
withdrawn ones alike — and names each by its exact address. In a checkout of
each project listed, read each version once with the installed build, which
fetches its content from GitHub, checks it against its digest and keeps it in
the record:

    instrukt todo artifacts diff <address> <name> <digest> <digest>

A version the command reports GitHub no longer serves stays without content.

### Evaluation reports cover the computer fleet; a shared cluster Redis grants the fleet query

`instrukt eval report` now covers every computer that shares this computer's
cluster Redis, unless `--computer` narrows it. Each computer's `instrukt run`
or `instrukt tick` keeps its entry on the fleet membership record and answers
evaluation requests over that cluster connection. The cluster Redis user
therefore needs the fleet query's permissions. A computer without
`redis.cluster.url` is unaffected and still reports on itself alone.

Where the cluster Redis is instrukt's own, the next `instrukt install`
rewrites its access list with these permissions and restarts it. Nothing else
is needed.

Where the cluster Redis is supplied by an operator, its access list is not
instrukt's to change. Before installing the new build on any computer that
configures `redis.cluster.url` there, in this order:

1. Grant the cluster Redis user these commands on instrukt's keys: `HSET` and
   `HGETALL` on `instrukt:fleet:members`; `RPUSH`, `BLPOP`, `LPOP`, `LLEN` and
   `EXPIRE` on `instrukt:eval:requests:*` and `instrukt:eval:reply:*`; and
   `MULTI` and `EXEC`. A user already scoped to `~instrukt:*` needs only the
   commands added, for example
   `ACL SETUSER <user> +hset +hgetall +lpop +llen +rpush +blpop +expire +multi +exec`.
2. On each such computer, run `instrukt install --check`. While any of these
   permissions is still denied, it reports a `fleet Redis` finding telling you
   to grant the fleet query's permissions to the cluster Redis user, then run
   `instrukt install`.
3. Run `instrukt install`.

Until a computer's user is granted these permissions, reports covering it list
it among their limitations, and its runner records the denial in its
`computer-fleet` worker state.
