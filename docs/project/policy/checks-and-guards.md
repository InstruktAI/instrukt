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
  to the operator; a part whose caller's wait is bounded retries within that
  bound and then gives its caller the wait, a bound of nothing being a single
  try. The exceptions are the best-effort unavailable notices governed by
  `project/instrukt/spec/feature/company/conversations` and dead-letter
  announcements governed by `project/instrukt/spec/feature/runner/event-consumers`. Any other failure of a
  part is a defect to fix in the code.
- A part designed or changed meets the rule above through the outside gate
  (`project/instrukt/design/architecture/outside-gate`): it talks to its service
  through an established off-the-shelf SDK, protocol client, or command-line
  program (`sdlc/policy/substrate-anchoring`) — adopting one rather than keeping
  or extending hand-written protocol requests, and never replacing a fitting
  client with hand-written requests to suit the outside gate; it brings that client's outcome to the
  outside gate, which alone judges the failure, decides the wait and reports it,
  so the part keeps no backoff schedule, `Retry-After` reading or rate-limit
  memory of its own; it keeps exactly one retry layer, a client that retries by
  itself being configured not to or brought through the outside gate as a
  single try; and it states its own settings for its calls, which the outside
  gate honours.
- A person writing to a company's agent may be told about their request and about
  their own world — a service of theirs that is unreachable, the assistant being
  unavailable for a while — and never about instrukt's internals. What instrukt
  cannot resolve goes to the operator. Each object generation uses a fixed native start key and thread ID; retries
  replay that identity. Outstanding work after confirmed native archive or deletion,
  or after an idle thread is confirmed to belong to a runtime project other than its
  checkout's own, uses a new generation automatically; a misplaced thread that is
  still busy is left alone until it is idle. Native lifecycle owns provider
  detachment; neither a create receipt nor a misplacement rollover supplies a
  physical-process exit guarantee.
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
- One mechanism makes the judgment once: parts that each read their service's
  answers and pick their own pauses drift apart, retry what is not theirs to
  retry and report the same outage in as many places.
- An established client already owns its service's protocol, auth, and edge
  cases; keeping or writing hand-rolled requests trades proven handling for new
  defects.
- Two retry layers multiply each other's tries and hide the outage from the
  report.
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
