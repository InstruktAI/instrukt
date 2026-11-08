# Upgrading instrukt

## 0.1.0

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

Unattended starts require the adapter's verified T3 CLI/server and WebSocket
protocol versions. Unknown versions or readiness response shapes refuse startup
with a repair reason. The adapter obtains and revokes a five-minute read-scoped
session for each new start's provider snapshot. Quota restrictions now come from
reported usage windows, with a recorded one-hour expiry when no reset is reported;
launch-error text supplies no quota or no-start evidence.
