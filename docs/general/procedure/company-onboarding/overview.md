---
description: Onboard a company — take the intake, give the company its repository and its home on this computer, its knowledge, the connector its people talk through and the services its work runs on, publish its final knowledge, and prove that a person of the company asks its agent and is answered; also the route for connecting or repairing an onboarded company's services.
activation_trigger: Use when a company is to be onboarded, when an onboarded company's setup is to be brought in line with this procedure or a step of it is to be found, or when an onboarded company's services are to be connected or repaired.
role: company-operator
visibility: public
---

# Company Onboarding — Procedure

## Goal

After onboarding, the company's people address the company's agent where they
already talk and are answered there, from the company's own knowledge and
through the services the company works with, each person under their own login
at each service. The company's repository holds its final knowledge, and the
operator holds the proof. You are the operator's agent setting the company up.

### Platform onboarding vs. company onboarding

Keep the two onboarding layers distinct:

- **Platform onboarding** (the `instrukt` cockpit skill): sets up the host
  computer itself — installing `instrukt`, adopting documentation and project
  repositories, configuring the runtime, model profiles, and GitHub Apps, and
  enabling host modules. Run and re-open the cockpit skill for host-level
  changes.
- **Company onboarding** (this procedure): runs on a computer where platform
  onboarding already stands — creating one company's private repository and
  home, declaring its members, messaging connector, and company knowledge,
  wiring its business services through
  `general/procedure/company-onboarding/service-connection`, and proving
  baseline business assistance (`general/procedure/company-assistance`).
  Technical coordination for a company (`general/procedure/technical-coordinator`)
  is a separate downstream workflow run from a direct operator session (or
  another session whose role grants repository, MCP, and todo actions and that
  did not start from `general/procedure/company-assistance`) rather than inside
  a company conversation started from `general/procedure/company-assistance`.

A service the company works with may need setting up for the company before
it can be connected — its accounts, its roles, its records. That is the
service's own procedure; you run it where the company uses that service, and
this procedure names none.

## Preconditions

- Platform onboarding stands on this computer: `instrukt status --json`
  reports `machine.configured: true` with no `machine.findings`, a selected
  `runtime`, and `modules.features.standing_agents.available: true` (no host
  module switch under `modules` is required for company onboarding);
  `instrukt agents list` reports the configured agent profiles.
- You work on the computer that is to hold the company's home.
- The operator is at hand to run, at their own terminal, the commands this
  procedure gives them, and to give you their output back.
- The operator, or someone the operator names who knows the company, answers
  the intake.
- The operator can create a private repository for the company and grant
  access to it.

## Steps

**Routes.** Onboarding a company, or bringing an onboarded one in line, runs
the steps in order, skipping what already stands. Connecting or repairing an
onboarded company's services runs step 6, then steps 7 and 8, then step 9 for
the services it touched, then step 10.

1. **Surface upfront caveats and take the intake.** Before creating a company
   home or connecting channels, orient the operator on three operational
   boundaries (`general/policy/leadership`, `general/policy/human-contract`):
   - **Compute and token budget:** every addressed company conversation turn
     runs against the configured model profiles and provider usage budget
     (company-specific profiles are scoped in step 4). Confirm budget
     expectations and account ownership before enabling active channels.
   - **Host isolation boundary:** on a single shared host without per-company
     containers, company homes are separate folders under one operating-system
     user rather than OS-isolated sandboxes; multi-tenant filesystem and
     runtime isolation requires per-company containers on a Linux host (macOS
     is not a multi-tenant company host). State the computer's current
     isolation topology explicitly before placing the company on it.
   - **Technical coordination and deployment infrastructure stay opt-in and
     downstream:** baseline company onboarding delivers business assistance
     (`general/procedure/company-assistance`) and connected business MCP
     services only. Technical coordination for the company
     (`general/procedure/technical-coordinator`) — and any deployment-host,
     container-runtime, or GitOps setup — is handled separately downstream in
     a direct operator session (or another session whose role grants
     repository, MCP, and todo actions and that did not start from
     `general/procedure/company-assistance`) after baseline onboarding stands,
     never inside baseline business onboarding or a company conversation
     started from `general/procedure/company-assistance`.

   Then ask these, and take the answers as the company's facts:
   - What is the company called, and what key names it on this computer — a
     slug, the same as its repository's name?
   - What does it do, and how do its people work?
   - Who owns it and who organizes its work, with each one's email address?
   - Where do its people talk to each other: which messaging provider and
     which workspace there?
   - Which services does its work run on, and what for?
   - Does this company stay on business assistance only, or does the operator
     also plan downstream technical coordination
     (`general/procedure/technical-coordinator`) for the company's repositories
     after baseline onboarding?

   The name and key lead to step 2, what it does and who leads it to step 3,
   the messaging to step 5, and the services to step 6. Any downstream
   technical-coordination, deployment-host, or GitOps setup is recorded in the
   step 10 handoff as separate post-onboarding work after baseline onboarding
   is proven.
2. **Give the company its repository and its home.** Create the company's
   private repository, one per company, and check it out at
   `~/.instrukt/companies/<name>/`, where `<name>` is the company's key.
   Protect its default branch under `general/policy/commits` so human and agent
   changes land through reviewed worktree heads and integration rather than
   direct pushes. Enable host-side branch protection or rulesets where the
   hosting plan supports them for the repository (on GitHub, public
   repositories, private personal repositories on GitHub Pro, or private
   organization repositories on GitHub Team and above; restricting push actors
   requires a plan feature that supports push restrictions or rulesets). Where
   the hosting plan does not support host-side branch protection — such as a
   private organization repository on GitHub Free — agent delivery still routes
   through reviewed worktree heads and instrukt's integration gate, while
   host-side push prevention is unavailable and a collaborator with write access
   can still push directly to the default branch unless repository permissions
   and operational discipline prevent it; upgrade the hosting plan when
   host-side push enforcement is required. Automated skill publication must use a route compatible with
   `general/policy/commits`'s single-writer rule rather than granting a second
   direct writer on the default branch; where the repository's skill publisher
   still requires pushing directly to the default branch, leave that part of
   rollout open and report it in step 10. In the home, run `instrukt config
   init --company <name>`, then `instrukt hooks install`: instrukt's pre-commit
   section renders the home's `AGENTS.md` into every commit made there. Run
   `instrukt config validate --company <name>`.
3. **Write the company's knowledge.** Author the company's knowledge in a
   separate worktree rather than in the serving checkout at
   `~/.instrukt/companies/<name>/`, so the serving home stays on the reviewed
   default branch throughout authoring and review (`general/policy/commits`).
   Place the company's knowledge — what it is for, what it does, how its
   people work, who owns and organizes it, and where people reach the agent —
   in documents under `docs/`, and create `docs/index.yaml` naming under
   `expanded` the documents every agent of the company starts with and under
   `listed` the ones an agent reads when they apply, each by its path
   relative to `docs/`. Follow the company home
   knowledge rules in `general/procedure/doc-snippet-authoring` and
   `general/policy/snippet-value-contract` (present-tense current truth without
   historical narration; self-contained with no references outside the
   company's own documents). Commit in the worktree.
4. **Choose the agent's profiles, where the company needs its own.** When the
   company's agent is to run on other agent profiles than the computer's,
   declare `profiles` as `instrukt config schema --company <name> profiles`
   shows, with the names `instrukt agents list` gives; otherwise leave it out.
5. **Connect the messaging.** Run `instrukt config schema --company <name>
   connectors`. It shows each connector kind instrukt has, with the fields its
   declaration takes and its setup at the provider. When the company's
   messaging provider has a kind there, carry out that setup in the company's
   workspace, declare the connector with `instrukt config patch --company
   <name> --revision <rev>` from the revision `instrukt config get --company
   <name> --revision` gives, and store each token it references through
   `instrukt secrets set --company <name> <NAME>` from a file or redirected
   standard input, never from the conversation
   (`sdlc/policy/security-awareness`, secrets never enter version control,
   logs or responses). For a Slack connector, the app's bot scopes include
   `channels:read`, `groups:read` and `im:read`; when you add a scope to an
   app already installed, reinstall it to the workspace and store the bot
   token it then issues. Run `instrukt config validate --company <name>`; it
   names each token that is missing or blank, which you store and validate
   again. When the company's provider has no kind there, the company cannot
   be reached in its messaging yet: report that as unfinished and continue with the services.
6. **Connect the services.** For each service from the intake that needs
   setting up for the company first, run that service's own procedure. Then
   load `general/procedure/company-onboarding/service-connection` with
   `instrukt docs get general/procedure/company-onboarding/service-connection`
   and run it. Tell
   each person to sign in to a service with the account whose email address
   their messaging workspace shows for them.
7. **Install what the company demands.** Make sure this computer publishes
   the procedure for the role `company-agent`: run `instrukt docs get
   general/procedure/company-assistance`. When it is not found, clone the docs
   repository that publishes it, adopt it with `instrukt init` and run
   `instrukt sync` in it. Then run `instrukt install`, then
   `instrukt install --check` and `instrukt events status`. Each names what is
   unmet or not connected; correct what it names and run them again.
8. **Publish the company's final knowledge.** In the separate worktree holding
   the company's committed documents, run `instrukt docs render --check`; it
   names a stale `AGENTS.md` or a reference the rendering refuses, which you
   correct (running `instrukt docs render` if hooks did not regenerate
   `AGENTS.md`) and commit before going on. Push the worktree branch and open
   the pull request so PR validation checks run, and obtain independent commit
   review bound to that committed head (`general/policy/documentation-discipline`,
   `general/policy/commits`). When the repository can verify and promote its
   exact integration candidate under `general/policy/commits`, hand the
   reviewed head to integration once checks and review pass, fast-forward the
   serving company home checkout to the repository's updated default branch,
   and remove the authoring worktree; the proof below runs on that revision.
   When the repository carries only PR checks (no candidate verification for
   integration) or its automated publisher is not yet compatible with
   `general/policy/commits`, retain the reviewed worktree, keep the serving
   home checkout at its current default branch, and report publication as open
   in step 10.
9. **Prove that a person is answered.** With a person of the company present,
   have them address the agent where they talk, and read:
   - the answer arrives in the same conversation and nowhere else;
   - asked what it can do, the agent names the company's connected services
     and that it keeps the company's knowledge, and nothing else;
   - for each service declared with a person's sign-in, a request that needs
     it is answered by one of these, each a success: the person already holds
     a login there and is answered; or a sign-in link reaches them in a
     direct message, they sign in, and the same turn answers; or they sign in
     after the agent's answer says the sign-in is pending, ask again, and are
     answered;
   - for each service declared with a stored login, a request under that
     person's login is answered;
   - whatever the person is told stays in their world and their words: a
     service of theirs that cannot be reached or refuses them, or a sign-in
     they have to complete. Nothing of instrukt's own reaches them — no
     command, setting, reference or diagnostic.

   Any other outcome is corrected and that part of the proof repeated before
   the company's people start (`general/policy/grounding`, confirmation is
   reserved for inspected claims).
10. **Report to the operator.** Give the home and its repository with the
    revision pushed, the connector and its state, each service's outcome from
    `general/procedure/company-onboarding/service-connection`, each outcome
    of step 9, and every step still open with what it waits on. On the full
    onboarding route (when step 1 ran), also report the isolation topology
    confirmed in step 1 and whether downstream technical coordination
    (`general/procedure/technical-coordinator`) or deployment-host/GitOps
    follow-up was planned as separate post-onboarding work.

## Outputs

- The company's private repository, checked out as its home on this computer,
  with instrukt's hook sections installed and its settings validated.
- The company's final knowledge, its services included, rendered, reviewed,
  merged to the company repository's default branch, and checked out in the
  home.
- The company's connector declared and connected, or reported as unfinished,
  and its services declared.
- The proof of step 9 and the report of step 10.

## Recovery

- **A person's message is not answered.** `instrukt events status` names the
  cause; correct what it names and have the person address the agent again.
- **A person is turned away.** Hold the connector against its kind's setup
  in `instrukt config schema --company <name> connectors.<kind>` and correct
  what differs; when it matches, report the person to the operator.
- **A request to a service fails for the person.** The person hears only
  that their service could not be reached or refused them, or that a sign-in
  waits for them. The cause is the operator's: have the operator run
  `instrukt events status` and `instrukt mcp list-tools <server> --company
  <name>` at their own terminal, act on what those name with the recovery of
  `general/procedure/company-onboarding/service-connection`, and report what
  neither names as open. A sign-in not kept for the person is repeated with
  the account whose address their messaging workspace shows.
- **Instrukt's own diagnostic reached the person.** That part of the proof
  fails; report it to the operator with the words the person was given.
- **The rendering check fails.** It names the file and the reference; correct
  it, commit, and run step 8 again.

## See Also

- [general/procedure/company-onboarding/service-connection](service-connection.md)
- [general/procedure/company-assistance](../company-assistance.md)
- [general/procedure/technical-coordinator](../technical-coordinator.md)
