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

A service the company works with may need setting up for the company before
it can be connected — its accounts, its roles, its records. That is the
service's own procedure; you run it where the company uses that service, and
this procedure names none.

## Preconditions

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

1. **Take the intake.** Ask these, and take the answers as the company's
   facts:
   - What is the company called, and what key names it on this computer — a
     slug, the same as its repository's name?
   - What does it do, and how do its people work?
   - Who owns it and who organizes its work, with each one's email address?
   - Where do its people talk to each other: which messaging provider and
     which workspace there?
   - Which services does its work run on, and what for?

   The name and key lead to step 2, what it does and who leads it to step 3,
   the messaging to step 5, and the services to step 6.
2. **Give the company its repository and its home.** Create the company's
   private repository, one per company, and check it out at
   `~/.instrukt/companies/<name>/`, where `<name>` is the company's key. In it,
   run `instrukt config init --company <name>`, then `instrukt hooks install`:
   instrukt's pre-commit section renders the home's `AGENTS.md` into every
   commit made there. Run `instrukt config validate --company <name>`.
3. **Write the company's knowledge.** In the home, write `AGENTS.master.md`:
   what the company does, how its people work, who owns and who organizes it,
   and where its people find the agent; a longer part goes into a document in
   the home's `docs/` folder that the master names as a required read by its
   path. Commit it in the home.
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
   logs or responses). When it has none, the company cannot be reached in its
   messaging yet: report that as unfinished and continue with the services.
6. **Connect the services.** For each service from the intake that needs
   setting up for the company first, run that service's own procedure. Then
   load `general/procedure/company-service-connection` with `instrukt docs get
   general/procedure/company-service-connection --closure` and run it. Tell
   each person to sign in to a service with the account whose email address
   their messaging workspace shows for them.
7. **Install what the company demands.** Run `instrukt install`, then
   `instrukt install --check` and `instrukt events status`. Each names what is
   unmet or not connected; correct what it names and run them again.
8. **Publish the company's final knowledge.** In the home, run `instrukt docs
   render --check`; it names a stale `AGENTS.md` or a reference the rendering
   refuses, which you correct and commit before going on. With the working
   tree clean, push the home's main to the company's repository and read that
   the repository's main is the home's head. The proof below runs on that
   revision.
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
     person's login is answered.

   Any other outcome is corrected and that part of the proof repeated before
   the company's people start (`general/policy/grounding`, confirmation is
   reserved for inspected claims).
10. **Report to the operator.** Give the home and its repository with the
    revision pushed, the connector and its state, each service's outcome from
    `general/procedure/company-service-connection`, each outcome of step 9,
    and every step still open with what it waits on.

## Outputs

- The company's private repository, checked out as its home on this computer,
  with instrukt's hook sections installed and its settings validated.
- The company's final knowledge, its services included, rendered, committed
  and pushed to the company's repository.
- The company's connector declared and connected, or reported as unfinished,
  and its services declared.
- The proof of step 9 and the report of step 10.

## Recovery

- **A person's message is not answered.** `instrukt events status` names the
  cause; correct what it names and have the person address the agent again.
- **A person is turned away.** The messaging provider does not count them as
  one of the company's own; that is the company's to change in its workspace.
- **A sign-in fails.** The agent's answer gives the failure as the call named
  it; act on what it names. A sign-in not kept for the person is repeated
  with the account whose address their messaging workspace shows.
- **The rendering check fails.** It names the file and the reference; correct
  it, commit, and run step 8 again.
