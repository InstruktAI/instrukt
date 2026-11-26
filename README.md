<p align="center">
  <img src="assets/logo.png" alt="instrukt logo" width="120">
</p>

<h1 align="center">instrukt</h1>

<p align="center"><strong>Give your agents a playbook, not a 2,000-line prompt.</strong></p>

instrukt turns the documentation in your git repositories into doctrine your coding agents read a piece at a time, right when a task calls for it. On that foundation it adds autonomous software delivery you can step into at any moment, event-driven standing procedures that react to real-world business and operational inputs, and a complete AI workspace for a team — all from one Rust binary, starting with nothing but git.

```sh
curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh
npx skills add InstruktAI/instrukt@v0.1.0 --skill instrukt -g -y
```

Or tell any coding agent **"install instrukt"**. It runs those two commands, then the cockpit skill ([`skills/instrukt/SKILL.md`](skills/instrukt/SKILL.md)) inspects your machine and asks only what it cannot discover itself.

- The installer verifies `SHA256SUMS` and writes only the `instrukt` binary, to `$INSTRUKT_INSTALL_DIR`, `$XDG_BIN_HOME` or `~/.local/bin`. It ends by printing the pinned skill command for the release it installed.
- Prebuilt for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
- `instrukt -h` lists every command. `instrukt status` shows what instrukt sees on this machine and in this repository.

[![Rust 2024 · MSRV 1.89](https://img.shields.io/badge/rust-2024%20%7C%20MSRV%201.89-orange.svg)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](Cargo.toml)
[![unsafe forbidden](https://img.shields.io/badge/unsafe__code-forbid-brightgreen.svg)](Cargo.toml)

---

## The problem

Every team that gets serious about coding agents ends up with the same file. `AGENTS.md` starts as ten helpful lines and grows into two thousand. Every agent reads all of it on every prompt, most of it has nothing to do with the task at hand, and nobody remembers which half is still true. Add a second agent CLI, and now you maintain two copies.

The process side goes the same way. Your engineering workflow lives in a prompt someone pasted, review rigor is a habit, and automation turns into fragile scripts or webhook glue that only its author understands.

A frontier model already knows how to write code. What it cannot know is what is true on your machine, in your codebase and in your business. instrukt keeps that knowledge in git, as documentation you review like code, and gives agents the exact piece a task needs at the moment they need it.

The whole design fits in one line: **mechanisms in Rust, doctrine in git, adapters at the edges.** The binary handles the mechanical work. Your documents define what agents know and how they operate. The agent runtime, the chat platform, the graph engine and the models all sit behind adapters, so none of them locks you in.

---

## Start with git. Turn on the rest as you need it.

You do not adopt all of instrukt at once. Start by running `instrukt init` in one repository to turn messy prompts into versioned, on-demand docs. When you want more, enable each capability in turn:

- **Docs on demand** — structured documentation, a generated `AGENTS.md`, trigger-based retrieval, schema validation, and reusable doctrine repositories you adopt with git.
- **Shared agent setup, history and blast radius** — connect [T3](https://github.com/pingdotgg/t3code) so every coding agent shares one synced global doctrine and searchable conversation history, and add `blueprint` so agents map real code dependencies before editing.
- **Autonomous software delivery** — turn on `todos` for tracked work, prose-routed orchestration, digest-bound independent review, and CI-gated merges to `main` (launched on demand or automatically with `runner.autostart`).
- **Event-driven standing procedures** — turn on `events` so signed external webhooks and real-world triggers (ERPNext, billing, Sentry, CRMs, or custom systems) wake continuing standing agents that run your git-tracked procedures.
- **Business in a box** — create a company folder to give a small or medium business its own AI workspace in Slack, wired to its ERP and tools with per-user sign-in and strict per-company isolation.

You do not have to edit configuration files by hand. Tell your agent "turn on blueprint" or "set up a company", and the cockpit skill patches the settings, converges the setup with `instrukt install`, and verifies it with `instrukt install --check`.

---

## 1. Docs your agents read on demand

### Adopt a repository

```console
$ cd my-project
$ instrukt init
```

`init` turns any git repository into an instrukt-managed project without overwriting your files or making a commit. It establishes the local scaffold and hooks so your repository's documentation becomes the single source of truth for agents:

```text
my-project/
├── AGENTS.md             # generated from docs/project/index.yaml from here on, and committed
├── instrukt.yaml         # the project's settings and slug
├── docs/project/
│   ├── index.yaml        # which snippets load in full, and which as one line
│   └── schema.yaml       # copied from sdlc when sdlc is published
└── .instrukt/schemas/    # JSON Schemas for your editor (git-ignored)
```

It also appends its own marked section to your git hooks without touching existing hooks. If you already had a hand-written `AGENTS.md`, `init` leaves it intact and saves an exact copy at `.instrukt/adopted/AGENTS.md` so nothing is lost.

**Adopting an existing (brownfield) repository.** `init` lays down the mechanical skeleton; intelligence harvests what is already there. The cockpit skill walks through `.instrukt/adopted/AGENTS.md` with you — moving the project's purpose into a project snippet named first in the index's `expanded`, placing each rule into a snippet under `docs/project/` with its index entry, and deleting the kept copy once placed. To capture the rest of the repository, ask your agent to analyze it: `sdlc/procedure/project-analysis` inventories the codebase alongside existing READMEs, API specs, ADRs, and existing `docs/` content, turning durable project knowledge into schema-valid snippets under `docs/project/` and indexing them in `docs/project/index.yaml`.

Commit to `main` and the hooks take over. Every commit overwrites the root `AGENTS.md` with the generated rendering of `docs/project/index.yaml` — your purpose snippet and the rest of the small baseline of always-loaded (`expanded`) snippets, then the compact one-line progressive index — so every clone and cloud agent gets it directly from git. It is the one instruction file a repository carries: nothing below the root is rendered, the same in every agent CLI. Commits on `main` also run `instrukt sync`, registering the repository on your machine and publishing its committed docs to `~/.instrukt/docs/`. Only committed `main` is ever published, so a half-written draft in a working tree never becomes live doctrine.

### What an agent sees

Progressive disclosure works in three tiers, controlled in `index.yaml`:

1. **Always loaded.** The few snippets listed under `expanded` are rendered in full — the core rules every task needs.
2. **Listed.** Everything under `listed` takes one line: its ID, what it covers, and the command that fetches it.
3. **Triggered.** Procedures add one more field to that line: *when* to load them.

Here is a real excerpt from an agent's generated instruction file (wrapped for readability):

```markdown
# Sdlc

Writing, reviewing, testing, refactoring, or debugging code or tests — `instrukt docs get --domain sdlc`
- sdlc/procedure/macos-stable-code-signing: Give locally built macOS artifacts a code identity
  that survives rebuilds … so Full Disk Access and firewall grants outlive recompilation.
  — when: Use when macOS security or firewall prompts recur after rebuilding a locally built
  binary or app bundle … — `instrukt docs get sdlc/procedure/macos-stable-code-signing --closure`
```

The agent runs those commands itself, not you. No model's training data knows your signing setup, so the day macOS starts re-prompting for Full Disk Access after every rebuild, the agent matches the trigger and fetches the procedure: one local signing authority, a stable identifier per executable, and grants that survive recompilation. `--closure` returns the procedure together with every doc it declares as required reading in one call, so the agent never wastes turns chasing references. On any other day, an agent writing a parser never loads a word of it.

You can browse the same index yourself:

```console
$ instrukt docs index --domain sdlc --areas procedure
sdlc/procedure/blast-radius-discovery: Derive the blast radius of an intended code change via the codebase Blueprint …
sdlc/procedure/greenfield-intake: The "what do you want me to build?" game for new projects: apply adopt-before-build triage first, harvest, classify the workload …
sdlc/procedure/network-access-rules: Grant a project's own binaries outbound network access through rules committed in …
…
```

Around that sits a tight validation toolchain: `instrukt docs validate` checks every snippet against its schema, the generated JSON Schemas give your editor autocomplete and inline diagnostics, and `instrukt specs coverage` checks which use cases in your specs have tests bound to them.

### Doctrine is a repository you adopt

Docs live in three places: `docs/project/` holds one repository's own specs and designs; `docs/general/` holds principles that apply everywhere; and `docs/domain/<name>/` holds a reusable body of domain knowledge. instrukt ships no built-in doctrine of its own — you adopt a domain by cloning the repository that publishes it:

```console
$ git clone https://github.com/InstruktAI/erpnext-ops && cd erpnext-ops
$ instrukt init    # registers this checkout and installs its hooks
$ instrukt sync    # publishes its committed docs/domain/erpnext/ to ~/.instrukt/docs/
```

From that moment every project and every company on your machine can retrieve `erpnext/…`. The domain appears in each agent's instruction file as one line plus its triggered procedures, and `git pull` on `main` republishes updates automatically. Each domain has one owner per machine: a second repository claiming `erpnext` is refused, and the error names the current owner.

This repository publishes our own doctrine: `general` (working principles for agents) and `sdlc` (a full software delivery lifecycle). Use them as-is, fork them, or write your own. Two other public domains running beside them today are [`erpnext`](https://github.com/InstruktAI/erpnext-ops), which teaches agents how to keep a company's books, and [`itsup`](https://github.com/Morriz/itsUP), which teaches them the safe GitOps sequence for changing container-host deployments. [How docs repositories work →](docs/project/spec/feature/publication/docs-repositories.md)

---

## 2. One doctrine across every agent, plus history and blast radius

When you connect [T3](https://github.com/pingdotgg/t3code), syncing the repository that owns `docs/general/` asks T3 which agent CLIs you have configured and writes each tool's global instruction file from one source: `~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.gemini/AGENTS.md`, `~/.cursor/rules/instrukt.mdc` and the rest. No copy-pasting rules across tools.

T3 also records every agent session, and `instrukt history search "why did we drop the cache"` searches across all of them — locally or across your machines when connected to a shared Redis.

Turn on `blueprint` and instrukt maintains a structural graph of your codebase as `main` advances:

```console
$ instrukt blueprint affected src/parser.rs
```

Instead of guessing from a quick grep, agents use `sdlc/procedure/blast-radius-discovery` to inspect real dependency impact before making non-trivial edits.

---

## 3. Autonomous software delivery, with you in control

instrukt does not reimplement an agent harness — it pairs cleanly with T3. instrukt owns the durable work record (mirrored to GitHub Issues), role-to-model routing, separate GitHub App identities, digest-bound review stamps, and the merge gate. T3 owns the live agent threads you watch and steer, the isolated git worktrees, and provider logins.

Here is how a piece of work moves from idea to `main`:

1. **You ask in plain words.** In any session, tell your agent what you want built. It answers instrukt's admission questions and files the todo: `instrukt todo create rate-limit-api "…" --kind feature --approved`. The work is stored locally and mirrored to a GitHub issue.
2. **Launch manually, or let the runner pick it up on its tick.** Git and GitHub delivery run on periodic runner ticks — checking git refs, todo state, and GitHub CI with conditional requests and no inbound webhooks required. By default you start work deliberately with `instrukt todo work <slug>`; enable `runner.autostart: true` (globally, per project, or per todo kind) for hands-off pickup. Either way, work launches a `todo-orchestrator` thread in T3 inside its own `<slug>/work` worktree.
3. **Prose routing, not a hard-coded state machine.** The orchestrator reads your published `sdlc` procedures and chooses only the steps the scope earns — fast-tracking a bounded 5-minute bug straight to diagnosis, build, and review, or routing a larger feature through PRD, architecture, and planning first. Every worker runs as a child thread in that worktree.
4. **Independent, digest-bound review.** Each review verdict is stamped against the exact commit SHA and findings report. Edit one line afterwards and the old approval no longer covers the head. Authoring and review act under separate GitHub App identities, so GitHub's own rule against approving your own work applies to agents too.
5. **CI-gated merge.** The standing integrator pushes the reviewed candidate to `integrate`, waits for CI, and fast-forwards `main` to that exact commit only when checks pass.

You can let it run hands-off or step in at any point:

- **Open any thread.** Every orchestrator, builder and reviewer is a live T3 thread — watch it think, or type straight into it to steer.
- **Control when work starts.** Pickup stays manual (`instrukt todo work <slug>`) until you opt in with `runner.autostart: true`.
- **Evolve the process in git.** Want a new review check or a different workflow? Edit the markdown procedure; nothing is hard-coded into the binary.

---

## 4. Event-driven standing procedures: react to the outside world

Git and software delivery do not need inbound events — periodic runner ticks already handle git refs, todos, and GitHub CI polling cleanly.

The **`events`** module unlocks a completely different capability: **standing agents that wake up in real time when the outside world pushes a signal** — business events from your ERP, a failed payment in your billing system, a Sentry production alert, a CRM update, or any external tool that sends a signed webhook.

Instead of building a separate webhook service or wiring brittle Zapier/n8n workflows, you declare the inbound sources in your company's or project's `instrukt.yaml` (behind your own HTTPS edge terminating TLS in front of the local receiver):

```yaml
webhooks:
  erpnext: {secret: ${ERPNEXT_HOOK_SECRET}, previous_secret: ${ERPNEXT_HOOK_PREVIOUS_SECRET}}
  billing: {verify: hmac-sha256, header: Billing-Signature, secret: ${BILLING_SECRET}, name_from: type}
  sentry:  {verify: hmac-sha256, header: Sentry-Hook-Signature, secret: ${SENTRY_SECRET}, name: issue_alert}
  github:  {verify: github, secret: ${GITHUB_HOOK_SECRET}}
```

Each declared domain automatically gets a signed endpoint — `/hooks/inbound/company/<name>/<domain>` for a company home, or `/hooks/inbound/project/<slug>/<domain>` for a project. Then you write an ordinary markdown procedure in git that subscribes to those events:

```yaml
---
description: Investigate failed recurring payments, check the customer's ledger, and follow up.
role: company-agent
subscription_home: company
subscriptions:
  - billing.payment_failed
  - erpnext.invoice_overdue
---
```

Here is what happens when an outside event arrives — say, your billing system POSTs a `payment_failed` webhook to `/hooks/inbound/company/acme/billing`:

1. **Signature verified before parsing.** instrukt checks the HMAC-SHA256 signature over the exact raw request bytes using that company's own `secrets.env` (with zero-downtime `secret` + `previous_secret` rotation). Invalid or undeclared requests are rejected before any body is parsed.
2. **Normalized and appended to the event stream.** The request becomes a canonical `<domain>.<name>` event (`billing.payment_failed`) appended to the cluster event stream before the HTTP request is acknowledged (if the stream is unavailable, the receiver returns `502` so the sender retries rather than dropping the event).
3. **Matched to your published procedures.** Every published procedure whose `subscriptions:` pattern matches is activated durably. (`instrukt events list` shows instrukt's own declared event catalog, while every active procedure subscription appears under "Configured Responsibilities" in the agent's instruction file.)
4. **A continuing standing agent wakes up.** instrukt starts or wakes the standing T3 thread for that procedure and company (or one thread per subject, such as per customer, order, or pull request, when the procedure declares `subscription_identity: subject`). The agent loads its procedure with `--closure`, inspects the payload, acts through your MCP tools and company docs, and keeps its context across subsequent events.

Adding a new reactive business or operations workflow takes one `webhooks:` entry and secret in `instrukt.yaml` plus one committed markdown procedure. And because every standing agent is a real T3 thread, you can open it anytime to inspect what it did or step in directly.

Internal engineering automations ride the exact same standing-procedure primitive too — for example, [`upstream-watch`](docs/general/procedure/upstream-watch.md) wakes when a watched upstream git ref moves (`runner.upstream_changed`) to assess whether a fork needs action, and [`lifecycle/integrate`](docs/domain/sdlc/procedure/lifecycle/integrate.md) wakes when `main` goes red (`runner.main_failing`) to restore the baseline before repairing the cause. [Inbound webhook framework →](docs/project/spec/feature/runner/inbound-webhook-endpoint-framework.md)

---

## 5. Business in a box: a complete AI workspace for a team

If you build AI for small and medium businesses, this is what replaces a one-off custom integration project. A business wants four things from an AI assistant: it should know how the company works, live where the team already talks, operate the company's real software, and act with each person's own permissions. One folder gives it all four.

```console
$ instrukt config init --company acme
```

```text
~/.instrukt/companies/acme/     # a checkout of Acme's own git repository
├── AGENTS.md                   # generated from docs/index.yaml              (committed)
├── docs/                       # who Acme is, its procedures and memory      (committed)
│   └── index.yaml              # which documents load in full, and which as one line
├── instrukt.yaml               # members, connectors, webhooks, MCP servers (this machine only)
└── secrets.env                 # company-scoped tokens and secrets          (this machine only)
```

The folder itself is the registration — a company exists wherever a folder carries a valid `instrukt.yaml`. Because `docs/` and its `index.yaml` live in Acme's own git repo, agents working for Acme can update company procedures and commit them, giving you full git history of everything the business has taught its assistant. The index selects which documents every agent starts with in full and which it reads by one line when the work calls for them.

**Strict separation by construction.** Each company has its own folder and docs, its own settings, its own `secrets.env` (`${NAME}` references resolve exclusively from that company's file, never falling back to the host or another company), its own members and per-user logins, its own chat connections, its own agent threads, and its own MCP servers. (OS-level Linux container isolation per company is [coming next](#built-to-be-pluggable-and-what-comes-next).)

Connecting Slack and the company's software takes a few lines in `instrukt.yaml`:

```yaml
connectors:
  slack:
    app_token: ${SLACK_APP_TOKEN}
    bot_token: ${SLACK_BOT_TOKEN}
mcp:
  erpnext:
    url: https://erp.example.com/api/v2/mcp
    description: Operate the company's ERPNext records
    speaker_login: sign-in
```

### One message, start to finish

Mirjam, who installs kitchens for Acme, sends the assistant a Slack direct message: *"I did six hours at the Jansen job today."*

1. **Delivered over an outbound connection.** The Slack connector opens an outbound Socket Mode connection, so the host needs no public IP or open inbound port for chat.
2. **Attributed to the speaker.** instrukt records that Mirjam sent the message and passes it to the T3 thread bound to that conversation, running inside Acme's folder with Acme's knowledge.
3. **The right procedure loads on demand.** Its instruction file lists the `erpnext` domain adopted in step 1, including the trigger *"Use when a person says how long they worked somewhere…"*. The agent runs `instrukt docs get erpnext/procedure/record-work-hours --closure` and learns the exact company rules: log once, on Mirjam's own employee record, against the named job.
4. **It calls ERPNext as Mirjam.** The agent executes `instrukt mcp call erpnext …`. instrukt attaches the authenticated login of the person being answered — nothing the agent passes can impersonate anyone else. On her first request, Mirjam receives a private sign-in link in her DM, signs in with her own ERPNext account in the browser, and the waiting call finishes in the same turn. From then on, instrukt refreshes her token automatically.
5. **ERPNext enforces her permissions.** The call runs as Mirjam under ERPNext's own access rules — no duplicated permission layer to drift out of sync.
6. **Verified before replying.** The procedure has the agent check for duplicates, submit the timesheet, read the record back, and reply in one clear sentence confirming the job, date, and hours.

No credential ever enters the agent's context — instrukt scrubs tokens from tool output and logs before anything is printed.

The same flow handles bookkeeping and finance: *"The bank kept a €12 fee on the Bakker payment"* triggers `erpnext/procedure/customer-payment-difference` to reconcile the invoice cleanly without creating phantom credit; *"March is finished, close it"* loads `erpnext/procedure/close-accounting-period`; and `erpnext/procedure/prove-native-effect` ensures the agent verifies every ledger mutation from the actual record before confirming.

### What else a company gets

- **Multiple named roles in chat.** Declare more than one named connector in the same company — for example, a general `@agent` for everyday team work alongside a dedicated `@coder` (`kind: slack`) acting as the company's in-house technical builder and coordinator. Each has its own admission policy, session scope (`conversation` or shared `company`), and reply mode (`turn_output` or `explicit` via `instrukt company reply`), so internal reasoning never leaks into customer or team channels.
- **Connect new tools by asking.** A workspace admin can DM the assistant *"connect our CRM"*. The agent probes the MCP endpoint live with `instrukt mcp discover <url>`, reads its OAuth/OIDC metadata, and wires it into the company's config — restricted strictly to admins, in direct messages, for that company alone.
- **Sign-in without running Keycloak.** `instrukt mcp issuer init --company acme` gives the company a lightweight built-in OIDC issuer. Register its discovery URL once with a service supporting enterprise-managed authorization and team members sign in seamlessly without a separate identity server.
- **Full visibility for admins.** Every company conversation and standing agent is a real T3 thread you can inspect, guide, or jump into directly.

---

## Built to be pluggable, and what comes next

Everything above keeps working when the infrastructure around it changes, because your doctrine and procedures never hard-code a vendor. A procedure says *"the reviewer role reviews this commit"* or *"handle `billing.payment_failed`"* — never which session runtime runs the agent, which model answers, or which chat app delivered the message. Those live behind adapters at the edges:

- **Agent runtimes:** [T3](https://github.com/pingdotgg/t3code) is the first runtime adapter (`runner.runtime`), connected through a single adapter trait and configurable instruction templates. **Next:** additional session hosts such as **Herdr** drop in behind the same interface while your docs, todos, events and procedures stay untouched.
- **Chat connectors and webhook providers:** Slack over outbound Socket Mode is the first chat connector on the shared company connector interface, and raw-body HMAC-SHA256 / GitHub webhooks are the first inbound event schemes. **Next:** **Microsoft Teams** for organizations living in Microsoft 365, **Mattermost** for self-hosted, privacy-first teams, and additional provider adapters such as **WhatsApp** or timestamp-signed webhooks — sharing the exact same company folder, members, procedures and MCP tools.
- **Models:** Roles map cleanly to model profiles in `agents.yaml` while the runtime supplies the providers. **Next:** first-class routing for **local and self-hosted models** alongside cloud frontier models, so roles touching sensitive business or financial data can stay on your own hardware while other roles optimize for capability or cost.
- **Company isolation:** Today each company has strict separation of files, git history, secrets, members, logins, connectors, webhooks and MCP servers. **Next:** **per-company Linux containers** ([spec](docs/project/spec/feature/runner/company-containers.md)), allowing many companies to share one host with full OS-level isolation.
- **Doctrine and code graphs:** Both are pluggable from day one — `general` and `sdlc` are ordinary git docs you can fork or replace, and `blueprint`'s graph engine sits behind its own adapter.

---

## Go deeper

This repository is documentation-first, written using the exact system it publishes:

- [Context map](docs/project/spec/domain/context-map.md) — how the domains of instrukt fit together
- [Architecture overview](docs/project/design/architecture/overview.md) — how the Rust workspace and adapters are structured
- [Docs repositories](docs/project/spec/feature/publication/docs-repositories.md) — adopting, forking and layering doctrine
- [Inbound webhook framework](docs/project/spec/feature/runner/inbound-webhook-endpoint-framework.md) — signed project and company event endpoints
- [Company homes](docs/project/spec/feature/configuration/company-homes.md) — what lives in a company folder
- [Cockpit skill](skills/instrukt/SKILL.md) — what your agent runs when you say "install instrukt"

---

## Development

One Cargo workspace ([`Cargo.toml`](Cargo.toml)), Rust 2024, MSRV 1.89, `unsafe` code forbidden, one binary.

```sh
make hooks          # once per clone: pre-commit runs the checks the staged files need
make commit-checks  # every commit check over every file, as CI runs it
make ratchet        # lower the suppression baseline after removing a suppression
make check          # lint, the whole test suite, coverage and the audit by hand
```

- Commit checks use the `pre-commit` framework at the version pinned in [`scripts/test-tools.env`](scripts/test-tools.env) (`uv tool install pre-commit==<version>`). [`.pre-commit-config.yaml`](.pre-commit-config.yaml) maps each check to its files, and the full functional suite runs in CI on every pull request.
- `make hooks` installs the pinned `ruff` and `mypy` hooks into `~/.cache/pre-commit` and needs network access. A commit made before that, or after a pinned tool changes, installs what it needs itself.
- Command-line integration tests run the compiled binary against isolated temporary `HOME` directories and temporary git repositories ([`crates/instrukt-cli/tests/common`](crates/instrukt-cli/tests/common)). They never touch your live home or agent settings.
