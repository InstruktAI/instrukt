---
description: Connect the services a company works with for its agent — record them in the company's documents, establish from each service's own documentation whether and how its MCP server is reached, declare each one in the company's home with a person's own sign-in wherever the service offers it, prove what can be proven as the company, and report what could not be connected and why.
visibility: public
---

# Company Service Connection — Procedure

## Required reads

- [general/procedure/company-onboarding/overview](overview.md)

## Goal

After this procedure the company's agent reaches each service the company
works with that offers a remote MCP server, and the company's own documents say
which services its work runs on. You are the operator's agent setting the
company up; the company's agent uses what you declare. You declare a service
and prove the declaration; where the service lets a person sign in
themselves, you collect no login of theirs.

Which services the company works with, and what for, is company knowledge and
goes into the company's documents in its home. A server's address, how a login
is obtained and any login value are this computer's configuration and go into
the home's settings document and secrets file, never into the company's
repository.

## Preconditions

- The company's home exists on this computer.
- The operator is at hand to run, at their own terminal, the commands this
  procedure gives them, and to give you their output back.
- Someone who knows which services the company's work runs on answers you, or
  the company's documents already say it.

## Steps

1. **Take the company's services.** Ask which services the company's work runs
   on and what each is used for, or read it from the company's documents.
2. **Record them as company knowledge.** Write the services and what each is
   used for into documents under `docs/` in a separate worktree (leaving the
   serving company home checkout on the reviewed default branch), following the
   authoring and review rules of `general/procedure/company-onboarding/overview`,
   and commit the change in that worktree. Write no address, login or
   connection state into them: the company's agent works from these documents
   and from what `instrukt mcp list` shows it
   (`general/procedure/company-assistance`).
3. **Establish each service's server from its own documentation.** For each
   service, read the service's own current documentation of its MCP server and
   its authorization: whether it offers a remote server and its address;
   whether a person signs in to it themselves, and what that sign-in requires
   of a client; whether, without such a sign-in, a person can create a login
   value to hand over; and whether the company can hold a login of its own for
   work no person asks for. A directory, a third party's listing or your
   recollection is a lead to that documentation, never a substitute for it
   (`general/principle/grounding`, solution truth is grounded in current
   vendor documentation). A service with no remote server is not connectable;
   note why and go on.
4. **Read what a server declaration takes.** Run `instrukt config schema
   --company <name> mcp`, and `instrukt config schema --company <name>
   members` when a person's stored login will be needed. They show each field
   with its contract; derive the declaration from them.
5. **Read the current settings.** Run `instrukt config get --company <name>
   mcp` and `instrukt config get --company <name> members` for what is
   declared now, and `instrukt config get --company <name> --revision` for
   the revision they belong to.
6. **Declare the server.** Choose how a person's login is obtained, in this
   order: the person's own sign-in wherever the service offers it; a value the
   person creates, only for a service without such a sign-in; no person's
   login for a server that needs none. Add the company's own login wherever
   the service gives the company one. Fill the fields the schema of step 4
   gives for that choice, from the service's documentation. Where the server
   is already declared under another choice, the patch also removes the keys
   the new choice does not take, with a null. Apply it with `instrukt config
   patch --company <name> --revision <rev>`. A sign-in that the operator must
   accept an exposure for, as its contract shows, is declared only once the
   operator, told what the exposure is, accepts it
   (`general/policy/leadership`, exposure of what the human owns needs their
   authorization).
7. **Store the values the declaration references.** A value — the company's
   own login, a registered client's secret, a person's created value — reaches
   the home's secrets file only through `instrukt secrets set --company <name>
   <NAME>`, from a file with `--from-file` or from redirected standard input:
   your runtime's secret prompt writes the value to a file you pass, or the
   operator runs the command with a file of their own. A person's created
   value comes to the operator from that person outside the conversation.
   Never ask for a value in the conversation, write it into a document or a
   command line, or print it (`sdlc/policy/security-awareness`, secrets never
   enter version control, logs or responses). A reference whose value is not
   there yet stays; note whose value is missing.
8. **Validate the home.** Run `instrukt config validate --company <name>`.
9. **Prove what can be proven as the company.** Give the operator, to run at
   their own terminal outside any session, `instrukt mcp list --company
   <name>` and then `instrukt mcp list-tools <server> --company <name>` for
   each server you declared, and read back what they return; neither output
   carries a secret. `mcp list` also shows the return address to register with
   a service whose client is registered beforehand: give it to the operator
   for that service. A returned catalog proves the address and the login a
   call with no person carries. A server whose calls carry only a person's
   login is proven by that person's first request to the company's agent.
   Call a declaration connected only on a returned catalog
   (`general/policy/grounding`, confirmation is reserved for inspected
   claims).
10. **Report.** Give each service with its outcome: connected, with the
    server's name and the number of tools listed; declared and awaiting a
    person's first request, with how that person's login is obtained;
    declared and unproven, with the failure as given; or not connectable,
    with why. Name each value still missing and whose it is.

## Outputs

- The company's documents in a separate worktree, with a commit naming the
  services the company works with and what for, ready for pull request, review,
  and integration.
- A declaration in the home's settings document for each connectable service,
  and the values it references in the home's secrets file.
- The report of step 10.

## Recovery

- **The patch is refused for its revision.** Another writer changed the
  settings: run step 5 again and make the patch from what it returns.
- **`list-tools` cannot reach the server.** Hold the address against the
  service's documentation again and correct the declaration; a server still
  unreachable is reported unproven with the failure as given.
- **A call is refused naming a missing value, client or acceptance.** Store
  the value (step 7), correct the declaration from the schema and the
  service's documentation (steps 4 to 6), or put the acceptance to the
  operator (step 6), and have the operator list again.
- **The service refuses a stored login.** The value is wrong, expired or
  lacks rights at the service; its holder gives the operator a new one
  (step 7).
- **The service refuses a person's signed-in login.** Report the service's
  reason as given. A signed-in login is not a value you store: the person
  ends theirs with a sign-out in their conversation with the company's agent,
  or the operator ends it at their own terminal with `instrukt mcp sign-out
  --company <name> --email <address> <server>` (`instrukt mcp sign-out -h`),
  and the person's next request signs them in again. What a login may do is
  the service's to grant.
- **A service no longer answers as the company.** Run steps 3 to 9 for that
  service: its documentation may name a new address or way to log in.
