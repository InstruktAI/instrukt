---
description: Company containers — on a Linux host each company runs in one long-running container of its own, with T3 and the agent tools inside; its agents reach no other company and nothing of the host, its state lives in its folder on the host and survives the container, it is signed in once, its tools follow their selected releases automatically without a new image, shared defaults reach every company, its owner opens its environment without the operator's credentials, and every company is backed up each night.
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

Four kinds of people are involved. The company's **members** talk to its
agent. The company's **owner** may open the company's own environment. The
**operator** runs the host and sets companies up on it. **Another company**
on the same host is kept apart from this one.

## Canonical fields

### The container

| Fact | Contract |
| --- | --- |
| Host | A Linux computer. A Mac running Docker runs the same container for testing and is no host. |
| One per company | A company has one container on a host, and a container serves one company. Two companies never share a container, its runtime or its agent program's login (`sdlc/policy/security-awareness`: isolate components so a compromise in one cannot reach others). |
| Long-running | The container runs until the operator stops it, and starts again with the host. A session in it keeps its conversation across a restart under `project/instrukt/spec/feature/runner/session-continuity`. |
| What runs in it | T3 and the agent programs it starts, for this company only. |
| The image | Lean: it carries nothing of any company, and the agent tools are not part of it. |

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
| T3's data: projects, threads, sessions of people with access, its own server keys | Yes |
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
not into the image, and follow their selected releases automatically: a
selected release reaches every company's container without anyone acting for
one company, without the image being rebuilt, without a version pinned by
hand, and without touching a company's logins, threads or projects. The
selection is the same for every company on a host. Which releases are
selected, every new release or only one that has passed instrukt's nightly
test against T3, is outside this contract.

### Shared defaults

The operator maintains one source of defaults for every company's tools, for
example that no conversation is compacted automatically after it has been
idle. It applies to every company's container, and a change to it reaches
every company from that one source.

### Limits a company cannot change

What bounds what a company's agent may do, and what its container reaches, is
set by the operator and cannot be changed from inside the container, by anyone
working there or by an agent (`sdlc/policy/security-awareness`: scope
credentials, permissions and network access so a compromise in one component
cannot cascade).

### The owner's access

- The operator gives a company's owner access to that company's environment:
  the T3 of the company's container. The owner opens it on their own device,
  with a login of their own; the operator's credentials are never handed over
  (`sdlc/policy/security-awareness`: no shared root credentials).
- The owner reaches that one company's environment and nothing beyond the
  company's container (`sdlc/policy/security-awareness`: isolate components
  so a compromise in one cannot reach others).
- The operator can revoke an owner's access at any time, and a revoked owner
  is turned away on their next request, also in a session they already had
  open (`sdlc/policy/security-awareness`: session invalidation is immediate
  and server-side).
- Whether the owner may start or steer agents in that environment, and whether
  the owner may give their own people access to it, is outside this contract.
  The two depend on each other: an agent working in the company's environment
  can reach what grants access to it, so an owner who may start agents can
  give access through one unless such an agent is kept from it. If the owner
  may start or steer agents, nothing such an agent does reaches beyond the
  company's container. If the owner may give access, what they give reaches
  that one company's environment only.

### Credentials that open an environment

No credential that opens a company's environment is kept in any log or
diagnostic output, whoever can read it; starting or restarting a company's
container writes none there (`sdlc/policy/security-awareness`: secrets never
enter logs). Handing a newly issued credential once to the person who asked
for it is a handoff, not a log: it reaches only that person and is not kept.

### The operator's access

The operator reaches each company's environment from the host to set it up,
sign it in, update it and look into it when needed, with access of the
operator's own. The operator does not need any company's account to do so
(`sdlc/policy/security-awareness`: separate credentials per service and
environment).

### Backup and restore

- Every night the operator's home `~/.instrukt` on the host, which holds every
  company's folder, is archived and uploaded as one timestamped archive to
  S3-compatible storage the operator sets through configuration. The newest
  archives, as many as the backup's settings name, are kept and older ones are
  removed; a night whose backup fails removes none. No separate snapshot exists
  (`sdlc/policy/state-storage`: state whose loss forces more than cheap local
  recomputation is persisted and survives restarts).
- Each company's folder in an archive is restorable on its own, and a company
  restored from it starts and works: the archive holds its stores in a
  consistent state.
- An archive holds every company's secrets and logins. It is written only to
  the storage the operator configures, read only with the operator's
  credentials for that storage, and never left where a company's container
  can read it (`sdlc/policy/security-awareness`: scope secret access to the
  component that needs it).
- Restoring a company puts its folder back as it was in a chosen archive, on a
  host where the operator's prerequisites for companies are in place, and
  touches no other company. A host prepared the same way can take a company
  restored from an archive; nothing here moves a company by itself or keeps a
  second copy of it running.
- A night whose backup fails leaves the archives already kept in place and is
  reported as an anomaly through
  `project/instrukt/spec/feature/runner/anomaly-notification`, which reaches
  the operator as far as that contract's notification is configured.

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
When a new release of their tools is selected
Then every company's container runs the new release without anyone acting for one company
And no image was rebuilt and no version was set by hand
And each company's logins, conversations and projects are unchanged
```

### UC-CC6: Change a shared default for every company at once

```gherkin
Given an operator maintains the defaults shared by every company on a host
When the operator changes one of them
Then every company's container runs with the changed default
And no company had to be changed on its own
```

### UC-CC7: Open one's own company's environment without the operator's credentials

```gherkin
Given an operator has given a company's owner access to their company's environment
When the owner opens it on their own device
Then they reach that company's environment and nothing of another company
And they never received the operator's credentials
But once the operator revokes their access they are turned away on their next request
```

### UC-CC8: Back up every company each night

```gherkin
Given an operator runs several companies on a host and has configured their backup storage
When a night passes
Then one new archive holding every company's folder is in the storage
And only the configured number of newest archives is kept
And no company's container can read any archive
But a night whose backup fails leaves the archives already kept in place and is reported as an anomaly where anomaly notification is configured
```

### UC-CC9: Restore a company whose folder was damaged

```gherkin
Given a company's agent erased the company's projects
When the operator restores that company from the archive of the night before
Then the company's projects, conversations and login are as they were that night
And its agent answers its members again
And no other company on the host changed
```

### UC-CC10: Keep a company's limits as the operator set them

```gherkin
Given an operator has set what a company's agent may do and what its container reaches
When someone working in the company's container tries to loosen those limits, directly or through the company's agent
Then the limits stay as the operator set them
And the company's agent still reaches nothing beyond its container
```

### UC-CC11: Start a company's environment without leaving a credential in its log

```gherkin
Given an operator runs a company on a host
When the company's container starts or restarts
Then no credential that opens the company's environment is kept in any log or diagnostic output
But a credential the operator issues for someone is handed once to the person who asked for it
```

## Known caveats

- Inside its container a company's team agent is not confined further: it
  can read, change or erase anything of its own company's that the container
  holds, the company's secrets file and its login included. A restore from
  the last archive brings it back, and work since that night is lost.
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

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/company/conversations](../company/conversations.md)
- [project/instrukt/spec/feature/runner/mcp-access](mcp-access.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/runner/t3-contract](t3-contract.md)
