---
source_sha256: 03620da129758c127db7963ece315d8468303b7b5f53640b393728ededf1a7f0
model: claude-opus-5-5
---
- a prerequisite (configuration, credential, published procedure, installed service) is checked where it enters: when written, at install, when the runner starts or takes a change; nothing running handles a missing one. Such an entry check refusing with its door is a gate; proof of ownership, version gate on rebuildable state or refusal of instrukt's own files is not.
- instrukt's own places (generated files, service units, crontab line, private Redis config, marked hook sections, each named with its home when not `~/.instrukt`) are written whole on converge, consulting no record of earlier writes; NEVER removed because remembered as written.
- a part depending on an outside service (runtime, Slack, Redis, network, model provider) retries until it returns and reports state to the operator, except best-effort unavailable notices and dead-letter announcements per their specs; any other failure of a part = a defect to fix in code.
- a person writing to a company's agent may hear about their request and their own world (their service unreachable, the assistant unavailable a while), NEVER instrukt internals; what instrukt cannot resolve goes to the operator.
- each object generation uses a fixed native start key and thread ID; retries replay it; outstanding work after confirmed native archive or deletion gets a new generation automatically; native lifecycle owns provider detachment; a create receipt guarantees no process exit.
- a record of older or unknown shape is discarded only when its spec names the source that rebuilds it; a person's stated choices and restrictions and records of external effects are migrated, NEVER discarded or expired.
- applies to runner, installation, hooks, company messaging and command preconditions; spec and code review enforce it on mechanisms a change adds or touches.
