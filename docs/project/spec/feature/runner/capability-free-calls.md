---
description: One-off model calls through the agent CLIs a person is signed in to, in which the model can change its answer and nothing else, served only by CLIs admitted on evidence and confined by the operating system.
delivered_by:
  - capability-free-model-call
---
# Capability-Free Calls — Spec

## What it is

A capability-free call puts one prompt in front of a model and returns one
answer of a shape the caller declares. During the call the model has no tools,
reads and writes nothing but a private working folder that ends with the call,
reaches no network host other than its own model service, holds no credential
other than the sign-in of the CLI serving the call, and leaves no session
behind. A prompt that manipulates the model can change the answer and nothing
else, so a caller can put text written outside the project in front of a model
without giving that text any capability.

Calls run through the agent CLIs a person is already signed in to on their
computer: Antigravity's `agy`, Claude Code's `claude`, Codex's `codex`, `pi`
and `opencode`. They need no model-provider key of instrukt's. A CLI serves
calls only after its admission has shown that it cannot be made to use a tool,
read outside its folder or reach the network, and the operating system confines
every call as well, so the guarantee does not rest on a CLI's own settings.

The call carries no knowledge of where its prompt came from or what its answer
is for. Untrusted-text screening
(`project/instrukt/spec/feature/runner/untrusted-text-screening`) is built on
it; any other caller uses it the same way.

## Canonical fields

### A call

| Field | Meaning |
| --- | --- |
| Prompt | The complete text the model receives. It is the call's only input. |
| Answer shape | The fixed shape the caller declares the answer must have. |
| Answer | One answer of the declared shape, within the call's size and time bounds. |
| Outcome | `answered` with the answer, or `refused` with its reason. |

A call is refused, and returns no answer, when no admitted CLI is available on
the computer, when its confinement cannot be established, when the model
service cannot be reached, when the answer does not have the declared shape,
or when the answer exceeds its size bound or the call its time bound. An
answer is never repaired, truncated or partially returned.

### What the model can reach

| Reach | During a call |
| --- | --- |
| Tools | None, built-in or added by extension, plugin or MCP server. |
| Files | Its private working folder only, created for the call and removed after it. The person's home, projects and configuration are out of reach. |
| Network | Its own model service only. |
| Credentials | The serving CLI's own sign-in only; no other environment value, key or secret of the person or of instrukt. |
| Session | None retained. No conversation, history or transcript of the call remains in the CLI's records. |

### Admission

A CLI is admitted per computer and per installed version. Admission runs the
CLI under exactly the confinement and settings a call uses and prompts it to
use a tool, to read a file outside its folder and to reach a network host other
than its model service. The CLI is admitted only when every attempt fails and
it still returns an answer of the requested shape.

A CLI that offers the model any tool in its call mode is refused even when
confinement would contain that tool's effect, because the model has no tools
during a call. A CLI is likewise refused when it has no non-interactive mode,
when it is not signed in, or when any attempt succeeds. A refusal names what
is missing. A CLI that is not installed is reported as absent.

A CLI whose installed version is not the admitted one serves no call until it
is admitted again. Admission takes place before a CLI serves calls, never
during a call.

### Confinement

The operating system confines every call to the reach above, independently of
the CLI, on each computer instrukt is released for. On a computer where that
confinement cannot be established, no CLI is admitted and no call runs, and the
reason is reported.

## Use cases

### UC-CF1: An agent gets a model's judgment of outside text without lending that text any capability

```gherkin
Given an operator's computer with an admitted agent CLI the operator is signed in to
And text written outside the project that instructs the model to run a command, read the operator's files and send them to a web address
When an agent acting for the operator asks for a capability-free call with that text and an answer shape
Then the agent receives one answer of that shape
And no command runs, no file outside the call's folder is read or changed and no host other than the model service is reached
And no session of the call remains in the CLI's records
```

### UC-CF2: An operator learns which of their signed-in CLIs serve capability-free calls

```gherkin
Given an operator signed in to several agent CLIs
When admission has run on their computer
Then each CLI that passed is shown as admitted for its installed version
And each CLI that failed is shown as refused with what is missing
And a refused CLI serves no call
```

## Known caveats

- A capability-free call bounds what a manipulated model can do; it does not
  make the answer trustworthy. A caller treats the answer as the judgment of a
  model that read untrusted text.
