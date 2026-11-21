---
description: What instrukt adds to trust-and-guardrails — prerequisites guarded at the gate, which places are instrukt's own, how parts that depend on outside services behave, what a person in a conversation may be told, and when a record may be discarded.
---

# Checks and Guards — Policy

## Required reads

- [general/principle/trust-and-guardrails](../../general/principle/trust-and-guardrails.md)
- [sdlc/policy/state-storage](../../domain/sdlc/policy/state-storage.md)

## Rules

- A prerequisite — configuration, a credential, a published procedure, an
  installed service — is checked where it enters: where it is written, at install,
  and when the runner starts or takes a change. Nothing running handles a missing
  prerequisite. Such a check at the entry, refusing with its door, is a gate in the
  sense of trust-and-guardrails; a proof of ownership, a version gate on rebuildable
  state or a refusal of instrukt's own files is not one.
- instrukt's own places are the files, service units, crontab line, private Redis
  configuration and marked hook sections it generates, each under a name that
  carries its home when the home is not `~/.instrukt`. instrukt writes them whole
  when it converges, without consulting any record of an earlier write, and removes
  nothing because it remembers having written it.
- A part that depends on a service outside instrukt — the runtime, Slack, Redis,
  the network, a model provider — retries until it returns and reports its state
  to the operator, except for the best-effort unavailable notices governed by
  `project/instrukt/spec/feature/company/conversations`. Any other failure of a
  part is a defect to fix in the code.
- A person writing to a company's agent may be told about their request and about
  their own world — a service of theirs that is unreachable, the assistant being
  unavailable for a while — and never about instrukt's internals. What instrukt
  cannot resolve goes to the operator. Each object generation uses a fixed native start key and thread ID; retries
  replay that identity. Outstanding work after confirmed native archive or deletion
  uses a new generation automatically. Native lifecycle owns provider detachment;
  a create receipt supplies no physical-process exit guarantee.
- A record in an older or unknown shape is discarded only when its specification
  names the source that rebuilds it. A person's stated choices and restrictions,
  and records of external effects, are migrated, never discarded or expired.

## Rationale

- A prerequisite checked where it enters, and a running process that uses only
  what it admitted, leave no runtime path for a missing prerequisite; what a
  service outside instrukt does later is availability, not a missing prerequisite.
- A check on a place only instrukt writes protects nothing a person owns and fails
  exactly where installations differ.
- An outside service can leave at any moment; a defect cannot be retried away.
- A person can act on what belongs to their world and on nothing else.
- Discarding a record is safe exactly when what rebuilds it survives.

## Scope

instrukt's product behaviour: the runner, installation, hooks, company messaging
and command preconditions. The todo workflow's own rules follow their contracts.

## Enforcement

- Specification and code review apply these rules to the mechanisms a change adds
  or touches.

## Exceptions

- None.
