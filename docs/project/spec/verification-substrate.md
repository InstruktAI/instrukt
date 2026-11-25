---
description: Live verification substrate for instrukt — archetype classification (Rust CLI/daemon/orchestrator), real surfaces, Docker-free isolated-root primitive, concurrency cap, and declared blind spots.
---

# Verification Substrate — Spec

## Required reads

- [sdlc/policy/live-verification](../../domain/sdlc/policy/live-verification.md)
- [project/instrukt/policy/testing](../policy/testing.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](feature/publication/agent-entrypoints.md)

## What it is

Project fill for the `project/<project>/spec/verification-substrate` extension
point declared by `sdlc/procedure/lifecycle/work/verify`, defining `instrukt`'s
archetypes, real observable surfaces, execution host, isolation primitive,
concurrency cap, and declared blind spots (`sdlc/policy/live-verification`).

## Canonical fields

### System archetypes and cheapest honest oracles

| Surface Class | Real Surface | Oracle & Tier |
| --- | --- | --- |
| Deterministic CLI commands, doc validation, rendering, specs, state migrations | Compiled candidate `instrukt` binary against isolated `TestEnv` | Tier 0 — compiler + real-binary `TestEnv` integration tests and `insta` snapshots (`project/instrukt/policy/testing`); skip separate live `verifier` |
| Daemon concurrency, SQLite locking, git hooks, release install, relocatable runtime publication | Built candidate binary (`./target/debug/instrukt`), real temp git repo hooks, live `instrukt tick` / `instrukt run`, and isolated relocatable runtime roots | Tier 1/2 — isolated-root live smoke executing the exact built candidate binary and inspecting filesystem, hook, lock, and rendered output effects |
| Inbound webhooks and company Slack connector | Local loopback webhook receiver and ephemeral local Redis transport | Tier 1/2 — isolated-root smoke with candidate binary + ephemeral local Redis port and signed webhook payloads (or org sandbox Slack credential pool when exercising live Slack delivery) |

### Execution host, isolation primitive, and concurrency

- **Execution host:** `local` (Docker-free isolated-root execution on the
  verification host; runs cleanly on workstations and inside containerized
  company workspaces without container privileges, and requires no dedicated
  live-verification workflow in `.github/workflows/`).
- **Candidate binary:** Build `cargo build -p instrukt-cli` once per candidate
  HEAD and invoke the built candidate executable explicitly
  (`./target/debug/instrukt`), never a `PATH`-resolved installed binary.
- **Isolation primitive:** Follow `project/instrukt/policy/testing` on
  inheriting the home and isolating application roots, and
  `project/instrukt/spec/feature/publication/agent-entrypoints`:
  - Isolate `INSTRUKT_HOME`, `INSTRUKT_LOG_DIR`, T3 root, scheduler roots
    (`SYSTEMD_UNIT_PATH`, `INSTRUKT_LAUNCH_AGENTS_DIR`,
    `INSTRUKT_CRONTAB_FILE`), and only the six relocatable native runtime homes
    (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, `FX_HOME`,
    `KIRO_HOME`, `PI_CODING_AGENT_DIR`) inside per-slot temporary directories
    paired with a temporary git repository.
  - Inherit `HOME`, broad XDG base directories, and `TMPDIR` unchanged
    (`sdlc/policy/testing`).
  - Initialize the slot with the candidate binary (`init`), execute the smoke
    flow, then kill any spawned daemon/Redis process group and delete the
    temporary slot directory on exit (`general/policy/shared-machine-resources`).
- **Max parallel concurrency:** 4 isolated slots per host.
- **Container stack:** None (`verification/Dockerfile.verify` omitted).

### External sandbox credential-pool references

- Org-level sandbox Slack workspace and GitHub App test installation secrets
  (referenced by secret name via `instrukt secrets` or CI secrets, omitting
  secret values from repository files).

## Known caveats

- **Non-relocatable default-only runtime homes:** Antigravity, Cursor, and Grok
  have no selected home-relocation environment variable
  (`project/instrukt/spec/feature/publication/agent-entrypoints`) and are never
  exercised as live filesystem deliveries against a real user home
  (`project/instrukt/policy/testing`). Their rendered output format and
  byte/token budgets are verified through pure rendering checks and isolated
  test fixtures; relocated Claude/fx filesystem checks do not by themselves
  prove runtime context receipt.
- **Remote external delivery:** Real remote GitHub webhook delivery and live
  Slack API round-trips require org-level sandbox credentials; absent those
  credentials on the execution host, live verification covers up to the signed
  loopback webhook boundary and reports the remote-delivery blind spot.

## See Also

- [project/instrukt/policy/reliability-checklist](../policy/reliability-checklist.md)
- [sdlc/procedure/lifecycle/work/verify](../../domain/sdlc/procedure/lifecycle/work/verify.md)
