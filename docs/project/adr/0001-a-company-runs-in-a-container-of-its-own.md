---
visibility: public
number: 1
date: 2026-11-18
description: A company's agents run in one long-running Linux container per company, with T3 as their runtime inside it and the agent tools installed outside the image, instead of on the operator's computer as the operator.
---

# A Company Runs in a Container of Its Own — ADR

## Context

A company's agent answered its people through T3 on the operator's own
computer, under the operator's operating-system user. Everything that user
can read, the agent could read: the operator's files, every other company's
home with its secrets, transcripts and logins, and the state database with
every signed-in login. The procedure that bounds the agent is an instruction
in prose, not confinement. The owner accepted that only until there was
isolation, and set the order: the agent in use now, then the separation of
company code from the rest of instrukt, then a container per company, then
channels open to people outside the company.

Three forces were in play. Companies must be kept apart from each other and
from the host, with their transcripts and logins separate. A company's owner
should reach their own company's agents without the operator's credentials.
And one host should carry several companies at little cost, growing by a
larger machine rather than by an orchestration platform.

## Decision

- Each company runs in one long-running Linux container of its own on the
  host. Its agents run only there.
- T3 is the runtime inside that container for the company's team agent, the
  same runtime instrukt already drives.
- The company's state lives on the host, in the company's folder, so that a
  container can be replaced without losing it.
- The image is lean. The agent tools are installed into the company's state
  by an install step and follow their own releases, without the image being
  rebuilt and without a version pinned by hand per company.
- The host is Linux; a Mac running Docker runs the same container for
  testing.

## Consequences

- A company's agent can reach only what its container holds and what the
  network lets it reach. Inside its container it is not confined further: T3
  cannot take tools away from a session, so the team agent can still change
  or erase anything of its own company's in the container. Restoring from
  backup answers that, not prevention.
- Every company costs one T3 server and one long-running container on the
  host.
- The company's folder holds secrets (T3's server keys, the agent program's
  login), so whatever copies it, a backup or a move, carries secrets.
- An agent that must not have a shell or write tools, as a help desk open to
  outsiders needs, is not served by this decision; it needs its own way to be
  started.
- How instrukt itself is split between the host and the container, how a
  message reaches a company's agent and how shared services are reached are
  left to the architecture that follows this decision.

## Alternatives Considered

- **The agent on the operator's computer as the operator.** Kept only until
  isolation exists: nothing separates a company from the operator or from
  another company.
- **One operating-system user per company on one host.** Separates files, but
  not network, processes or installed tools, and gives a company's owner no
  environment of their own to open.
- **A container without T3, with instrukt starting the agent program
  directly.** Weighed the same week: it allows taking tools away from an
  agent, which T3 does not. It was set aside for the team agent because T3
  gives the company's owner their own environment and is the runtime
  instrukt already drives; it stays the path for agents that must have no
  tools.
- **A versioned image with the tools inside.** Every tool release would need
  a new image and a recreated container per company.
- **Kubernetes or a container per agent.** More cost and operation than a
  handful of companies on one host needs.
