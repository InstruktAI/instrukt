---
description: Company containers — on a Linux host each company runs in one long-running container of its own, with T3 and the agent tools inside; its agents reach no other company and nothing of the host, its state lives in its folder on the host and survives the container, it is signed in once, its tools take each release that passed instrukt's nightly check automatically and without a new image, shared defaults reach every company, no credential that opens its environment is kept in a log, and its irreplaceable state can be exported while it runs and imported on a prepared host.
---

# Company Containers — Spec

## What it is

A host runs several companies. Each company runs in one container of its
own, and every agent session that works for that company runs there and
nowhere else: its conversations
(`project/instrukt/spec/feature/company/conversations`) and the procedures
that run in its home
(`project/instrukt/spec/feature/runner/procedure-standing-agent`). T3 is the
runtime inside the container. The container is long-running and holds no
state of its own: the company's state lives in the company's folder on the
host.

A **company's folder** is the one folder on the host, under the operator's
`~/.instrukt`, that holds all of one company's state. The company's home
(`project/instrukt/spec/feature/configuration/company-homes`) is part of it;
how the rest is laid out beside the home is the architecture's.

Three kinds of people are involved. The company's **members** talk to its
agent. The **operator** runs the host and sets companies up on it. **Another company** on the same host is kept apart from this one.

## Canonical fields

### The container

| Fact | Contract |
| --- | --- |
| Host | A Linux computer. A Mac runs the same container for testing and is no host. |
| One per company | A company has one container on a host, and a container serves one company. Two companies never share a container, its runtime or its agent program's login (`sdlc/policy/security-awareness`: isolate components so a compromise in one cannot reach others). |
| Long-running | The container runs until the operator stops it, and starts again with the host. A session in it keeps its conversation across a restart under `project/instrukt/spec/feature/runner/session-continuity`. |
| What runs in it | T3 and the agent programs it starts, for this company only. |
| The image | Lean: it carries nothing of any company, and the agent tools are not part of it. |

### What a company's container reaches

- The company's own folder: the company's home at the location
  `project/instrukt/spec/feature/configuration/company-homes` names, and the
  company's projects. Each has one location that holds on the host and in the
  container alike, so a document or procedure that names the company's home or
  its projects names one path, valid wherever it is read.
- The services the company is connected to: the channels its connectors
  speak to, the MCP servers it declares
  (`project/instrukt/spec/feature/runner/mcp-access`), the agent program's
  provider and the shared services of the host the company uses.
- Nothing else of the host: no other company's folder, no file of the
  operator's, no state database or secrets file that holds another company's
  values, and no way to start, stop or change another container or the host
  (`sdlc/policy/security-awareness`: isolate components so a compromise in one
  cannot reach others).

Every behaviour the company context and the runner specify for a company
holds the same in its container: who is answered, the answer posted back,
whose login a call carries (`project/instrukt/spec/domain/company`,
`project/instrukt/spec/feature/runner/mcp-access`).

### Where a company's state lives

Everything a company needs to run, except the image and what the operator
maintains for every company, lies in the company's folder on the host, which
the container uses as its own. None of it lives only in a container
(`sdlc/policy/state-storage`: state whose loss forces more than cheap local
recomputation is persisted and survives restarts).

| State | Survives the container being recreated |
| --- | --- |
| The company's home: settings, secrets, documents | Yes |
| The company's projects | Yes |
| The installed tools: T3 and the agent programs, each at the version it was updated to | Yes |
| T3's data: projects, threads, its own server keys | Yes |
| The agent program's login, its settings and its transcripts | Yes |
| Caches the tools rebuild | Not required |

A company's state never lies in another company's folder, and no folder holds
two companies' transcripts or logins (`sdlc/policy/security-awareness`: scope
secret access to the component that needs it).

### Signing a company in

The operator signs a company's agent program in once, from the host, with the
account the company's agent uses. The holder of that account approves the
sign-in in a browser on any device, a phone included, and hands back the code
the browser shows. Nothing from outside the host has to reach the container
for it. The login lands in the company's folder and is renewed by the agent
program itself; it survives restarts and recreation until the account holder
or the operator ends it (`sdlc/policy/state-storage`: state whose loss forces
more than cheap local recomputation is persisted and survives restarts). A
wrong code leaves the sign-in waiting for the right one. A sign-in that ends
unfinished, cancelled or expired, leaves no login and nothing waiting.

### Tools and their releases

The tools in a company's container are installed into the company's folder,
not into the image. Each tool takes its newest release that has passed
instrukt's nightly check against that tool, automatically: once a release has
passed, it reaches every company's container without anyone acting for one
company, without the image being rebuilt, without a version set by hand, and
without touching a company's logins, threads or projects. A release that fails
the check is not taken, and the failure is reported as an anomaly through
`project/instrukt/spec/feature/runner/anomaly-notification`; every company
keeps the release it runs meanwhile.

### Shared defaults

The operator maintains one source of defaults for every company's tools, for
example that no conversation is compacted automatically after it has been
idle. It applies to every company's container, and a change to it reaches
every company from that one source.

### Credentials that open an environment

No credential that opens a company's environment is kept in any log or
diagnostic output, whoever can read it; starting or restarting a company's
container writes none there (`sdlc/policy/security-awareness`: secrets never
enter logs).

### Export and import

- instrukt exports a company's irreplaceable state as one consistent copy
  while the company keeps running: its settings and secrets, its
  conversations, its agent's sessions with their history and the logins kept
  for it. A database in it is copied consistently even while it is being
  written to. What a repository governs, the company's own documents and every
  checkout under its projects, and what can be installed again, the agent
  tools, are not in the copy.
- Work in a checkout that is not committed and pushed is in no copy; the
  export names every such checkout in its output.
- instrukt imports such a copy on a host where the operator's prerequisites
  for companies are in place. The company then starts and works, with its
  conversations, sessions and logins as they were when the copy was taken, and
  no other company is touched. Its documents and projects come from their
  repositories and its tools are installed again.

## Use cases

### UC-CC1: Keep a company's agent away from other companies and the host

```gherkin
Given two companies run on one host
When a member of the first company asks its agent to read or change the second company's files or a file of the host
Then the agent cannot reach them
And the second company's files and the host's are unchanged
But the first company's agent still answers through the services its company is connected to
```

### UC-CC2: Keep each company's transcripts and logins its own

```gherkin
Given two companies on one host, each signed in with its own account
When members of both companies talk to their agents
Then each company's conversations are recorded only in its own folder
And each company's agent works under its own company's login
And neither company's agent can read the other's transcripts or login
```

### UC-CC3: Sign a company's agent in once

```gherkin
Given an operator has set a company up on a host and its agent is not signed in
When the operator starts the sign-in from the host and the account holder approves it in a browser on their phone and hands back the code
Then the company's agent answers its members under that account
And nothing outside the host had to reach the company's container
But a sign-in given a wrong code leaves the company signed out and can be completed with the right one
And a sign-in that ends unfinished leaves no login and nothing waiting
```

### UC-CC4: Keep a company running when its container is replaced

```gherkin
Given a signed-in company whose members have conversations with its agent and whose agent works on its projects
When the operator replaces the company's container with a new one from the same image
Then the company's agent answers without being signed in again
And a member's conversation continues where it was
And the company's projects, threads and installed tools are as they were
```

### UC-CC5: Update the tools of every company without a new image

```gherkin
Given an operator runs companies on a host
When a new release of their tools passes instrukt's nightly check
Then every company's container runs the new release without anyone acting for one company
And no image was rebuilt and no version was set by hand
And each company's logins, conversations and projects are unchanged
But a release that fails the check is not taken, every company keeps the release it runs and the failure is reported
```

### UC-CC6: Change a shared default for every company at once

```gherkin
Given an operator maintains the defaults shared by every company on a host
When the operator changes one of them
Then every company's container runs with the changed default
And no company had to be changed on its own
```

### UC-CC11: Start a company's environment without leaving a credential in its log

```gherkin
Given an operator runs a company on a host
When the company's container starts or restarts
Then no credential that opens the company's environment is kept in any log or diagnostic output
```

### UC-CC12: Export a company while it runs

```gherkin
Given an operator runs a company whose members are talking to its agent
When the operator exports that company
Then the copy holds its settings, secrets, conversations, sessions and logins, consistent as of one moment
And no checkout of a repository and no installed tool is in it
And the export names every checkout that holds work not committed and pushed
And the company's agent kept answering its members throughout
```

### UC-CC13: Import a company on a prepared host

```gherkin
Given an operator has an exported copy of a company and a host prepared for companies that runs other companies
When the operator imports the copy there
Then the company starts with its settings, secrets, conversations, sessions and logins as they were when the copy was taken
And its documents and projects come back from their repositories and its tools are installed again
And its agent answers its members
And no other company on the host changed
```

## Known caveats

- Inside its container a company's team agent is not confined further: it
  can read, change or erase anything of its own company's that the container
  holds, the company's secrets file and its login included. Importing its
  last export brings back what that copy holds and what the repositories
  hold; anything newer, and work in a checkout that was not committed and
  pushed, is lost.
- A company's container is not confinement enough for an agent that answers
  people outside the company, and this feature delivers no such agent.
- A Mac is for testing the container, not for hosting companies.
- A company whose container is stopped receives no messages; a message
  written meanwhile is received when the container runs again only as far as
  its connector's provider still offers it
  (`project/instrukt/spec/feature/company/conversations`).

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/company/conversations](conversations.md)
- [project/instrukt/spec/feature/runner/mcp-access](../runner/mcp-access.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](../runner/anomaly-notification.md)
- [project/instrukt/spec/feature/runner/t3-contract](../runner/t3-contract.md)
