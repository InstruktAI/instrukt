# Upgrading instrukt

## 0.1.0

### Blueprint tool and checkout instructions

For projects with blueprint enabled, make `uv` available on the computer, then
run `instrukt install --module blueprint` followed by `instrukt install --check
--module blueprint`. The shipped adapter now uses the release-pinned Graphify
inside the selected instrukt home. A host `graphify` on PATH no longer supplies
the shipped adapter; existing configured `blueprint.engine` commands retain
their command behavior.

Run `instrukt hooks install` in each adopted repository to retain its
post-checkout section. Existing published projects render their baseline into
new linked worktrees. For a previously created worktree without an entrypoint,
run `instrukt hooks run post-checkout` there. An unpublished linked worktree
requires one `instrukt sync` from its primary checkout first.

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
