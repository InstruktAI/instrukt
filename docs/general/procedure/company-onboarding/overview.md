---
description: Onboard a company — take the intake, have instrukt generate the company's repository and its home on this computer, write its knowledge, connect the messaging its people talk in and the services its work runs on, hand its reviewed knowledge to its repository's integrator, and prove that a person of the company asks its agent and is answered; also the routes for connecting or repairing an onboarded company's services and for turning on software building for an onboarded company.
activation_trigger: Use when a company is to be onboarded, when an onboarded company's setup is to be brought in line with this procedure or a step of it is to be found, when an onboarded company's services are to be connected or repaired, or when software building is to be turned on for an onboarded company.
role: company-operator
visibility: public
---

# Company Onboarding — Procedure

## Required reads

- [general/policy/onboarding](../../policy/onboarding.md)

## Goal

After onboarding, the company's people address the company's agent where they
already talk and are answered there, from the company's own knowledge and
through the services the company works with, each person under their own login
at each service. The company's repository holds its reviewed knowledge, its
home on this computer is that repository's checkout, and the operator holds the
proof. You are the operator's agent setting the company up, and every company
goes through the same onboarding.

### Platform onboarding and company onboarding

- **Platform onboarding** (the `instrukt` cockpit skill) sets up this computer
  itself: installing `instrukt`, adopting repositories, the runtime, model
  profiles, the GitHub Apps and the computer's modules. Host-level changes go
  through it.
- **Company onboarding** (this procedure) runs where platform onboarding
  stands. It gives one company its home, its knowledge, its messaging and its
  services, and proves business assistance
  (`general/procedure/company-assistance`). Software building for a company is
  a later addition the operator asks for (step 11); baseline onboarding never
  raises it.

A service the company works with may need setting up for the company before
it can be connected — its accounts, its roles, its records. That is the
service's own procedure; you run it where the company uses that service, and
this procedure names none.

### Working with the human

- Work with them as `general/policy/onboarding` sets out: you run and verify
  everything, and lead each step only they can take one action at a time.
- A secret — a token, a key, a password — never passes through the
  conversation. The human enters it in the runtime's private secret entry,
  which hands you a file; you store it with `instrukt secrets set --company
  <key> <NAME> --from-file <file>` (`sdlc/policy/security-awareness`).
- Name no service or provider the company has not named.
- State one caveat, once, in step 1: every message the company's assistant
  answers uses the AI subscription or usage configured on this computer. Make
  no other claim about the computer, its isolation or how many companies it
  holds.

## Preconditions

- Platform onboarding stands on this computer: `instrukt status --json`
  reports `machine.configured: true` with no `machine.findings`, a selected
  `runtime`, and `modules.features.standing_agents.available: true`;
  `instrukt agents list` reports the configured agent profiles.
- You work on the computer that is to hold the company's home.
- The operator, or someone the operator names who knows the company, answers
  the intake and is at hand for the steps that are theirs.

## Steps

**Routes.** Onboarding a company, or bringing an onboarded one in line, runs
steps 1 to 10 in order, skipping what already stands. Connecting or repairing
an onboarded company's services runs step 6, then steps 7 and 8, then step 9
for the services it touched, then step 10. Turning on software building for an
onboarded company runs step 11, then step 7, then step 10.

1. **State the caveat and take the intake.** Tell the operator, in one plain
   sentence, that every message the company's assistant answers uses the AI
   subscription or usage configured on this computer. Then ask these, and
   take the answers as the company's facts:
   - What is the company called?
   - What does it do, and how do its people work?
   - Who owns it and who organizes its work, with each one's email address?
   - Where do its people talk to each other: which messaging app, and which
     workspace there?
   - Which services do you use?
   - Who looks after the company's GitHub account, and what is their GitHub
     username?

   From the name, choose the company's key: a short name in lowercase letters,
   digits and dashes that names the company on this computer. Tell the
   operator the key as a statement they can correct. The company's repository
   is named `company-<key>`; the key and the repository's name are not the
   same. The key and the name lead to step 2, what it does and who leads it to
   step 3, the messaging to step 5, and the services to step 6.
2. **Have the company's home generated.** Run `instrukt init --company <key>
   --name "<name>" --admin <github-username>`, declaring `--human-involved` as
   `general/policy/human-involvement-recording` directs. instrukt creates the
   company's private repository `company-<key>` in the GitHub organisation
   this computer declares as the owner of company homes, with the skeleton
   every company receives, makes the company's GitHub administrator an
   administrator of it, and checks it out as the company's home, ready as a
   project like any other. Where the repository already exists, it checks that
   repository out as the home and changes nothing on GitHub; the name and the
   username are then not used. Act on what the command reports:
   - **No owning organisation is declared, or the declared one is no
     organisation.** Ask the operator which GitHub organisation owns the
     companies they onboard. Where they have none, lead them to create one on
     GitHub (it is free); their part is done when the organisation's page
     opens for them. Set it with `instrukt config patch --global --revision
     <rev>` as `github.org`, from the revision `instrukt config get --global
     --revision` gives, and run the command again.
   - **The project's GitHub App cannot create repositories in that
     organisation.** The refusal names the App, the organisation and the
     permission. Lead the operator, one action at a time, to install that App
     on the organisation, or to grant its installation that permission; their
     part is done when GitHub shows the App installed on the organisation with
     it. Run the command again.
   - **The company already has a home on this computer.** Nothing is changed;
     that step stands. Continue with the next step that does not.
   - **The command was interrupted.** Run it again with the same key; it
     completes the creation it recorded.
   - **Automatic review is withheld for the home.** The report names what
     this computer is missing for it. Correct it through platform onboarding,
     or carry it to step 10 as open.

   Where the repository was created and the company's GitHub administrator
   is not a member of the owning organisation, GitHub has sent them an
   invitation. Lead them to accept it: one action, from GitHub's email or
   their GitHub notifications for `company-<key>`; their part is done when the
   repository opens for them. Run `instrukt config validate --company <key>`.
3. **Write the company's knowledge.** Author it in a separate worktree of the
   company's repository on a new branch, never in the home itself, so the home
   stays on the reviewed default branch (`general/policy/commits`). Place the
   company's knowledge — what it is for, what it does, how its people work,
   who owns and organizes it, and where people reach the agent — in documents
   under `docs/`, and name in `docs/index.yaml` under `expanded` the documents
   every agent of the company starts with and under `listed` the ones an agent
   reads when they apply, each by its path relative to `docs/`. Follow the
   company knowledge rules in `general/procedure/doc-snippet-authoring` and
   `general/policy/snippet-value-contract`: present-tense current truth with
   no history, and no reference outside the company's own documents. Commit
   in the worktree with `instrukt repo commit`.
4. **Choose the agent's profiles, where the company needs its own.** When the
   company's agent is to run on other agent profiles than the computer's,
   declare `profiles` as `instrukt config schema --company <key> profiles`
   shows, with the names `instrukt agents list` gives; otherwise leave it out.
5. **Connect the messaging.** Run `instrukt config schema --company <key>
   connectors`. It shows each connector kind instrukt has, with the fields its
   declaration takes; `instrukt config schema --company <key>
   connectors.<kind>` shows that kind's setup at the provider and the app it
   installs. When the company's messaging app has a kind there:
   - Prepare the app from that setup. Lead the workspace's administrator
     through creating and installing it, one action at a time, each with what
     you prepared and what they see when it is done.
   - For each token the provider issues, the administrator enters it in the
     private secret entry and you store it under the name the setup gives.
     Their part is done when the provider shows the app installed to the
     workspace and every token is stored.
   - Declare the connector with `instrukt config patch --company <key>
     --revision <rev>` from the revision `instrukt config get --company <key>
     --revision` gives. For a Slack connector, the app's bot scopes include
     `channels:read`, `groups:read` and `im:read`; when you add a scope to an
     app already installed, the administrator reinstalls it to the workspace
     and enters the bot token it then issues.
   - Run `instrukt config validate --company <key>`; it names each token that
     is missing or blank, which you have entered and stored, and validate
     again.

   When the company's messaging app has no kind there, the company cannot be
   reached in its messaging yet: report that as unfinished and continue with
   the services.
6. **Connect the services.** For each service the company uses that needs
   setting up for the company first, run that service's own procedure. Then
   load `general/procedure/company-onboarding/service-connection` with
   `instrukt docs get general/procedure/company-onboarding/service-connection`
   and run it for the services the intake named. Tell each person to sign in
   to a service with the account whose email address their messaging
   workspace shows for them.
7. **Install what the company needs.** Make sure this computer publishes the
   procedure for the role `company-agent`: run `instrukt docs get
   general/procedure/company-assistance`. When it is not found, clone the docs
   repository that publishes it, adopt it with `instrukt init` and run
   `instrukt sync` in it. Then run `instrukt install`, then `instrukt install
   --check` and `instrukt events status`. Each names what is unmet or not
   connected; correct what it names and run them again.
8. **Hand the company's knowledge to integration.** In the worktree holding
   the company's committed documents, run `instrukt docs render --check`; it
   names a stale `AGENTS.md` or a reference the rendering refuses, which you
   correct and commit before going on. Push the branch and open its pull
   request. Its head is reviewed automatically where this computer admits
   automatic review for the home; otherwise obtain an independent review bound
   to that head (`general/policy/commits`). Hand the reviewed head to the
   company repository's integrator, as every adopted project's reviewed
   worktree head is handed; once it is integrated, the home holds it as every
   adopted project's checkout holds its default branch, and you remove the
   worktree. Where the head cannot be integrated yet, keep the reviewed
   worktree and report it as open in step 10. The proof of step 9 runs on the
   home once it holds the knowledge.

   The integrator renders the company's skill with its catalog entry, and the
   `AGENTS.md`, into every candidate it promotes, under the company identity
   the home's committed `instrukt.yaml` declares with `company_name`; nothing
   else renders them onto the default branch. So:
   - In the worktree, `instrukt docs render --plugin --check` names the skill
     as stale until the knowledge is integrated; that is expected, and Claude
     serves the previous skill until the integrated revision is promoted.
   - Once the home holds the integrated knowledge, run `instrukt docs render
     --plugin --check` there; it passes with nothing named. Where it names the
     skill or `AGENTS.md`, report it as open in step 10.
   - Where `instrukt config validate` names the home's missing company
     identity, the integrator renders nothing and instrukt withholds the
     home's event responses until it is repaired: patch `company_name` into
     the project layer in a worktree with `instrukt config patch`, as the
     finding names, and hand that change to integration as above.
   - A repository that carries the shared workflows repository's publishing
     workflow keeps it, and that workflow then writes the default branch
     beside the integrator. Report it in step 10 when you find it.
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
10. **Report to the operator.** In their words: the company's home and its
    repository, and whether the repository was created or already existed;
    the messaging and its state; each service's outcome from
    `general/procedure/company-onboarding/service-connection`; each outcome of
    step 9; and every step still open with what it waits on.
11. **Turn on software building.** Run this when the operator asks for
    software or apps to be built for an onboarded company. Skip each part
    that already stands, so running it again changes nothing more.
    - **Where the company's software lives.** Ask the operator which GitHub
      organisation of the company's own its software starts in. Where the
      company has none, lead them to create one on GitHub (it is free); their
      part is done when its page opens for them. Record in the company's
      knowledge, as in step 3, that software building is turned on for the
      company, the organisation its software repositories start in, and that
      its people reach the coding assistant through its own app in their
      messaging workspace; then run step 8 for it. Skip this part when the
      company's knowledge already says so.
    - **The coding assistant's app.** Declare a second connector named
      `coder`, of the same kind as the company's messaging connector, with
      `role: technical-coordinator`, `admit: admins_only` and `session_scope:
      company`, prepared and installed as step 5 does: its own app in the
      company's workspace, set up one action at a time with the workspace's
      administrator, and its own tokens entered in the private secret entry
      under names of their own. Skip this part when `instrukt config get
      --company <key> connectors.coder` shows it declared and `instrukt config
      validate --company <key>` reports nothing for it.

## Outputs

- The company's private repository, generated or taken as it exists, checked
  out as its home on this computer, an adopted project with its settings
  validated.
- The company's knowledge, its services included, rendered, reviewed and
  handed to its repository's integrator, or reported as open.
- The company's messaging connector declared and connected, or reported as
  unfinished, and its services declared.
- The proof of step 9 and the report of step 10.
- When software building is turned on: the company's software organisation
  recorded in its knowledge and the coding assistant's connector declared.

## Recovery

- **A person's message is not answered.** `instrukt events status` names the
  cause; correct what it names and have the person address the agent again.
- **A person is turned away.** Hold the connector against its kind's setup
  in `instrukt config schema --company <key> connectors.<kind>` and correct
  what differs; when it matches, report the person to the operator.
- **A request to a service fails for the person.** The person hears only
  that their service could not be reached or refused them, or that a sign-in
  waits for them. Run `instrukt events status`. A company's own servers are
  served outside any session only, so lead the operator to run `instrukt mcp
  list-tools <server> --company <key>` in a terminal of their own: one action,
  the command you prepared, done when they give you its output. Act on what
  those name with the recovery of
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
