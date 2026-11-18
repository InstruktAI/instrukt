---
description: Onboard a company — take the intake, give the company its repository and its home on this computer, its knowledge, the connector its people talk through and the services its work runs on, and prove that a person of the company asks its agent and is answered.
activation_trigger: Use when a company is to be onboarded, when an onboarded company's setup is to be brought in line with this procedure, or to find where a step of a company's onboarding stands.
visibility: public
---

# Company Onboarding — Procedure

## Goal

After onboarding, the company's people address the company's agent where they
already talk and are answered there, from the company's own knowledge and
through the services the company works with, each person under their own login
at each service. The operator holds the proof. You are the operator's agent
setting the company up.

A service the company works with may need setting up for the company before
it can be connected — its accounts, its roles, its records. That is the
service's own procedure; you run it where the company uses that service, and
this procedure names none.

## Preconditions

- You work on the computer that is to hold the company's home, outside any
  session of a company: the `--company` path of `instrukt config`,
  `instrukt secrets` and `instrukt mcp` is the local operator's
  (`instrukt mcp -h`).
- The operator, or someone the operator names who knows the company, answers
  the intake.
- The operator can create a private repository for the company and grant
  access to it.

## Steps

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
   `~/.instrukt/companies/<name>/`, where `<name>` is the company's key. Run
   `instrukt config init --company <name>` in it: it writes the settings
   document and the ignore lines that keep the settings, the secrets and
   instrukt's local state out of the repository. Then run `instrukt config
   validate --company <name>`.
3. **Write the company's knowledge.** In the home, write `AGENTS.master.md`:
   what the company does, how its people work, who owns and who organizes it,
   and where its people find the agent. A longer part goes into a document in
   the home's `docs/` folder, named as a required read by its path from the
   master. The master and those documents reference only the company's own
   documents. Commit the knowledge in the home and push it to the company's
   repository; every agent that works for the company, wherever it checks the
   repository out, starts from the master it renders.
4. **Choose the agent's profiles, where the company needs its own.** When the
   company's agent is to run on other agent profiles than the computer's
   default mapping, set `profiles` in the home's settings to the ordered
   names the computer's agent configuration declares (`instrukt agents -h`);
   otherwise leave it out.
5. **Connect the messaging.** Under `connectors.<kind>` in the home's
   settings, declare the connector for the messaging provider the company's
   people use, with the settings that kind takes; the app or bot the kind
   needs is created in the company's workspace as instrukt specifies for that
   kind, and its tokens reach the home's secrets file only through
   `instrukt secrets set --company <name> <NAME>`, run by the operator
   (`sdlc/policy/security-awareness`, secrets never enter version control,
   logs or responses). Make the agent present where the people are to address
   it. The agent answers the people the provider knows as the company's own;
   no list of people is kept for that.
6. **Connect the services.** For each service from the intake that needs
   setting up for the company first, run that service's own procedure. Then
   load and run `general/procedure/company-service-connection` with
   `instrukt docs get general/procedure/company-service-connection --closure`.
   A person signs in to a service under the email address the messaging
   provider holds for them: a sign-in under another address is not kept.
7. **Install what the company demands.** Run `instrukt install`: a declared
   connector demands the runner service that hosts it, and a service declared
   with sign-in demands what the sign-in's return needs on this computer. Then
   `instrukt install --check` reports no unmet demand, and `instrukt events
   status` shows the connector connected. A connector declared while the
   runner runs is connected when the runner next starts.
8. **Prove that a person is answered.** With a person of the company present,
   have them address the agent where they talk, and read:
   - the answer arrives in the same conversation and nowhere else;
   - asked what it can do, the agent names the company's connected services
     and that it keeps the company's knowledge, and nothing else;
   - for each service declared with sign-in, a request that needs it brings
     the person a sign-in link in a direct message, and after they sign in the
     same turn answers;
   - for each service declared with a stored login, a request under that
     person's login is answered.

   An outcome that differs is corrected and the proof repeated before the
   company's people start (`general/policy/grounding`, confirmation is
   reserved for inspected claims).
9. **Report to the operator.** Give the home and its repository, the
   connector and its state, each service's outcome from
   `general/procedure/company-service-connection`, each outcome of step 8,
   and every step still open with what it waits on.

## Outputs

- The company's private repository, checked out as its home on this computer,
  with its settings validated and kept out of the repository.
- The company's knowledge, committed and pushed.
- The company's connector declared and connected, and its services declared.
- The proof of step 8 and the report of step 9.

## Recovery

- **A person's message is not answered.** `instrukt events status` names the
  cause: the connector's connection, the runner service, or the first message
  waiting while no published procedure gives the company's agent its role.
  Correct what it names and have the person address the agent again.
- **A person is turned away.** The messaging provider does not know them as
  one of the company's own, for instance as a guest; that is the company's to
  change in its workspace.
- **A sign-in is refused as not the person's.** The person signed in under
  another address than the messaging provider holds for them; they sign in
  again under that address.
- **A sign-in link does not arrive.** The failure the call names says whether
  the link is still undelivered or could not be delivered, with the
  provider's cause.
- **The knowledge is not rendered.** `instrukt config validate --company
  <name>` names the document and the reference that leaves the company's own
  documents; correct it and commit again.
