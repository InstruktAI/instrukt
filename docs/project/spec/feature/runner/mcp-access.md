---
description: MCP access — agents reach declared MCP servers through `instrukt mcp list|list-tools|call`; a server is declared machine-wide or in a company home, its tools load only when listed, and a call carries the speaker's or the company's login without the agent handling it.
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
| `mcp.<name>` | The server's name, following the grammar of a project slug. Declared in the global layer for every session on the computer, or in a company home for work done for that company. |
| `url` | The server's network address. Required. |
| `description` | One line saying what the server is for; `list` shows it. Required. |
| `credential` | Company home only: the name under `members.<member>.credentials` whose value a speaker's call carries (`project/instrukt/spec/feature/configuration/company-homes`). |
| `login` | A `${NAME}` reference into the secrets file of where the server is declared, the company home's or the machine's: that company's or machine's own login for the server. Never a literal value. |

A company home's declaration takes the place of a machine-wide declaration of
the same name for that company's work. An unknown field, a literal `login`, a
`credential` in the global layer and a missing `url` or `description` are
validation findings naming the server and the field.

### Whose declarations a call sees

The command recognises its caller through the selected runtime's caller
identity, as `instrukt discuss` does
(`project/instrukt/spec/feature/runner/conversation-links`), and reads the
runner's recorded assignment for that session.

| Caller | Declarations |
| --- | --- |
| A session whose recorded assignment names a company home | That company's, and the machine-wide ones it does not replace. `--company` naming another company is refused. |
| Any other caller, with `--company <name>` | That company's, and the machine-wide ones it does not replace. |
| Any other caller | The machine-wide ones. |

### The login a call carries

The speaker is the company member the runner recorded as the sender of the
addressed message that started the calling session's current turn
(`project/instrukt/spec/feature/runner/conversations`). A session with no such
record has no speaker.

| Call | Login |
| --- | --- |
| Company declaration with `credential`, and a speaker | The speaker's credential of that name. A speaker without it is refused naming the member and the missing credential; no other login takes its place. |
| Company declaration, and no speaker | The declaration's `login`. Without one the call is refused naming the server and the missing login. |
| Machine-wide declaration | Its `login` when declared, otherwise none. |

The resolved value is sent as the request's `Authorization` header. The agent
passes no credential, and none appears in the command's output, in logs or in
diagnostics, which name references only.

### Commands

| Command | Does |
| --- | --- |
| `instrukt mcp list` | Names each server the caller sees with its description and declaring home. Contacts no server and reads no secret. |
| `instrukt mcp list-tools <name>` | Returns that server's tool catalog: each tool's name, description and input shape. |
| `instrukt mcp call <name> <tool> --json '<arguments>'` | Calls the tool with the given JSON object and prints the tool's result as JSON. |

- `list` and `list-tools` accept `--json`.
- `list-tools` and `call` each open their own connection, perform the request
  and close it. No connection and no server state outlives the command.
- An unknown server name is refused naming the servers the caller sees. A
  failure the server reports is printed as the server gave it, with a non-zero
  exit. An unreachable server fails naming the server and its address.
- The command writes no agent program's settings.

## Use cases

### UC-MA1: An agent uses a tool as the person who asked

```gherkin
Given a company member asks the company's agent for work that needs a server the company declares
When the agent calls a tool of that server
Then the call reaches the server under that member's own login
And the agent supplied no credential and sees none
But when that member has no login for the server
Then the call is refused naming the member and the missing login
And no other login is used
```

### UC-MA2: An agent working with nobody asking uses the company's own login

```gherkin
Given an agent started by an event works in a company's home
When it calls a tool of a server that company declares
Then the call reaches the server under the company's own login
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
