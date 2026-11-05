---
activation_trigger: Use when an external Git repository must come local (clone or pull) before
  analysis can start.
description: Ingest Git repositories for analysis. Clone or pull into a checkout root,
  produce durable index and history artifacts, and reuse prior work across requests.
visibility: public
---

# Git Repo Ingestion — Procedure

## Required reads

- [general/spec/history-log](../spec/history-log.md)

## Goal

Ingest a Git repository and produce durable artifacts that answer the current objective while preserving a reusable record of what the repository is and how it is organized.

## Preconditions

- `git` is available.
- Repo reference provided as host + owner + repo (preferred) or a full URL.
- An objective describing what to learn or extract from the repo.

## Steps

The checkout lives at `~/.instrukt/git/<host>/<owner>/<repo>/`; `index.md` and `history.md` live beside the checkout. These are re-creatable home artifacts.

1. **Clone or pull the checkout** of `<host>/<owner>/<repo>` (from a URL, or `https://<host>/<owner>/<repo>.git`):
   - No checkout yet: `git clone <url> <checkout>`.
   - Existing checkout: when HEAD is detached, check out the default branch. When the working tree has local changes, do not pull; report them. Otherwise record HEAD, run `git fetch --all --prune` and `git pull --ff-only`, and record HEAD again.

2. **Collect `latest_changes`** — one-line change summaries since the last sync: `git log --oneline <before>..<after>` when HEAD moved, otherwise none.

3. **Derive the artifact path** from the repo reference as `<host>/<owner>/<repo>` (never from the checkout folder name). When given a URL, parse host/owner/repo from it.

4. **Check history first** — read `history.md` and reuse prior answers if the objective is already covered.

5. **Handle incremental changes** — when `latest_changes` is non-empty:
   - Scan the files implied by the change summaries (or related folders).
   - Re-check the entry points referenced in `index.md` (README/docs/examples).
   - Update any affected sections in `index.md` that depend on repo contents.

6. **Ensure `index.md` completeness** — minimum sections:
   - Repo identity and structure
   - Key entry points (README, docs, examples)
   - Root agent file reference (if present)
   - Brief interpretation of how the repo is organized

7. **Update `index.md`** — preserve stable sections; refresh only what changed. Use inline `@` references for every file cited.

8. **Agent file rule** — if a root agent instruction file exists (a shared entrypoint or a runtime-specific instruction file), include exactly one inline `@` reference to it in `index.md`. Do not reference agent files not found in the repo root.

9. **Formulate answer** that satisfies the objective, grounded in repo evidence.

10. **Append to history** using the required history entry format (see required reads).

11. **Respond** with the formulated answer.

## Outputs

- Repo checkout, keyed by `<host>/<owner>/<repo>`.
- Repo artifacts: `index.md` and `history.md`, keyed by `<host>/<owner>/<repo>`.
- Answer delivered to the user.

## Recovery

- If the clone fails, verify network access and repo URL validity.
- If `history.md` is corrupted, delete it and rebuild from a fresh ingestion.
- If `index.md` sections are stale after multiple incremental syncs, regenerate from scratch.
