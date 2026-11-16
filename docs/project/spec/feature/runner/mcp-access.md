---
description: MCP access — agents reach declared MCP servers through `instrukt mcp list|list-tools|call`; a server is declared machine-wide or in a company home, its tools load only when listed, and a call carries the speaker's login, or the company's own when nobody is asking.
---
# MCP Access — Spec

## What it is

An agent reaches an MCP server through `instrukt mcp`, never through its
agent program's own settings. A server is declared once, for the machine or
for one company. Listing servers costs one line each; a server's tools enter
the agent's context only when it lists them. A call carries the right login by
code: the person being answered, or the company itself when nobody is asking.

## Canonical fields

### Declaration

```yaml
mcp:
  erpnext:
    url: https://erp.example.com/api/v2/mcp
    description: Operate the company's ERPNext records
    credential: erpnext
    login: ${ERPNEXT_AGENT_LOGIN}
```

| Field | Contract |
| --- | --- |
| `mcp.<name>` | The server's name, following the grammar of a project slug. Declared in the global layer for the computer, or in a company home for work done for that company. |
| `url` | The server's network address. Required. |
| `description` | One line saying what the server is for; `list` shows it. Required. |
| `credential` | Company home only: the name under `members.<member>.credentials` whose value a speaker's call carries (`project/instrukt/spec/feature/configuration/company-homes`). |
| `login` | A `${NAME}` reference, never a literal value. In a company home it is the company's own login, references that home's secrets file, requires `credential`, and is carried only by a call with no speaker. In the global layer it is the machine's own login and references the machine's secrets file. |

A declaration that carries neither `credential` nor `login` is an open server:
its calls carry no login. A company home's declaration takes the place of a
machine-wide declaration of the same name for that company's work. An unknown
field, a literal `login`, a `credential` in the global layer, a `login` without
`credential` in a company home and a missing `url` or `description` are
validation findings naming the server and the field.

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

### The login a call carries

The speaker is the company member the runner recorded as the sender of the
addressed message that started the calling session's current turn
(`project/instrukt/spec/feature/runner/conversations`). A session with no such
record has no speaker. No argument and no value the agent passes selects a
member.

| Declaration | Speaker | Login |
| --- | --- | --- |
| Open, machine-wide or a company's | Any, or none | None. |
| A company's, with `credential` | A member holding that credential | That member's credential. |
| A company's, with `credential` | A member without it | The call is refused naming the member and the missing credential. |
| A company's, with `credential` and `login` | None | The company's `login`. |
| A company's, with `credential` and no `login` | None | The call is refused naming the server and the missing company login. |
| Machine-wide, with `login` | None: it is offered to no session that has a speaker | The machine's `login`. |

A call that has a speaker never carries a company's or the machine's login.
The resolved value is sent as the request's `Authorization` header.

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

- `list` and `list-tools` accept `--json`.
- `list-tools` and `call` each open their own connection, perform the request
  and close it. No connection and no server state outlives the command.
- An unknown server name is refused naming the servers the caller sees. An
  unreachable server fails naming the server and its address. A failure the
  server reports reaches the caller with the server's cause and a non-zero
  exit.
- Before anything a server returned is printed or logged, a result, a catalog
  or a failure, every resolved credential value is removed from it. The
  command's own output, logs and diagnostics name references, never values.
- The command writes no agent program's settings.

## Use cases

### UC-MA1: An agent uses a tool as the person who asked

```gherkin
Given a company member asks the company's agent for work that needs a server the company declares
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
When that work needs a server the company declares
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

## Known caveats

- Only servers reached over the network are supported. A server that runs as a
  local program, or that keeps state between calls, is not.
- A server that requires an interactive sign-in is not supported; the login is
  a stored value.
- The agent program's own MCP settings are untouched and unknown to instrukt:
  a server configured there is reached by that program, without these rules.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/conversations](conversations.md)
