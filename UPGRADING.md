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

Configuration schema 1 migrates to schema 2 before reads, validation, patches,
publication decisions, installation and ticks. Supported global model choices
move to `agents.yaml`; non-bot git authors remain in `instrukt.yaml`. If a
different `agents.yaml` already exists, conversion refuses rather than discarding
either choice. Reconcile those selections and retry. A failed conversion keeps
the source configuration.

Procedure frontmatter no longer accepts `dispatch`. Configure its role's order
and tier in `agents.yaml`. Model names are open; use `instrukt config show agents`
to inspect effective mappings and `instrukt agents availability` for local
capacity restrictions. T3 retains model-provider credentials.

An explicitly selected `INSTRUKT_HOME` isolates publication and operational state.
Use a distinct absolute path for each isolated worktree. Its secrets still come
from the normal machine home's protected file; do not copy them into a worktree.
