---
description: Company containers — on a Linux host each company runs in one long-running container of its own, with T3 and the agent tools inside; its agents reach no other company and nothing of the host, its state lives in its folder on the host and survives the container, it is signed in once, its tools follow their releases without a new image, shared defaults reach every company, its owner opens its environment without the operator's credentials, and every company is backed up each night.
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
host, so a container can be replaced, a company restored from the nightly
backup, and the tools updated without a new image
(`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`).

A **company's folder** is the one folder on the host, under the operator's
`~/.instrukt`, that holds all of one company's state. The company's home
(`project/instrukt/spec/feature/configuration/company-homes`) is part of it;
how the rest is laid out beside the home is the architecture's.

Four kinds of people are involved. The company's **members** talk to its
agent. The company's **owner** may open the company's own environment. The
**operator** runs the host and sets companies up on it. **Another company**
on the same host is kept apart from this one.

## Canonical fields

### The container

| Fact | Contract |
| --- | --- |
| Host | A Linux computer. A Mac running Docker runs the same container for testing and is no host (`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`). |
| One per company | A company has one container on a host, and a container serves one company. Two companies never share a container, its runtime or its agent program's login (`sdlc/policy/security-awareness`, Least privilege & blast radius). |
| Long-running | The container runs until the operator stops it, and starts again with the host. A session in it keeps its conversation across a restart under `project/instrukt/spec/feature/runner/session-continuity`. |
| What runs in it | T3 and the agent programs it starts, for this company only (`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`). |
| The image | Carries the operating system, the container's user and the steps that install and start the tools, and nothing of any company. Every company's container runs the same image (`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`). |

### What a company's container reaches

- The company's own folder: the company's home at the location
  `project/instrukt/spec/feature/configuration/company-homes` names, the same
  in the container as on the host, and the company's projects.
- The services the company is connected to: the channels its connectors
  speak to, the MCP servers it declares
  (`project/instrukt/spec/feature/runner/mcp-access`), the agent program's
  provider and the shared services of the host the company uses, such as a
  shared ERPNext.
- Nothing else of the host: no other company's folder, no file of the
  operator's, no state database or secrets file that holds another company's
  values, and no way to start, stop or change another container or the host
  (`sdlc/policy/security-awareness`, Least privilege & blast radius).

Every behaviour the company context and the runner specify for a company
holds the same in its container: who is answered, the answer posted back,
whose login a call carries (`project/instrukt/spec/domain/company`,
`project/instrukt/spec/feature/runner/mcp-access`).

### Where a company's state lives

Everything a company needs to run, except the image and what the operator
maintains for every company, lies in the company's folder on the host, which
the container uses as its own. Losing any of it costs more than a restart, so
none of it lives only in a container (`sdlc/policy/state-storage`, Durability
follows loss cost).

| State | Survives the container being recreated |
| --- | --- |
| The company's home: settings, secrets, documents | Yes |
| The company's projects | Yes |
| The installed tools: T3 and the agent programs, each at the version it was updated to | Yes |
| T3's data: projects, threads, sessions of people with access, its own server keys | Yes |
| The agent program's login, its settings and its transcripts | Yes |
| Caches the tools rebuild | Not required |

A company's state never lies in another company's folder, and no folder holds
two companies' transcripts or logins (`sdlc/policy/security-awareness`,
Secrets management).

### Signing a company in

The operator signs a company's agent program in once, from the host, with the
account the company's agent uses. The holder of that account approves the
sign-in in a browser on any device, a phone included, and hands back the code
the browser shows. Nothing from outside the host has to reach the container
for it. The login lands in the company's folder and is renewed by the agent
program itself; it survives restarts and recreation until the account holder
or the operator ends it (`sdlc/policy/state-storage`, Durability follows loss
cost). A sign-in that was not completed leaves no login and no waiting
process behind.

### Tools and their releases

The tools in a company's container are installed by an install step into the
company's folder, not into the image. A new release of a tool reaches every
company's container without the image being rebuilt, without anything being
pinned by hand for one company, and without touching a company's logins,
threads or projects
(`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`). Which
releases are taken, and when, is the same for every company on a host.

### Shared defaults

The operator maintains one set of defaults for every company's tools, for
example that no conversation is compacted automatically after it has been
idle. They are kept on the host, outside every company's folder, and apply to
every company's container; a change to them reaches every company without any
company's own files being edited
(`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`). What
bounds what a company's agent may do and what its container reaches cannot be
changed from inside the container, by the company's owner or by an agent
(`sdlc/policy/security-awareness`, Posture: fail closed).

### The owner's access

- The operator gives a company's owner access to that company's environment:
  the T3 of the company's container. The owner opens it on their own device,
  with a login of their own; the operator's credentials are never handed over
  (`sdlc/policy/security-awareness`, Secrets management: no shared root
  credentials).
- The owner sees and works in that one company's environment only. Nothing the
  owner does there, themselves or through an agent they start, reaches beyond
  the company's container (`sdlc/policy/security-awareness`, Least privilege &
  blast radius).
- The operator can revoke an owner's access at any time, and a revoked owner
  is turned away on their next request (`sdlc/policy/security-awareness`,
  Authentication & authorization).
- Nothing that opens a company's environment is written to a log or output
  that someone without that access can read; a credential that is shown once
  to be handed to its holder is shown only to the person who asked for it
  (`sdlc/policy/security-awareness`, Observability & incident readiness: never
  log secrets).

### The operator's access

The operator reaches each company's environment from the host to set it up,
sign it in, update it and look into it when needed, with access of the
operator's own. The operator does not need any company's account to do so
(`sdlc/policy/security-awareness`, Least privilege & blast radius: separate
credentials per service).

### Backup and restore

- Every night the operator's home `~/.instrukt` on the host, which holds every
  company's folder, is archived and uploaded to S3-compatible storage as one
  timestamped archive. The last ten archives are kept; older ones are
  removed. No separate snapshot exists (`sdlc/policy/state-storage`,
  Durability follows loss cost).
- Each company's folder in an archive is restorable on its own: it is taken
  while its stores are consistent, so a company restored from it starts and
  works.
- An archive holds every company's secrets and logins. It is written only to
  the storage the operator configures, read only with the operator's
  credentials for that storage, and never left where a company's container
  can read it (`sdlc/policy/security-awareness`, Secrets management).
- Restoring a company puts its folder back as it was in a chosen archive and
  touches no other company. Bringing a company up on another host is the
  same: its folder and the same image.
- A night whose backup fails is reported to the operator as an anomaly
  (`project/instrukt/spec/feature/runner/anomaly-notification`); the archives
  already kept stay.

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
Given an operator runs companies on a host and a new release of their tools is taken
When the update reaches the host
Then every company's container runs the new release
And no image was rebuilt and no company's version was set by hand
And each company's logins, conversations and projects are unchanged
```

### UC-CC6: Change a shared default for every company at once

```gherkin
Given an operator maintains the defaults shared by every company on a host
When the operator changes one of them
Then every company's container runs with the changed default
And no company's own files were edited
But a company's owner or agent who tries to change what bounds the company's agent from inside its container leaves it unchanged
```

### UC-CC7: Open one's own company's environment without the operator's credentials

```gherkin
Given an operator has given a company's owner access to their company's environment
When the owner opens it on their own device
Then they see and can work with that company's agents and nothing of another company
And they never received the operator's credentials
And nothing they or an agent they start do there reaches beyond their company's container
But once the operator revokes their access they are turned away
```

### UC-CC8: Back up every company each night

```gherkin
Given an operator runs several companies on a host and has configured their backup storage
When a night passes
Then one new archive holding every company's folder is in the storage
And only the last ten archives are kept
And no company's container can read any archive
```

### UC-CC9: Restore a company whose folder was damaged

```gherkin
Given a company's agent erased the company's projects
When the operator restores that company from the archive of the night before
Then the company's projects, conversations and login are as they were that night
And its agent answers its members again
And no other company on the host changed
```

## Known caveats

- Inside its container a company's team agent is not confined further: T3
  cannot take tools away from a session, so the agent can read, change or
  erase anything of its own company's that the container holds, the
  company's secrets file and its login included. A restore from the last
  archive is the answer, and work since that night is lost
  (`project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own`).
- An owner who may start agents in their company's environment can, through
  such an agent, do anything inside the company's container that the agent
  can.
- A connector open to people outside the company needs an agent with no
  tools. A company's container does not provide one.
- A Mac is for testing the container, not for hosting companies.
- A company whose container is stopped receives no messages; a message
  written meanwhile is received when the container runs again only as far as
  its connector's provider still offers it
  (`project/instrukt/spec/feature/company/conversations`).

## See Also

- [project/instrukt/adr/0001-a-company-runs-in-a-container-of-its-own](../../../adr/0001-a-company-runs-in-a-container-of-its-own.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/company/conversations](../company/conversations.md)
- [project/instrukt/spec/feature/runner/mcp-access](mcp-access.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
- [project/instrukt/spec/feature/runner/t3-contract](t3-contract.md)
