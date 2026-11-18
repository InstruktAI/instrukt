---
description: Connect the services a company works with for its agent — record them in the company's documents, establish from each service's own documentation whether and how its MCP server is reached, declare each one in the company's home so that its people sign in themselves, prove what can be proven as the company, and report what could not be connected and why.
activation_trigger: Use when the services a company works with are to be connected for its agent — while onboarding a company, when it starts working with another service, or when a connected service no longer answers as the company.
visibility: public
---

# Company Service Connection — Procedure

## Goal

After this procedure the company's agent reaches each service the company
works with that offers a remote MCP server, and the company's own documents say
which services its work runs on. You are the operator's agent setting the
company up; the company's agent uses what you declare.

You declare a service and prove the declaration. You do not collect a
person's login: where a service lets a person sign in themselves, instrukt
asks that person the first time a call of theirs needs it, and keeps and
renews the login.

Two kinds of material come out of it, and they stay apart. Which services the
company works with, and what for, is company knowledge: it goes into the
company's documents in its home, where its agent reads it. A server's address,
how a login is obtained and any login value are this computer's
configuration: they go into the home's settings document and secrets file,
which stay on the computer and out of the company's repository.

## Preconditions

- The company's home exists on this computer and `instrukt config validate
  --company <name>` reports no findings.
- You work on that computer, outside any session of the company: the
  `--company` path of `instrukt config`, `instrukt secrets` and `instrukt mcp`
  is the local operator's (`instrukt mcp -h`).
- Someone who knows which services the company's work runs on answers you,
  or the company's documents already say it.

## Steps

1. **Take the company's services.** Ask which services the company's work runs
   on and what each is used for, or read it from the company's documents. A
   service is what the company's people use by name; one service may be used
   for several things.
2. **Record them as company knowledge.** Write the services and what each is
   used for into the company's documents in its home — the master or a
   document it references — and commit the change in the home. Write no
   address, login or connection state there: the company's agent works from
   these documents and from what `instrukt mcp list` shows it
   (`general/procedure/company-assistance`).
3. **Establish each service's server from its own documentation.** For each
   service, read the service's own current documentation and establish:
   whether it offers a remote MCP server and its address; whether a person
   signs in to it in their browser, and if so whether a client must be
   registered with it beforehand and whether its sign-in names the person's
   email address; whether, without sign-in, a person can create a login value
   to hand over; and whether the company can hold a login of its own for work
   no person asks for. A directory, a third party's listing or your
   recollection is a lead to that documentation, never a substitute for it
   (`general/principle/grounding`, solution truth is grounded in current
   vendor documentation). A service that offers no server, or one that runs
   only as a local program, is not connectable; note why and go on.
4. **Declare the server for the company.** Read the settings with
   `instrukt config get --company <name> --revision`, then apply one patch with
   `instrukt config patch --company <name> --revision <rev>` that adds
   `mcp.<server>` with its `url` and a one-line `description` saying what the
   company uses the service for, and then by how a person's login is
   obtained:
   - **The person signs in.** Declare `speaker_login: sign-in`. Add
     `sign_in.client` when the service requires a client registered
     beforehand, and `sign_in.identity` when its sign-in does not name the
     person's email address and it documents a request that does. Add
     `sign_in.accept_return_without_issuer` only when a call or `instrukt
     config validate --company <name>` names it as missing and the operator,
     told that it lets a
     compromised authorization server capture a sign-in, accepts it
     (`general/policy/leadership`, exposure of what the human owns needs their
     authorization).
   - **The service offers no sign-in, only a value a person creates.**
     Declare `credential: <name>`, and add the reference
     `members.<member>.credentials.<name>` to the member entry of each person
     who uses the service, under the email address their messaging provider
     holds for them.
   - **No login at all.** `url` and `description` alone.

   Where the company holds a login of its own for work no person asks for,
   add `login` as a `${NAME}` reference, beside sign-in or a stored
   credential. The patch validates the whole document and names any field it
   refuses.
5. **Store the values the declaration references.** Each `${NAME}` the
   declaration and the member entries reference — the company's own login, a
   registered client, a person's created value — reaches the home's secrets
   file only through `instrukt secrets set --company <name> <NAME>`, run by
   the operator, with the value from your runtime's secret prompt by
   `--from-file` or typed into standard input in the operator's terminal. A
   person's created value comes to the operator from that person outside the
   conversation. Never ask for a value in the conversation, write it into a
   document or a command line, or print it (`sdlc/policy/security-awareness`,
   secrets never enter version control, logs or responses). A reference whose
   value is not there yet stays; note whose value is missing. For a
   registered client, give the operator the return address `instrukt mcp list
   --company <name>` shows, to register with the service.
6. **Validate the home.** Run `instrukt config validate --company <name>`. It
   also reports a settings document or secrets file the company's repository
   tracks.
7. **Prove what can be proven as the company.** Run `instrukt mcp list
   --company <name>`, then `instrukt mcp list-tools <server> --company <name>`
   for each server declared open or with the company's own login. A returned
   catalog proves the address and that login. A server whose calls carry only
   a person's login is proven by a person's first call, in the conversation
   where they address the company's agent; until then it is declared and
   unproven. Report a declaration as connected only on a returned catalog
   (`general/policy/grounding`, confirmation is reserved for inspected
   claims).
8. **Report to the operator.** Give each service with its outcome: connected,
   with the server's name and the number of tools listed; declared and
   awaiting a person's first call, with how that person's login is obtained;
   or not connectable, with why — no remote server, a server that runs only
   as a local program, or no way for a person or the company to log in. Name
   each value still missing and whose it is.

## Outputs

- The company's documents in its home, committed, naming the services the
  company works with and what for.
- A declaration in the home's settings document for each connectable service,
  and the values it references in the home's secrets file.
- The report of step 8.

## Recovery

- **The patch is refused for its revision.** Another writer changed the
  settings: read them again and make the patch against the new revision.
- **`--company` is refused.** You are inside a company's session; run the
  command as the operator outside any session.
- **`list-tools` cannot reach the server.** Hold the address against the
  service's documentation again and correct the declaration; a server still
  unreachable is reported unproven with the failure as given.
- **A call is refused naming a missing value or client.** The refusal names
  the reference or what the service requires; have the operator store it
  (step 5), or correct the declaration from the service's documentation, and
  list again.
- **The service refuses a login.** The value is wrong, expired or lacks
  rights at the service; the operator stores a new one (step 5). What the
  login may do is the service's to grant.
- **A service no longer answers as the company.** Run steps 3 to 7 for that
  service: its documentation may name a new address or way to log in.
