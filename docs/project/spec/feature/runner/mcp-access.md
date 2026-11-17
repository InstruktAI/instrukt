---
description: MCP access — agents reach declared MCP servers through `instrukt mcp list|list-tools|call`; a server is declared machine-wide or in a company home, its tools load only when listed, and a call carries the speaker's login, stored or signed in once in the person's own browser, or the company's own when nobody is asking.
---
# MCP Access — Spec

## What it is

An agent reaches an MCP server through `instrukt mcp`, never through its
agent program's own settings. A server is declared once, for the machine or
for one company. Listing servers costs one line each; a server's tools enter
the agent's context only when it lists them. A call carries the right login by
code: the person being answered, or the company itself when nobody is asking.
A person's login is either a value stored for them or one they sign in for
once, in their own browser, when a call first needs it; instrukt keeps a
signed-in login and renews it itself for as long as the server lets it.

## Canonical fields

### Declaration

```yaml
mcp:
  erpnext:
    url: https://erp.example.com/api/v2/mcp
    description: Operate the company's ERPNext records
    credential: erpnext
    login: ${ERPNEXT_AGENT_LOGIN}
  books:
    url: https://books.example.com/mcp
    description: Read and keep the company's books
    speaker_login: sign-in
    login: ${BOOKS_AGENT_LOGIN}
    sign_in:
      scopes: [all]
      identity:
        url: https://books.example.com/api/method/frappe.auth.get_logged_user
        email_field: message
```

| Field | Contract |
| --- | --- |
| `mcp.<name>` | The server's name, following the grammar of a project slug. Declared in the global layer for the computer, or in a company home for work done for that company. |
| `url` | The server's network address. Required. It is an `https` address, or an `http` address on this computer's loopback; any other address is a validation finding, because a login would cross the network unprotected. |
| `description` | One line saying what the server is for; `list` shows it. Required. A description that spans lines is a validation finding. |
| `speaker_login` | Company home only: how a speaker's login is obtained. `stored`, the default: the value stored for the speaker's member entry under `credential`. `sign-in`: the speaker signs in to the server once, and instrukt keeps that login (see Sign-in). |
| `credential` | Company home only, with `speaker_login: stored`: the name under `members.<member>.credentials` whose value a speaker's call carries (`project/instrukt/spec/feature/configuration/company-homes`). |
| `login` | A `${NAME}` reference, never a literal value. In a company home it is the company's own login, references that home's secrets file, requires `credential` or `speaker_login: sign-in`, and is carried only by a call with no speaker; it is a stored value whatever `speaker_login` says. In the global layer it is the machine's own login and references the machine's secrets file. |
| `sign_in.scopes` | With `speaker_login: sign-in`, optional: the scopes a sign-in asks for. Without it the scopes follow the MCP authorization specification's scope selection: those the server names when it refuses a call without a login, otherwise those its resource metadata lists, otherwise none. |
| `sign_in.client.id`, `sign_in.client.secret` | With `speaker_login: sign-in`, optional: references into the company home's secrets file to a client registered with the server beforehand; `secret` only for a client that has one. Without them instrukt registers its own client with the server. |
| `sign_in.identity.url`, `sign_in.identity.email_field` | With `speaker_login: sign-in`, optional, together: a request that answers who signed in, for a server whose standard identity endpoint does not. instrukt sends a `GET` to `url` with the new login and reads the email address from `email_field`, a dot-separated path into the JSON answer. |

A declaration that carries none of `credential`, `login` and `speaker_login:
sign-in` is an open server: its calls carry no login. A company home's
declaration takes the place of a machine-wide declaration of the same name for
that company's work.

A third kind of speaker login, one identity the company holds acting at the
server for a person it names, is not specified; `speaker_login` takes no other
value.

These are validation findings naming the server and the field: an unknown
field; a literal `login` or client value; `speaker_login`, `credential` or
`sign_in` in the global layer; `speaker_login: stored` without `credential`;
`speaker_login: sign-in` together with `credential`; `sign_in` without
`speaker_login: sign-in`; one of the two `sign_in.identity` fields without the
other; a `login` without `credential` or `speaker_login: sign-in` in a company
home; and a missing `url` or `description`.

### Whose declarations a call sees

The command takes its caller from the selected runtime's caller identity, as
`instrukt discuss` does
(`project/instrukt/spec/feature/runner/conversation-links`), and reads the
runner's recorded assignment for that session.

| Caller | Declarations |
| --- | --- |
| A session whose recorded assignment names a company home | That company's, and the machine-wide open servers it does not replace. A machine-wide server that carries a login is offered there only when the company home declares that server itself. `--company` naming another company is refused. |
| The local operator, outside any session, with `--company <name>` | The same as that company's sessions see. |
| Any other caller | Every machine-wide declaration. |

A recognized session working in a company home is not served as any other
caller while its assignment is not recorded: `list-tools` and `call` are
refused there, before any secret is read or any server contacted, naming the
company and the missing binding, until the runner has recorded the session as
that company's. A session without an assignment outside company homes, and
the local operator outside any session, are served as the table says. The
working directory only withholds access; it never gives a session a company,
a role or a login.

### The login a call carries

The speaker is the person the runner recorded as the sender of the addressed
message that started the calling session's current turn
(`project/instrukt/spec/feature/runner/conversations`). The current turn is the
one the session has in progress when the command runs: a command an agent
leaves running beyond its turn is judged by the turn then in progress, and is
refused when the session has none or the runtime cannot say. A session with no
such record has no speaker. No argument and no value the agent passes selects
a speaker or a member.

A speaker's stored logins are those of the one member entry of that company
(`project/instrukt/spec/feature/configuration/company-homes`) whose email
address equals the address recorded for the speaker with that message,
compared without regard to case. The match never compares names and never
consults another company. A speaker recorded without an address, or with an
address no member lists, has no member entry. A speaker's signed-in login is
kept for the company, the speaker's address and the server; no member entry is
needed for it.

| Declaration | Speaker | Login |
| --- | --- | --- |
| Open, machine-wide or a company's | Any, or none | None. |
| A company's, `stored` | A speaker whose member entry holds that credential | That member's credential. |
| A company's, `stored` | A speaker with no member entry, or whose entry lacks it | The call is refused naming the speaker and the missing credential. |
| A company's, `sign-in` | A speaker with a login for the server | That login, renewed first when it has run out. |
| A company's, `sign-in` | A speaker with no login for the server | A sign-in starts, and the call waits for it. |
| A company's, `sign-in` | A speaker recorded without an email address | The call is refused naming the speaker and the missing address. |
| A company's, with `login` | None | The company's `login`. |
| A company's, without `login` | None | The call is refused naming the server and the missing company login. |
| Machine-wide, with `login` | None: it is offered to no session that has a speaker | The machine's `login`. |

A call that has a speaker never carries a company's or the machine's login. A
stored value is sent as the request's `Authorization` header; a signed-in login
is sent as `Authorization: Bearer <access token>`.

### Sign-in

A sign-in follows the MCP authorization specification: the authorization code
flow with a code challenge (PKCE, `S256`), resource indicators, and discovery
through the server's protected resource metadata and its authorization
server's metadata.

#### When a sign-in starts

A call with a speaker to a server declared with `speaker_login: sign-in`, for
which the speaker has no login, starts a sign-in. instrukt records a pending
sign-in in the state database and the runner sends the speaker a one-time link
as a direct message through the connector the turn came from
(`project/instrukt/spec/feature/runner/conversations`). The link is never
posted in a channel and never passes through the agent. The call then waits
for the sign-in, at most 90 seconds:

| Outcome | The call |
| --- | --- |
| The sign-in completes while the call waits | Proceeds with the new login; the same turn answers. |
| The wait ends first | Ends with a failure saying that a sign-in to the server is pending and that a link was sent to the speaker by direct message. It carries no link, code or other secret value. |

When a sign-in completes after its call stopped waiting, the runner tells the
speaker by direct message that they are signed in and can ask again. No turn
is started for it. Whether a completed sign-in reaches a waiting call or a
direct message is decided once, in one transaction.

A company, person and server have at most one pending sign-in. A call that
finds one still open waits for it and sends no second link. A pending sign-in
that is not completed within its ten minutes expires, and the next call starts
a new one.

#### Discovery and the client

Before its first sign-in to a server, instrukt finds the server's
authorization server: from the `resource_metadata` the server names when it
refuses a call without a login, otherwise from the server's protected resource
metadata at its well-known addresses, then the authorization server's metadata
at its well-known addresses, OAuth's first and OpenID Connect's next. Metadata
whose `resource` or `issuer` differs from the address it was fetched for is
not used. An authorization server whose metadata does not list `S256` among
its code challenge methods is not used; a call that needs it is refused naming
the server.

The client instrukt signs in with is the one declared under `sign_in.client`
when there is one. Otherwise instrukt registers a client dynamically where the
authorization server offers registration, once per company home and server: a
public client (`token_endpoint_auth_method: none`) whose one redirect address
is the sign-in return, for the authorization code and refresh token grants,
named `instrukt <company>`. The registration is kept in the state database and
made again when the return address changes or the authorization server no
longer knows the client. A server that offers no registration, declared
without `sign_in.client`, refuses the call naming the server and the missing
client.

#### The link

The link is the authorization server's authorization endpoint with:
`response_type=code`, the client, the redirect address, the scopes, a state,
a code challenge with `code_challenge_method=S256`, and `resource`, the
server's `url`. The state and the code verifier are random values of at least
256 bits, made for this one sign-in. The state is bound to the company, the
speaker's address, the server and the message whose call started the sign-in;
the database keeps its SHA-256 digest, not the state itself. A state is
usable once and for ten minutes.

#### The return

The authorization server sends the person's browser back to the sign-in
return: `<ingress.url>/hooks/inbound/sign-in` on the ingress computer's
receiver (`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`).
It is an unsigned `GET` carrying `state` and either `code` or the
authorization server's `error`, and `iss` where the server sends one. It is
not an event: nothing is published to the event stream and nothing can
subscribe to it.

The receiver writes one entry to cluster Redis, keyed by the SHA-256 digest
of the state and carrying the code or the error and the `iss` value, only when
no entry holds that key, and expiring after ten minutes unclaimed. It does not
know which sign-ins are pending. The browser is shown only that the return was
received and that the person can go back to the conversation; a request
without a `state`, or without a `code` or an `error`, is answered `400`.

The runner of the computer that issued the state claims its entry while the
sign-in is open: it reads and removes it in one operation, so an entry is
claimed once. The waiting call learns the outcome from the pending sign-in in
the state database. The code verifier, the speaker and the conversation never
leave that computer.

#### Who signed in

The issuing computer refuses a return whose `iss` differs from the
authorization server's issuer, and a return without `iss` from an
authorization server whose metadata says it sends one. Otherwise it exchanges
the code at the token endpoint with the code verifier, the redirect address,
the client and `resource`. It then asks who signed in:

| Declaration | Asked |
| --- | --- |
| With `sign_in.identity` | The declared request, with the new login; the address is the string at `email_field`. |
| Without it | The authorization server's OpenID Connect UserInfo endpoint, named in its metadata; the address is its `email`. A sign-in for such a server asks for the scopes `openid` and `email` as well. |

The login is kept only when that address equals the address stored with the
speaker's message, exactly and without regard to case. A login for another
address, or for no address, is discarded, revoked at the authorization server
where it offers revocation, and the speaker is told by direct message. An
`error` from the authorization server, or a refused exchange, ends the sign-in
and the speaker is told.

#### Keeping and renewing a login

A login is kept in the state database for the company, the person's address
and the server: the access token, when it runs out, the refresh token, the
scopes, and the client it was issued to. It is never written to the company
home. No value of it is shown to the agent: the output rule below removes it
from everything a server returns.

instrukt renews a login itself with its refresh token, before a call when the
access token runs out within a minute, and once after the server refuses a
call's access token. One renewal of a login runs at a time; a call that finds
one under way waits for its result. The renewed values are stored in one
transaction. When the authorization server issues a new refresh token, the
previous one is revoked after the new one is stored, where the authorization
server offers revocation. A renewal the authorization server refuses
(`invalid_grant`) removes the login, and the call continues as a call with no
login: it starts a sign-in. A renewal that fails otherwise fails the call
naming the server and keeps the login.

#### Ending a login

| Who | Does |
| --- | --- |
| The person | Asks the agent to be signed out of a server, or of all; the agent runs `instrukt mcp sign-out [<name>]`, which acts for the speaker of its turn only. |
| The local operator | Removes a person's logins for a company with `instrukt mcp sign-out --company <company> --email <address> [<name>]`. |
| The authorization server | Refuses a renewal. |

Removing a login revokes its refresh token at the authorization server where
it offers revocation, and removes it from the state database whether or not
revocation succeeds. The next call of that person to that server starts a new
sign-in.

#### Direct messages

The runner sends these to the speaker, `<name>` being the server's name:

| When | Text |
| --- | --- |
| A sign-in starts | `To let the agent use <name> as you, sign in here. The link works once, for ten minutes: <link>` |
| A sign-in completes after its call stopped waiting | `You are signed in to <name>. Ask the agent again to continue.` |
| The login was for another address | `That sign-in to <name> was not for your address and was not kept.` |
| The authorization server refused the sign-in | `The sign-in to <name> did not complete: <the server's error code>.` |

### What the caller identity establishes

The caller identity is the runtime's statement of which session a process
belongs to, and instrukt does not authenticate a caller beyond it. Every
process of the same operating-system user is trusted: a caller presenting
another session's identity is served as that session, and that user can read a
company home's files, its secrets file among them. Closing that takes
confining the agent, which this contract does not provide
(`project/instrukt/spec/feature/runner/conversations`). The `--company` path
is the local operator's.

### Commands

| Command | Does |
| --- | --- |
| `instrukt mcp list` | Names each server the caller sees with its description and where it is declared. Contacts no server and reads no secret. |
| `instrukt mcp list-tools <name>` | Returns that server's tool catalog: each tool's name, description and input shape. |
| `instrukt mcp call <name> <tool> --json '<arguments>'` | Calls the tool with the given JSON object and prints the tool's result as JSON. |
| `instrukt mcp sign-out [<name>]` | Removes the speaker's signed-in login for that server, or for every server of the company. Refused in a session with no speaker. |
| `instrukt mcp sign-out --company <company> --email <address> [<name>]` | The local operator's: removes that person's signed-in logins for the company, for one server or all. |

- `list` and `list-tools` accept `--json`.
- `list-tools` carries the login a call to that server would carry and is
  refused as that call would be; it has no identity of its own.
- `list-tools` and `call` each open their own connection, perform the request
  and close it. instrukt speaks the Streamable HTTP transport of MCP version
  2025-06-18. A reply that is not a well-formed answer to the request it was
  sent, or that lacks what the method requires, is a failure, never an empty
  result. An answer that arrives as an event completes the request without
  waiting for the stream to end.
- Opening a connection, each request, the size of a reply and the number of
  pages of a catalog each have a limit; the limits are the client's own, in
  `crates/instrukt-runner/src/mcp/_client.rs`. A request that outlasts its
  limit, a reply that exceeds its size and a catalog that repeats a cursor or
  exceeds its pages each end as a failure.
- No connection outlives the command. When the server opened a session,
  instrukt asks it once to end that session on every path, a failed one
  included; a server that does not end it, or answers that it cannot, is logged
  and changes no outcome.
- An unknown server name is refused naming the servers the caller sees. An
  unreachable server fails naming the server and its address. A failure the
  server reports reaches the caller with the server's cause and a non-zero
  exit.
- Before anything a server returned is printed or logged, a result, a catalog,
  a failure or a part of the server's reply that the transport reports, every
  occurrence of a resolved credential value and of a value of a signed-in
  login is removed from it, in text and in JSON strings and keys, also where
  JSON escaped it. The command's own output, logs and diagnostics name
  references and servers, never values.
- The command writes no agent program's settings.

## Use cases

### UC-MA1: An agent uses a tool as the person who asked

```gherkin
Given a company member asks the company's agent for work that needs a server the company declares as needing a login
When the agent calls a tool of that server
Then the call reaches the server under that member's own login
And the agent supplied no credential
But when that member has no login for the server
Then the call is refused naming the member and the missing login
And no other login is used
```

### UC-MA2: Run a company's unattended work under the company's own login

```gherkin
Given a developer has a company's work started by events, with nobody asking
When that work needs a server the company declares as needing a login
Then the server is reached under the company's own login
But when the company has declared no login of its own for that server
Then the call is refused naming the server and the missing login
```

### UC-MA3: An agent learns a server's tools only when it needs them

```gherkin
Given a developer has declared servers for the machine and for a company
When an agent working for that company lists the servers
Then it sees each server's name and purpose, with no tool definitions and without contacting a server
When it lists one server's tools
Then it receives that server's tool catalog
```

### UC-MA4: Each company reaches its own server under one name

```gherkin
Given a developer has two companies that each declare a server under the same name at their own address
When each company's agent calls a tool of that server
Then each call reaches its own company's address
```

### UC-MA5: Sign in once while the agent waits and be answered in the same turn

```gherkin
Given a company member asks the agent for work that needs a server the company declares with sign-in
And the member has never signed in to that server
When the agent calls a tool of that server
Then the member receives a sign-in link in a direct message, and nowhere else
When the member signs in with their own address while the call waits
Then the call reaches the server under the member's own login
And the same turn answers
```

### UC-MA6: Finish a late sign-in and ask again

```gherkin
Given a member has received a sign-in link and has not used it
When the agent's call stops waiting
Then the agent learns that a sign-in is pending and that a link was sent, and nothing secret
When the member then signs in
Then the member is told in a direct message to ask again
And no turn starts until they do
```

### UC-MA7: Refuse a sign-in as someone else

```gherkin
Given a member has received a sign-in link
When they sign in as a different person
Then that login is not kept
And the member is told that the sign-in was not for their address
And the agent's next call for them starts a new sign-in
```

### UC-MA8: Keep working without signing in again

```gherkin
Given a member has signed in to a server
When their login runs out and the agent calls a tool for them
Then instrukt renews the login and the call reaches the server
But when the server refuses to renew it
Then the login is removed and a new sign-in starts
```

### UC-MA9: Sign out of a server

```gherkin
Given a member has signed in to a server
When they ask the agent to sign them out of it
Then their login is removed and revoked at the server
And the agent's next call for them starts a new sign-in
```

### UC-MA10: Remove a person's logins as an operator

```gherkin
Given a person has signed in to a company's servers
When the operator removes that person's logins for the company
Then none of them is kept on the computer
And a call for that person starts a new sign-in
```

## Known caveats

- Only servers reached over the network are supported. A server that runs as a
  local program, or that keeps state between calls, is not.
- A credential value a server returns changed, encoded or in pieces is not
  recognised and is not removed.
- Refusing a session in a company home until it is bound holds for a session
  that stays where the runner started it; it is not confinement.
- A speaker's login is a stored value or a sign-in. One identity the company
  holds acting for a person it names is not supported.
- A sign-in needs the person to open the link in a browser that reaches the
  authorization server. A server may ask the person to approve every sign-in.
- A call that starts a sign-in holds the agent's turn for up to 90 seconds.
- A sign-in's return reaches the issuing computer only through cluster Redis:
  a computer that cannot reach it completes no sign-in, and a return arriving
  while it cannot be reached is lost; the person signs in again.
- Clients are registered beforehand or dynamically. A client identified by a
  metadata document of its own is not supported, nor is an authorization
  server without `S256`.
- A person who can no longer reach the agent keeps their logins in the state
  database until they sign out, the operator removes them, or the
  authorization server refuses a renewal.
- Logins lie in the state database, where any process of the same
  operating-system user can read them. With a person's own login the agent
  holds that person's rights at the server, so text of other people that
  accompanies the speaker's message can steer what happens under the
  speaker's name.
- The agent program's own MCP settings are untouched and unknown to instrukt:
  a server configured there is reached by that program, without these rules.
- A speaker's logins are found by the email address the connector's provider
  holds for them. Two accounts the provider gives one address reach the same
  member entry.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/conversations](conversations.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](single-ingress-webhook-termination.md) — the receiver that serves the sign-in return.
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md) — where logins, clients and pending sign-ins are kept.
- https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization — MCP authorization: protected resource metadata, authorization server discovery order, client registration, PKCE with `S256` required, `resource` in authorization and token requests, scope selection, the bearer header.
- https://www.rfc-editor.org/rfc/rfc6749 — OAuth 2.0: the authorization code grant, `state`, the refresh token grant, `invalid_grant`, and codes that live at most ten minutes.
- https://www.rfc-editor.org/rfc/rfc7636 — PKCE: a verifier of 43 to 128 characters with at least 256 bits of entropy, and `S256`.
- https://www.rfc-editor.org/rfc/rfc8707 — resource indicators: the `resource` parameter.
- https://www.rfc-editor.org/rfc/rfc9728 — protected resource metadata: `resource`, `authorization_servers`, `scopes_supported`, and that `resource` must equal the address fetched.
- https://www.rfc-editor.org/rfc/rfc8414 — authorization server metadata: `issuer`, `registration_endpoint`, `revocation_endpoint`, `code_challenge_methods_supported`, and that `issuer` must equal the address fetched.
- https://www.rfc-editor.org/rfc/rfc7591 — dynamic client registration: `redirect_uris`, `token_endpoint_auth_method: none` for a public client, registration without an initial token.
- https://www.rfc-editor.org/rfc/rfc7009 — token revocation: `token` and `token_type_hint`; success also for an unknown token.
- https://www.rfc-editor.org/rfc/rfc9207 — the `iss` authorization response parameter and when a response without it is refused.
- https://openid.net/specs/openid-connect-core-1_0.html — the UserInfo endpoint, the `openid` and `email` scopes and the `email` claim.
