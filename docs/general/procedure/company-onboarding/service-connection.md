---
description: Connect or manage the MCP services a company works with — usable both by the platform operator during onboarding and by a verified company administrator in a direct message; discover remote endpoints via the Official MCP Registry and vendor docs, probe OAuth/EMA capabilities live, inherit host-level shared OAuth clients or enable zero-click per-company OIDC issuer auth where supported, verify local ingress reachability, prove each declaration, and record the services in company knowledge.
visibility: public
---

# Company Service Connection — Procedure

## Required reads

- [general/procedure/company-onboarding/overview](overview.md)

## Goal

After this procedure the company's agent reaches each service the company
works with that offers a remote MCP server, and the company's own documents say
which services its work runs on.

This procedure serves two callers without duplicating playbooks:

- **The platform operator's agent** (on the host, outside a company-agent
  session) during initial company onboarding or operator maintenance, with full
  access to `--company <name>`, global `mcp_clients`, `instrukt secrets`, and
  `instrukt mcp issuer init`.
- **The company's agent answering a verified workspace administrator in a
  direct message** (`admin=true` and `direct=true` in the platform-framed
  member envelope), scoped strictly to that company's `mcp` declarations,
  `instrukt mcp discover`, `instrukt mcp check-ingress`, `instrukt mcp clients`
  (read-only names/client IDs), and `instrukt mcp issuer show`. It never reads
  or writes secrets, global configuration, connectors, members, or other
  companies.

Where the service lets a person sign in themselves (interactive OAuth or
zero-click built-in OIDC issuer `jwt-bearer`), collect no login of theirs.

Which services the company works with, and what for, is company knowledge and
goes into the company's documents in its home. A server's address, how a login
is obtained and any login value are this computer's configuration and go into
the home's settings document and secrets file, never into the company's
repository.

## Preconditions

- The company's home exists on this computer.
- Either:
  - you are the platform operator's agent with the operator at hand to run
    host-level or secret commands outside any session; or
  - you are the company's agent in a direct-message turn whose platform-framed
    member envelope carries `admin=true` and `direct=true`.
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
3. **Discover each service's remote MCP server and auth mode.**
   - Use the Official MCP Registry (`https://registry.modelcontextprotocol.io/v0.1/servers`)
     and the service's own current vendor documentation to find its official
     remote Streamable HTTP MCP URL (`general/principle/grounding`, solution
     truth is grounded in current vendor documentation; the registry is a
     discovery lead, never a substitute for vendor docs). A service with no
     remote MCP server is not connectable directly; note why and go on.
   - Run `instrukt mcp discover <url> --json` against the remote URL to probe
     RFC 9728 / RFC 8414 / OIDC metadata live. Classify the server from the
     probe output:
     - **Zero-click EMA (`ema_jwt_bearer_supported: true`):** supports
       `sign_in.mode: issuer` using the company's built-in OIDC issuer so
       employees need zero interactive sign-in clicks once the admin registers
       the company's issuer URL at the service.
     - **Bucket A (`registration_supported: true`):** supports Dynamic Client
       Registration — declare `speaker_login: sign-in` with no client entry.
     - **Bucket B (`registration_supported: false`):** requires a
       pre-registered OAuth client. Run `instrukt mcp clients --json` to check
       whether this host already declares a shared global client
       (`mcp_clients.<name>`) for that vendor (such as Google or Microsoft).
       - If a matching host client exists, reference it with
         `sign_in.client_ref: <name>` — the company inherits the host's
         verified OAuth client in one line with no vendor app registration.
       - If no matching host client exists: as the platform operator, register
         the host client once in global `mcp_clients.<name>` +
         `~/.instrukt/secrets.env`, register its single shared return address
         from `instrukt mcp clients` at the vendor (for Google Workspace /
         Drive, enable *"Use this client for an AI-powered agent"* in Google
         Auth Platform), and then reference it via `sign_in.client_ref`. As a
         company agent in an admin DM, do not invent secrets or touch global
         config — tell the admin cleanly that the platform operator must
         register that vendor's host OAuth client once before it can be
         attached.
   - Note any additional requirement reported by `discover` (for example,
     missing `iss_parameter_supported` requiring operator acceptance of
     `sign_in.accept_return_without_issuer: true`, or non-OpenID Provider
     requiring `sign_in.identity`).
4. **Verify local callback reachability.** For any server using
   `speaker_login: sign-in`, run `instrukt mcp check-ingress --json`. This
   validates local `ingress.url` and cluster Redis and proves end-to-end
   external callback reachability through the real sign-in return path without
   inspecting any other computer's configuration. If it fails, report the
   local-boundary failure (operator resolves it on the host; in a company-admin
   DM, report cleanly that service sign-in infrastructure is currently
   unavailable without exposing internal host details).
5. **Read what a server declaration takes and current settings.** Run
   `instrukt config schema --company <name> mcp` (and `members` for the
   platform operator when a stored login is needed), then
   `instrukt config get --company <name> mcp` and
   `instrukt config get --company <name> --revision`.
6. **Declare the server.** Choose how a person's login is obtained, in this
   order:
   1. `speaker_login: sign-in` with `sign_in.mode: issuer` when the server
      supports EMA (`jwt-bearer`) and the company's built-in issuer is enabled
      (platform operator runs `instrukt mcp issuer init --company <name>` once
      if not yet enabled; `instrukt mcp issuer show` prints the issuer URL for
      the admin to register at the service).
   2. `speaker_login: sign-in` (`authorization-code`) — with no client for
      Bucket A (DCR), or `sign_in.client_ref` (preferred) / `sign_in.client`
      for Bucket B.
   3. `speaker_login: stored` only for a service without OAuth sign-in
      (platform operator only).
   4. Open server for a server that needs no login.

   Where the server is already declared under another choice, the patch also
   removes the keys the new choice does not take with a null. Apply with
   `instrukt config patch --company <name> --revision <rev>`. A sign-in that
   requires accepting an exposure (`accept_return_without_issuer: true`) is
   declared only once the operator/admin, told what the exposure is, accepts it
   (`general/policy/leadership`).
7. **Complete one-time vendor admin approval and any host/company secrets.**
   - **Host/company secrets (platform operator only):** store values only via
     `instrukt secrets set` (`--global` for host `mcp_clients`, `--company
     <name>` for company-only secrets) from `--from-file` or redirected stdin;
     never ask for secrets in conversation, commit them, or print them
     (`sdlc/policy/security-awareness`).
   - **Google Workspace / Google Drive:** give the company's Google Workspace
     administrator the non-secret Google OAuth Client ID (from
     `instrukt mcp list` / `instrukt mcp clients`) and direct them to mark that
     Client ID as **Trusted** in Google Workspace Admin Console
     (`Security > Access and data control > API controls > App access control`)
     so employees do not hit unverified-app or policy blocks on their one-time
     sign-in.
   - **EMA (`sign_in.mode: issuer`) services:** give the company administrator
     the issuer URL from `instrukt mcp issuer show` to register once as a
     trusted issuer in the service's admin console.
8. **Validate the home.** Run `instrukt config validate --company <name>`.
   Validation checks only this computer's own configuration and secrets; it
   never inspects another computer.
9. **Prove what can be proven.** Run `instrukt mcp list --company <name>` (or
   `instrukt mcp list` in a company-agent session) and `instrukt mcp list-tools
   <server>` where a company login or open access exists. A returned catalog
   proves the address and non-speaker login. A server whose calls carry only a
   speaker's login is proven by that person's first request to the company's
   agent (interactive one-time sign-in link in DM for `authorization-code`, or
   silent zero-click token exchange for `issuer`). Call a declaration connected
   only on a returned catalog or proven tool call
   (`general/policy/grounding`).
10. **Report.** Give each service with its outcome: connected, with the
    server's name and the number of tools listed; declared and awaiting a
    person's first request (or awaiting one-time vendor admin trust/issuer
    registration), with how that person's login is obtained; declared and
    unproven, with the failure as given; or not connectable, with why. In a
    company-admin DM, keep the reply in the company's words and never disclose
    host internals or secrets.

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
