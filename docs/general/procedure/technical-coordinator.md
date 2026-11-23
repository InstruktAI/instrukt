---
description: Act as technical coordinator in a direct operator session (or another session whose role grants repository, MCP, and todo actions and that did not start from `general/procedure/company-assistance`) at platform level or in a company home — hold one shared technical picture across requesters and repositories, triage requests top-down through the four-lane adopt-before-build ladder, coordinate work through the SDLC todo pipeline, and communicate deliberately in human register.
activation_trigger: 'Use in a direct operator session (at platform level or in a company home) — or another session whose role grants repository, MCP, and todo actions and that did not start from `general/procedure/company-assistance` — when triaging technical requests or deciding whether to adopt, self-host, or build software; sessions started from `general/procedure/company-assistance` stay under that procedure regardless of configured role, and `coordinator` sessions stay under `sdlc/procedure/lifecycle/coordinate`.'
visibility: public
---

# Technical Coordinator — Procedure

## Required reads

- [general/policy/message-identity-headers](../policy/message-identity-headers.md)
- [general/policy/no-output-turn](../policy/no-output-turn.md)
- [general/procedure/mcp-server-tools](mcp-server-tools.md)

## Goal

Hold one coherent technical picture across requesters, repositories, and
sessions; triage every technical request top-down from adopting existing tools
to custom software delivery; coordinate overlapping work through the todo
pipeline; and communicate in concise human register.

### Home-aware posture and session boundary

This procedure runs in a direct operator session — or another session whose
role grants the required repository, MCP, and todo actions and that did not
start from `general/procedure/company-assistance` — in either home scope:

- **Company home** (a direct operator session working in
  `~/.instrukt/companies/<name>/` and its checked-out repositories on requests
  from company founders, admins, or members): stay within that company's home,
  git-backed `docs/`, connected MCP servers, and repositories, and keep
  operator-only host internals with the operator.
- **Platform level** (a direct operator session coordinating platform
  infrastructure, shared domains, and platform repositories): operate across
  the host's configured repositories, published documentation, declared host
  MCP servers, and any adopted deployment domain.

Any connected company conversation started from
`general/procedure/company-assistance` keeps that procedure's contract
regardless of its connector's configured role; this procedure never widens
such a session's authority.

## Preconditions

- You are in a direct operator session — or a session whose role grants the
  required repository, MCP, and todo actions — that did not start from
  `general/procedure/company-assistance` (regardless of configured role).
- A technical request, integration need, architecture question, or software
  workstream has arrived (directly in this session or relayed from company or
  platform stakeholders).
- The active home's git-backed documentation, relevant repositories, and
  `instrukt` CLI surfaces (`instrukt mcp`, `instrukt todo`, `instrukt docs`)
  are reachable.

## Steps

1. **Anchor home scope and shared technical state.** Identify whether this
   session operates for a company home or at platform level. Where relayed
   messages arrive inside platform-framed envelopes, read speaker and context
   from the envelopes (`general/policy/message-identity-headers`). Before
   proposing a solution or opening work, ground in the live shared state
   (`general/principle/grounding`):
   - the home's git-backed `docs/` (recorded systems, architectural decisions,
     and operating conventions);
   - each target repository's active work via `instrukt todo list` (from that
     repository or with `--project <slug>`) and `instrukt todo show <slug>` in
     that repository for overlapping items;
   - declared MCP servers via `instrukt mcp list` and current repository state
     (`git status`, open branches, and recent commits).

   Treat the git-backed `docs/`, active `instrukt todo` ledger, and repository
   state as the single source of shared technical truth across requesters,
   channels, and direct sessions.

2. **Reconcile overlaps across requesters.** Check incoming requests against
   active todos, in-flight branches, and recorded decisions before starting new
   work. When requests from different stakeholders touch the same system,
   compete for priority, or conflict with an earlier decision:
   - surface the overlap and trade-offs explicitly in the coordinating session
     so the decision owners align scope and priority before parallel work
     starts (`general/principle/stewardship`, `general/policy/human-contract`);
   - consolidate related work into one todo or a dependency-ordered sequence
     under `sdlc/policy/todo-creation-discipline`;
   - keep each requester oriented on aligned scope, priority, and status when
     answering them, keeping private direct-message details in the conversation
     where they were shared.

3. **Triage top-down through the four-lane ladder (Adopt before Build).**
   Evaluate every capability request top-down from Lane 1 to Lane 4, grounding
   solution truth in prior art and vendor documentation before proposing custom
   software (`general/principle/grounding`). Stop at the highest lane that
   meets the need cleanly, or enter a custom build lane when the human has
   already settled a bespoke build after reviewing the adopt/self-host
   trade-off (`general/policy/leadership`, `general/policy/human-contract`):
   - **Lane 1 — Existing SaaS or tool + MCP.** Connect a service the company or
     operator already pays for or can adopt economically over MCP. Verify its
     remote endpoint and auth mode via vendor documentation and
     `instrukt mcp discover`, then connect it through
     `general/procedure/company-onboarding/service-connection` (for a company
     home) or host MCP configuration (at platform level).
   - **Lane 2 — Self-host OSS + MCP.** When no suitable SaaS fits cost,
     privacy, or control requirements, deploy a proven open-source application
     on the company's or operator's target container host — using any adopted
     deployment domain when present, or the configured host infrastructure —
     and connect it over MCP.
   - **Lane 3 — Lean reviewed v1 prototype (in-context worktree delivery).**
     When custom glue, a thin MCP adapter, or a focused v1 is needed and the
     work satisfies the settled, bounded, explicitly authorized in-context
     criteria of `sdlc/policy/todo-creation-discipline`, execute it in a
     short-lived one-off worktree without speculative abstractions and route
     its exact committed head through independent commit review before
     integration (`general/policy/commits`,
     `sdlc/procedure/lifecycle/work/review-commits`).
   - **Lane 4 — Steady-state SDLC (`instrukt todo`).** Whenever custom software
     requires planning, builders, handoffs, or contested design judgment under
     `sdlc/policy/todo-creation-discipline` — or starts a new repository via
     `sdlc/procedure/greenfield-intake` — coordinate it through the published
     SDLC pipeline (`instrukt todo list`, `instrukt todo create`,
     `instrukt todo show`).

4. **Execute, coordinate, and record durable truth.** Orient the human before
   consequential boundaries — new vendor commitments, self-hosted operational
   footprint, priority shifts, or irreversible architecture changes
   (`general/policy/leadership`, `general/policy/human-contract`):
   - **Lanes 1–2:** verify service reachability and tool access, then record
     the adopted service and its purpose in the home's git-backed `docs/`
     (authored in a separate worktree and put through independent review before
     integration for company homes, `general/policy/commits`,
     `general/policy/documentation-discipline`).
   - **Lanes 3–4:** follow the owning repository's commit, review, and todo
     filing discipline (`general/policy/commits`,
     `sdlc/policy/todo-creation-discipline`), and report progress from the live
     repository and todo ledger rather than assumption.

5. **Deliver deliberate, human-register replies.** Answer on the active
   session's reply surface in concise human register calibrated to the listener
   (`general/principle/calibration`):
   - state what was checked, the selected lane and next move, current status,
     and any decision owned by the human;
   - keep human-facing summaries free of raw tool traces, command diagnostics,
     and intermediate coordination noise (and in company-facing replies, speak
     in the company's own terms without exposing host internals);
   - when nothing is owed, close the turn with bare `NOOP`
     (`general/policy/no-output-turn`).

## Outputs

- A grounded lane classification (Lanes 1–4) and clear next action for each
  technical request.
- Updated shared technical state: git-backed home `docs/` for adopted tools and
  decisions, reviewed worktree heads for Lane 3 work, or `instrukt todo`
  records for Lane 4 software work.
- Concise, human-register replies keeping affected stakeholders oriented.

## Recovery

- **Adopted SaaS or OSS candidate lacks remote MCP or fails discovery.** Confirm
  against current vendor documentation and `instrukt mcp discover`; either
  scope a thin adapter under Lane 3/4 as `sdlc/policy/todo-creation-discipline`
  decides, or step explicitly to the next lane.
- **In-context prototype work expands beyond settled scope.** Move follow-on
  work into Lane 4 by recording its behavioral contracts and filing a todo
  under `sdlc/policy/todo-creation-discipline`.
- **Missing deployment host, adopted deployment domain, or operator-only
  prerequisite.** Report the unmet prerequisite to the platform operator; in a
  company-facing update, state cleanly what waits without exposing host
  internals.

## Discipline

Evaluate Lanes 1 through 4 top-down before proposing custom software, anchor
every coordination move in the shared git-backed `docs/` and live target-repo
`instrukt todo list`, and leave every session started from
`general/procedure/company-assistance` within that procedure's contract.

## See Also

- [general/procedure/company-assistance](company-assistance.md)
- [general/procedure/company-onboarding/overview](company-onboarding/overview.md)
- [general/procedure/company-onboarding/service-connection](company-onboarding/service-connection.md)
- [sdlc/procedure/greenfield-intake](../../domain/sdlc/procedure/greenfield-intake.md)
- [sdlc/policy/todo-creation-discipline](../../domain/sdlc/policy/todo-creation-discipline.md)
- [sdlc/procedure/lifecycle/work/review-commits](../../domain/sdlc/procedure/lifecycle/work/review-commits.md)
