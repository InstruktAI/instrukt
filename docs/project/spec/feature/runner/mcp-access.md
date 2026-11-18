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
| `sign_in.client.id`, `sign_in.client.secret` | With `speaker_login: sign-in`, optional: references into the company home's secrets file to a client registered with the authorization server beforehand. `id` is required when `client` is declared; `secret` only for a client that has one. Without them instrukt registers its own client (see Registrations). |
| `sign_in.accept_return_without_issuer` | With `speaker_login: sign-in`, optional, `true` or `false` (the default): the operator's acceptance of a return that does not name its issuer, for a server whose authorization server does not announce that it names itself on the return (see The return path and the issuer). |
| `sign_in.identity.url`, `sign_in.identity.email_field` | With `speaker_login: sign-in`, optional, together: a request that answers who signed in, for a server whose authorization server is no OpenID Provider. `url` is an absolute `https` address without credentials or fragment, which the operator trusts to receive this server's tokens; instrukt sends a `GET` to it with the new access token and reads the email address from `email_field`, a dot-separated path into the JSON answer. |

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
`speaker_login: sign-in`; `sign_in.client` without `id`; an
`accept_return_without_issuer` that is not `true` or `false`; one of the two
`sign_in.identity` fields without the other; an identity `url` that is not an
absolute `https` address or carries credentials or a fragment; a `login`
without `credential` or `speaker_login: sign-in` in a company home; and a
missing `url` or `description`.

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
| A company's, `sign-in` | A speaker with a usable login for the server | That login, renewed first when it runs out. |
| A company's, `sign-in` | A speaker with no usable login for the server | A sign-in starts, and the call waits for it. |
| A company's, `sign-in` | A speaker recorded without an email address | The call is refused naming the speaker and the missing address. |
| A company's, with `login` | None | The company's `login`. |
| A company's, without `login` | None | The call is refused naming the server and the missing company login. |
| Machine-wide, with `login` | None: it is offered to no session that has a speaker | The machine's `login`. |

A call that has a speaker never carries a company's or the machine's login. A
stored value is sent as the request's `Authorization` header; a signed-in login
is sent as `Authorization: Bearer <access token>`.

A call under a speaker's signed-in login may do whatever that server permits
that person. instrukt adds no restriction of its own, by tool, by kind of call
or by where the message came from: what the person may do is decided by the
server's own permissions for the login they signed in with. The server's
answer, a refusal included, reaches the caller as it does for any call.

### Sign-in

A sign-in follows the MCP authorization specification: the authorization code
flow with a code challenge (PKCE, `S256`), resource indicators, and discovery
through the server's protected resource metadata and its authorization
server's metadata. Logins, registrations and pending sign-ins are kept in the
state database of the computer whose call started the sign-in; they are never
shared with another computer.

#### Destinations

instrukt sends a code, a token or a client secret only to an `https` address:
the declared `sign_in.identity.url`, and the authorization, token,
registration, revocation, UserInfo and key-set endpoints named in the metadata
of the authorization server it selected for the server. It follows no
redirect of a request that carries one of them; such a redirect fails the
request. It derives no other destination from what a server returns. A
declared or discovered address may be a private one.

#### Registrations

A registration is the client instrukt signs in with for a company home, a
server and an authorization server, together with its own return path:
`/hooks/inbound/sign-in/<return id>`, where `<return id>` is 32 lowercase
hexadecimal characters from 128 random bits. instrukt makes the return id when
it registers a client dynamically, and when it first reads a declared client
for that company home and server. The return address is `ingress.url`
followed by that path. Each registration is kept in the state database with its
issuer, client identifier, return id and return address, and for a declared
client the SHA-256 digest of its secret.

The client is the one declared under `sign_in.client` when there is one. It
authenticates at the token and revocation endpoints with its secret by HTTP
Basic authentication, or in the request body when the authorization server
lists only `client_secret_post`; a client declared without a secret sends only
its identifier. The operator registers the declared client's return address
with the authorization server; `instrukt mcp list --company <company>` shows
it. When `ingress.url` changes, the operator updates it there. When the
declared client's identifier or secret no longer matches its registration,
that registration is retired and a new one, with a new return id, is made for
the declared client as it now is. The logins and open sign-ins of the retired
registration become unusable: they are removed and cancelled without a
revocation request, and the next call starts a sign-in. A code or refresh
token issued to one client is never sent with another client's credentials.

Without a declared client instrukt registers one where the authorization
server offers registration: a public client (`token_endpoint_auth_method:
none`) whose one redirect address is the return address, for the authorization
code and refresh token grants, named `instrukt <company>`. The registration is
refused when the authorization server's answer states something that
contradicts what was asked: another redirect address, grant types without the
authorization code grant, or a client authentication method that needs a
secret while none was issued. What the answer leaves out is no refusal. A
refused registration refuses the call naming the server and the declared
client it needs; a server that offers no registration and has no declared
client refuses the call the same way. A server that issues no refresh token
gives logins that cannot be renewed (see Keeping and renewing a login). A
dynamic registration is made again when the return address changes or the
authorization server no longer knows the client; a new registration has a new
return id. A login and a pending sign-in keep the registration they began
with.

#### Discovery

Before its first sign-in to a server, instrukt finds the server's
authorization server: from the `resource_metadata` the server names when it
refuses a call without a login, otherwise from the server's protected resource
metadata at its well-known addresses. Resource metadata whose `resource`
differs from the server's `url` is not used. When it lists several
authorization servers, the first is selected and recorded with the
registration. The authorization server's metadata is read at its well-known
addresses, OAuth's first and OpenID Connect's next; metadata whose `issuer`
differs from the issuer identifier its address was built from is not used. An
authorization server whose metadata does not list `S256` among its code
challenge methods is not used; a call that needs it is refused naming the
server and the missing `S256`.

Who signed in is learned in one of two ways (see Who signed in). Without
`sign_in.identity`, the authorization server must be an OpenID Provider: its
OpenID Connect discovery document names a UserInfo endpoint and a key set.
When it is not, the call is refused naming the server and the missing
`sign_in.identity`.

#### When a sign-in starts

A `call` or `list-tools` with a speaker to a server declared with
`speaker_login: sign-in`, for which the speaker has no usable login, starts a
sign-in. `list-tools` waits and ends as a call does: what this section says of
a call holds for both. The call finds the authorization server and the
registration (see Discovery and Registrations), makes the link and, in one
transaction, records a pending sign-in in the state database with the link,
and itself as waiting on it. A company, person and server have at most one
pending sign-in, open or being exchanged; a call that finds one records itself
as waiting on that one, and no second link is made.

The runner sends the link to the speaker as a direct message through the
connector the turn came from
(`project/instrukt/spec/feature/runner/conversations`); the link is never
posted in a channel and never passes through the agent. Once the connector has
accepted it, the link is removed from the database. A failure the connector's
provider documents as final closes the sign-in as `undelivered`; another
failure is retried until the sign-in expires. After a restart the runner sends
the link of every open sign-in whose link was not accepted yet.

Every waiting call waits until the sign-in closes or its own 90 seconds have
passed, whichever comes first:

| The sign-in, when the call stops waiting | The call |
| --- | --- |
| `completed` | Proceeds with the new login; the same turn answers. |
| Still open, its link accepted | Fails saying that a sign-in to the server is pending and that a link was sent to the speaker by direct message. |
| Still open, its link not yet accepted | Fails saying that a sign-in to the server is pending and that its link has not been delivered yet. |
| Being exchanged | Fails saying that the sign-in to the server is being completed and that the person can ask again shortly; how it ends is decided as for a sign-in that completes while no call waits. |
| `undelivered` | Fails saying that the sign-in link could not be delivered, with the provider's cause. |
| `declined` | Fails saying that the authorization server did not complete the sign-in, with its error code. |
| `refused` | Fails saying that the sign-in was not for the speaker's address and was not kept. |
| `failed` | Fails saying that the sign-in failed, with its cause. |
| `expired` | Fails saying that the sign-in expired. |
| `cancelled` | Fails saying that the sign-in was cancelled by a sign-out. |

No result carries a link, code, state, verifier or token.

A sign-in is completed in one transaction that stores the login and decides
who learns of it: every call still waiting on it proceeds, and when no call is
waiting at that moment the runner tells the speaker by direct message that
they are signed in and can ask again. No turn is started for it. A pending
sign-in that is not completed within ten minutes of its making expires, and
the next call starts a new one.

#### The link

The link is the authorization server's authorization endpoint with:
`response_type=code`, the client, the registration's return address, the
scopes, a state, a code challenge with `code_challenge_method=S256`,
`resource` (the server's `url`), and, without `sign_in.identity`, a nonce. The
state, the nonce and the code verifier are each the base64url form of 256
random bits, made for this one sign-in; the code challenge is the base64url
form of the SHA-256 digest of the verifier. The state is bound to the company,
the speaker's address, the server, the registration and the message whose
call started the sign-in. The database keeps the SHA-256 digest of the state,
never the state itself apart from the link while it is undelivered. A state is
usable once, within the sign-in's ten minutes.

#### The return

The authorization server sends the person's browser to the registration's
return address. The ingress computer's receiver
(`project/instrukt/spec/feature/runner/single-ingress-webhook-termination`)
takes it without a signature and knows no company or pending sign-in. It
checks only the request:

- The method is `GET`; another method is answered `405`.
- The path is the sign-in path followed by one return id in its form;
  another path is not found.
- A return without a `state` belongs to no pending sign-in: it is received
  and discarded, nothing is written, and the browser is shown the same page as
  for any return. The pending sign-in it may have come from ends by its own
  lifetime, and the person is told nothing that the return did not establish.
- Otherwise the query is decoded once and carries one nonempty `state`, and
  exactly one of `code` and `error`, each once; `iss` at most once. Other
  parameters are ignored. A repeated, missing or undecodable parameter, or a
  request larger than the receiver takes, is answered `400`.

It then writes one entry to cluster Redis, keyed by the SHA-256 digest of the
state and carrying the return id, the code or the error code, and `iss` when
present. The entry is written only when no entry holds that key, and expires
after ten minutes unclaimed. At most 1,000 sign-in entries exist at once;
beyond that a return is answered `503` and nothing is written. The entry is no
event: nothing is published to the event stream and nothing can subscribe to
it.

The browser is shown a plain page saying that the return was received and
that the person can go back to the conversation. It does not say that a login
was accepted, loads nothing from elsewhere, and is sent with
`Cache-Control: no-store` and `Referrer-Policy: no-referrer`. The receiver,
the runner and their logs, status and recorded causes never hold a return's
query, code, state, verifier or token. The operator's edge filters abuse of
the sign-in path as it does for sources.

#### Claiming a return

The runner of the computer that issued a state reads only the entries of its
own open sign-ins. It claims an entry by reading and removing it in one
operation, and then, in one transaction, moves the sign-in from `open` to
`exchanging`, only when it is still open and unexpired; a sign-in that is no
longer open is never exchanged, and an entry made again for it expires
unread. A return whose return id is not the sign-in's registration's, or whose
`iss` differs from the issuer, is not accepted: the sign-in closes as `failed`.
Where the authorization server's metadata says it sends `iss`, a return
without it is not accepted either. A return carrying an error closes the
sign-in as `declined`.

#### The return path and the issuer

A return path of its own shows only which registration a return claims to
belong to. It does not show which authorization server sent the return: an
authorization server that knows its own return path can register that path
for a client of its own at another authorization server. Only `iss` on the
return names the authorization server that issued the code.

A server whose authorization server announces in its metadata that it names
itself on the return (`authorization_response_iss_parameter_supported: true`)
is always checked by that: a return without `iss`, or with another one, is not
accepted. A server whose authorization server does not announce it is signed
in to only when its declaration has `sign_in.accept_return_without_issuer:
true`; without it a call that needs a sign-in is refused naming the server and
the missing acceptance, and once the server's metadata is known to the
computer, `instrukt config validate --company <company>` reports the same.
Declaring it means that the operator trusts that authorization server not to
misuse the return path and state it learns. A compromised one can register
that return path for a client of its own at any other authorization server
that accepts such a registration and names no issuer on its return, whether
or not that server belongs to the company home or is declared in instrukt at
all; it can then send a person there to sign in, receive the code and the
verifier at its own token endpoint, and use the person's login at that
server.

#### Exchange and who signed in

The runner exchanges the code at the token endpoint with the code verifier,
the return address, the client and `resource`. The answer is used only when it
carries an access token of type `Bearer`; an `expires_in` gives its lifetime,
and without one the lifetime is unknown. A refused exchange or an unusable
answer closes the sign-in as `failed`.

It then asks who signed in:

| Declaration | Asked | The address |
| --- | --- | --- |
| With `sign_in.identity` | A `GET` to the declared `url` with the new access token. | The string at `email_field` in the JSON answer. |
| Without it | The OpenID Connect ID token from the token answer, validated as OpenID Connect Core §3.1.3.7 requires (its signature by the provider's key set, `iss`, `aud`, expiry and the nonce), then the provider's UserInfo endpoint. | UserInfo's `email`, accepted only when its `sub` equals the ID token's and `email_verified` is `true`. A sign-in for such a server asks for the scopes `openid` and `email` as well. |

Declaring `sign_in.identity` is the operator's statement that the named field
holds the account's login identifier at that server, unique there at any
moment, as the server reports it for the token presented, and that it is an
email address. A person who changes their login identifier is recognised at
the login's next renewal; until then the stored login acts under the address
it was accepted for. Nothing stronger than what the server reports is
proven.

The login is kept only when that address equals the address stored with the
speaker's message, exactly and without regard to case. The comparison proves
only that the server reports that address for the account that signed in: two
accounts with the same address at the server, or an address reassigned there
to another account, are each accepted as the speaker. A login for another
address, for no address, or without the required identity evidence is
discarded and revoked at the authorization server where it offers revocation;
the sign-in closes as `refused`, and the speaker is told. A failed identity
request closes it as `failed`.

#### Keeping and renewing a login

A login is kept in the state database for the company, the person's address
and the server, with the resource it was issued for (the server's `url` at
sign-in), its registration, the access token and when it runs out, the
refresh token and the scopes. It is never written to the company home. No
value of it is shown to the agent: the output rule below removes it from
everything a server returns.

A login is used only for the resource it was issued for, and renewed only at
its own authorization server with its own client. When the server's declared
`url` differs from that resource, the login is unusable: it is removed,
revoked at its own authorization server where it offers revocation, and the
call starts a sign-in.

Before a call, instrukt renews a login whose access token runs out within a
minute; it also renews once when the server answers a call's access token with
`401`. A `403` is the server's refusal and is not renewed. A call renews a
login at most once. The renewal is a refresh token request with the refresh
token, `resource` and the client. One renewal of a login runs at a time; a
call that finds one under way waits for its result, within its own wait.

| Renewal | Outcome |
| --- | --- |
| Succeeds | The new access token and lifetime are stored in one transaction, the access token marked unchecked. A new refresh token replaces the previous one, which is discarded and not revoked; an answer without one keeps the current refresh token. Who the login identifies as is then asked again. |
| Refused with `invalid_grant` | The login is removed and the call starts a sign-in. |
| Fails otherwise | The call fails naming the server; the login is kept. |
| Not possible: the login has no refresh token | The login is unusable: it is removed and the call starts a sign-in. |

A lifetime the server did not give is unknown, not unlimited: such a login is
used until the server answers `401`.

Each time a login is renewed, who it identifies as is asked again with the new
access token, by the same identity source as at sign-in: the declared identity
request, or the UserInfo endpoint, whose `sub` must equal the one recorded at
sign-in and whose `email` must be verified. Until that check succeeds the new
access token is unchecked: no call uses it, after a restart as well, and every
call waiting on the renewal shares its outcome. At some servers a person can
change their own login address, so a login is kept only while that answer
equals the address it is stored under. When it no longer does, the login is
removed and revoked where the authorization server offers revocation, and the
call starts a sign-in. When the identity request fails, the call fails naming
the server; the login and its newest refresh token are kept, the access token
stays unchecked, and a later call repeats the check, within its own wait,
before it uses the login.

#### Revocation

A revocation request names the kind of token it carries (`token_type_hint`:
`refresh_token` or `access_token`). A revocation request is best effort: a
successful answer is not taken as proof that the token no longer works, and a
failed one does not keep a removed login.

#### Ending a login

| Who | Does |
| --- | --- |
| The person | Asks the agent to be signed out of a server, or of all; the agent runs `instrukt mcp sign-out [<name>]`, which acts for the speaker of its turn: their company, their address and the named server or every server. |
| The local operator | Removes a person's logins for a company with `instrukt mcp sign-out --company <company> --email <address> [<name>]`, outside any session; inside a session this form is refused, and no caller selects another person that way. |
| The authorization server | Refuses a renewal with `invalid_grant`. |

A removal covers the login and every sign-in of that company, person and
server, or of every server, that is open or being exchanged: they close as
`cancelled`, and their waiting calls learn so. It applies also when there is no
login, only a sign-in. Each company, person and server has a generation number
that a removal raises, and every operation captures it when it begins: the
start of a sign-in before its discovery, a claim, an exchange, a renewal and
its identity check, and a call before it sends a login to the server. An
operation whose generation is no longer current when it would act stores
nothing and touches no later login: a sign-in start records no pending
sign-in and its call learns that it was cancelled, an exchange or renewal
revokes the tokens it received where the authorization server offers
revocation, an identity check's outcome changes nothing, and a call does not
send the login and fails saying that it was removed. Network requests run
outside the database's writer section; the generation is compared in the
transaction that records each outcome. Removing a login revokes its refresh
token, or its access token when it has none, at its own authorization server
where it offers revocation, and removes it from the state database whether or
not revocation succeeds. A call already carried out at the server is not
undone. The next call of that person to that server starts a new sign-in.

#### Direct messages

The runner sends these to the speaker, `<name>` being the server's name. Each
is posted for that person alone and accompanies no later message
(`project/instrukt/spec/feature/runner/slack-connector`).

| When | Text |
| --- | --- |
| A sign-in starts | `To let the agent use <name> as you, sign in here. The link works once, for ten minutes: <link>` |
| A sign-in completes while no call waits | `You are signed in to <name>. Ask the agent again to continue.` |
| The login was not for the speaker's address | `That sign-in to <name> was not for your address and was not kept.` |
| The sign-in closed as `declined` or `failed` | `The sign-in to <name> did not complete: <error code>.` |

`<error code>` is the error code the authorization server returned when it
consists of up to 64 of the characters OAuth allows in one, and `unknown`
otherwise; it is sent as plain text.

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
| `instrukt mcp list` | Names each server the caller sees with its description and where it is declared, and for a server with a declared sign-in client the return address to register for it. Contacts no server and reads no secret. |
| `instrukt mcp list-tools <name>` | Returns that server's tool catalog: each tool's name, description and input shape. |
| `instrukt mcp call <name> <tool> --json '<arguments>'` | Calls the tool with the given JSON object and prints the tool's result as JSON. |
| `instrukt mcp sign-out [<name>]` | Removes the speaker's signed-in login for that server, or for every server of the company. Refused in a session with no speaker. |
| `instrukt mcp sign-out --company <company> --email <address> [<name>]` | The local operator's, outside any session: removes that person's signed-in logins and open sign-ins for the company, for one server or all. |

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
Given a company member asks the company's agent for work that needs a server the company declares with stored logins
When the agent calls a tool of that server
Then the call reaches the server under that member's own login
And the agent supplied no credential
But when that member has no stored login for the server
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
Given a company member asks the agent in a direct message for work that needs a server the company declares with sign-in
And the member has never signed in to that server
When the agent calls a tool of that server
Then the member receives a sign-in link in a direct message, and nowhere else
And the link reaches the agent neither then nor with a later message
When the member signs in with their own address while the call waits
Then the call reaches the server under the member's own login
And the same turn answers
But when the member signs in with an account the server reports under another address
Then that login is not kept, the member is told so, and the agent's call learns that the sign-in was not kept
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

### UC-MA7

### UC-MA8: Keep working without signing in again

```gherkin
Given a member has signed in to a server
When their login runs out and the agent calls a tool for them
Then instrukt renews the login and the call reaches the server
But when the authorization server no longer accepts the login's renewal
Then the login is removed and a new sign-in starts
But when, at a renewal, the server reports the login under another address than the member's
Then the login is removed and a new sign-in starts
```

### UC-MA9: Sign out of a server

```gherkin
Given a member has signed in to a server
When they ask the agent to sign them out of it
Then their login is removed from the computer
And a request to revoke it is sent where the server offers revocation, whether or not that ends it at the server
And the agent's next call for them starts a new sign-in
```

### UC-MA10: Remove a person's logins as an operator

```gherkin
Given a person has signed in to a company's servers
When the operator removes that person's logins for the company
Then none of them is kept on the computer
And a sign-in that person had started keeps nothing when they complete it
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
- A call that waits on a sign-in holds the agent's turn for up to 90 seconds.
- A sign-in's return reaches the issuing computer only through cluster Redis:
  a computer that cannot reach it completes no sign-in, and a return arriving
  while it cannot be reached is lost; the person signs in again. A writer to
  cluster Redis can disturb returns; cluster Redis is trusted for that.
- Clients are registered beforehand or dynamically. A client identified by a
  metadata document of its own is not supported, nor is an authorization
  server without `S256`, a public client the authorization server registers
  differently than asked, or, without `sign_in.identity`, an authorization
  server that is no OpenID Provider. Each is refused with its reason.
- A server whose authorization server does not announce that it names itself
  on the return is signed in to only on the operator's acceptance; a
  compromised authorization server could then capture a person's sign-in at
  any other authorization server that accepts a registration of its return
  path and names no issuer on its return, inside the company home or outside
  it, declared in instrukt or not.
- The address comparison relies on what the server reports: with OpenID
  Connect a verified address, with a declared identity request what the
  operator's declaration asserts. It does not prove that one person holds the
  account.
- An authorization server that keeps a replaced refresh token valid, as
  instrukt discards it rather than revoking it, leaves a copy of that token
  usable at the server after renewal and after sign-out, possibly without
  end; one that offers no revocation keeps every token it issued valid after
  sign-out until it expires there.
- A server may keep honouring a login's renewal for an account it has
  disabled while refusing the renewed access token. The stored login is then
  paused, not ended: calls fail naming the server until the account is
  enabled again or the login is removed.
- A server may send a refusal back without a `state`. Such a return belongs
  to no sign-in: the person sees the neutral page, the agent's call learns
  that the sign-in is still pending, and the sign-in ends when it expires.
- A person who can no longer reach the agent keeps their logins in the state
  database until they sign out, the operator removes them, or the
  authorization server refuses a renewal.
- Logins lie in the state database, where any process of the same
  operating-system user can read them. With a person's own login the agent
  holds that person's rights at the server, and in a channel text of other
  people accompanies the speaker's message and can steer what happens under
  the speaker's name.
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
- https://www.rfc-editor.org/rfc/rfc6749 — OAuth 2.0: the authorization code grant, `state`, optional refresh tokens, the refresh token grant and replacing a refresh token, `invalid_grant`; a code lifetime of at most ten minutes is recommended. The ten minutes of a pending sign-in are instrukt's own.
- https://www.rfc-editor.org/rfc/rfc7636 — PKCE: a verifier of 43 to 128 unreserved characters with at least 256 bits of entropy, and `S256`, the base64url form of the verifier's SHA-256 digest.
- https://www.rfc-editor.org/rfc/rfc8707 — resource indicators: the `resource` parameter, an absolute identifier without a fragment, in authorization and token requests.
- https://www.rfc-editor.org/rfc/rfc9728 — protected resource metadata: `resource`, `authorization_servers`, `scopes_supported`; `resource` must equal the protected resource's identifier the metadata address was built from.
- https://www.rfc-editor.org/rfc/rfc8414 — authorization server metadata: `issuer`, `registration_endpoint`, `revocation_endpoint`, `code_challenge_methods_supported`; `issuer` must equal the issuer identifier the metadata address was built from.
- https://www.rfc-editor.org/rfc/rfc7591 — dynamic client registration: `redirect_uris`, `token_endpoint_auth_method: none` for a public client; a server may protect registration or register other values than asked, which its answer shows.
- https://www.rfc-editor.org/rfc/rfc7009 — token revocation: `token` and `token_type_hint`; revoking one token may revoke others of the same grant; success also for an unknown token.
- https://www.rfc-editor.org/rfc/rfc9207 — the `iss` authorization response parameter, compared exactly, and refused when missing from a server that says it sends it.
- https://www.rfc-editor.org/rfc/rfc9700 — OAuth security best current practice: mix-up attacks between authorization servers and their defenses, among them a distinct return address per authorization server.
- https://openid.net/specs/openid-connect-core-1_0.html — the ID token and its validation (§3.1.3.7), the UserInfo endpoint and its `sub` agreeing with the ID token's (§5.3.2), the `openid` and `email` scopes, `email` and `email_verified`.
