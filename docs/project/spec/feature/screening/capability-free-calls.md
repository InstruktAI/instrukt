---
description: Internal one-off model calls to a configured OpenAI-compatible endpoint whose request defines no tools, so the model can change its bounded text answer and nothing else.
delivered_by:
  - capability-free-model-call
---
# Capability-Free Calls — Spec

## What it is

A capability-free call is an internal instrukt client that puts one prompt in
front of a model and returns one bounded text answer. instrukt makes it as a
single chat-completions request to an OpenAI-compatible endpoint with `tools`
omitted. The model receives text and returns text: it has no tools, no files,
no shell and no session that outlives the request. A prompt that manipulates
the model can change the answer and nothing else, so an internal caller can put
text written outside the project in front of a model without giving that text
any capability.

The call is an internal capability, not a standalone prompt-forwarding CLI
command. Untrusted-text screening
(`project/instrukt/spec/feature/screening/untrusted-text-screening`) is built
on it, and other internal callers — such as outside pull-request assessment or
future internal features — use it the same way.

The endpoint is the operator's choice, configured explicitly per purpose in
global configuration under the `llm` map — starting with `llm.screening` — so
an operator can pick a local server such as LM Studio or a cloud provider's
endpoint for each purpose, and share provider credentials across entries
through the same secret reference. The capability-free guarantee holds for a
plain model, one with no built-in search or browsing that runs nothing beyond
what the request asks. instrukt does not check which model is configured: the
configuration's description and the setup documentation say what to bring, and
a model that searches on its own is outside the guarantee. An endpoint's API
key, when it needs one, is a protected instrukt secret referenced from
configuration (`project/instrukt/spec/feature/configuration/owned-files`),
never a value in a configuration document. Where agent runtimes keep their
model-provider keys belongs to
`project/instrukt/spec/feature/configuration/agent-selection`.

The call carries no knowledge of where its prompt came from or what its answer
is for.

## Canonical fields

### A call

| Field | Meaning |
| --- | --- |
| Prompt | The complete text the model receives. It is the call's only input. |
| Answer | One bounded text answer returned by the model. |
| Outcome | `answered` with the bounded text answer, or `refused` with its reason. While the endpoint is unavailable the call is `waiting` and returns no outcome yet. |

The request omits `tools`, carries no earlier conversation and no data beyond
the prompt, and bounds the answer's size.

Before a call takes its prompt, any missing or unresolvable endpoint
prerequisite — an unconfigured or incomplete endpoint entry, or missing, blank,
unreadable or insufficiently protected referenced API key secret material
(`project/instrukt/spec/feature/configuration/owned-files`) — is `refused` at
entry, naming what is missing and the door to supply it
(`project/instrukt/policy/checks-and-guards`). A call is also `refused`, and
returns no answer, when the model stopped before finishing its answer because
the size bound ran out, or when the endpoint rejects the request as one it
cannot serve, such as one too large for the model. An answer is never repaired,
truncated or partly returned. A failure that is neither an entry refusal, an
unavailable endpoint nor one of these post-call refusals, such as a malformed
request or a client or protocol failure, is a defect
(`project/instrukt/policy/checks-and-guards`): the call fails without retrying
and returns no answer.

### The endpoint

| Setting | Meaning |
| --- | --- |
| Endpoint | The base address of an OpenAI-compatible chat-completions service, local or cloud, configured per purpose under global `llm` (starting with `llm.screening`). |
| Model | The model the endpoint serves the call with. |
| API key | Optional `${NAME}` reference to a protected instrukt secret; absent for an endpoint that needs none, and shareable across `llm.*` entries. |
| Timeout | Optional per-request time bound for one try against the endpoint. |

An endpoint that reports or shows that it cannot serve the request now is
unavailable, not a refusal: it cannot be reached, does not answer in time,
limits the rate, has exhausted its quota or spend limit, is overloaded, answers
with a server error, rejects the configured key or does not know the configured
model. The call is `waiting`, a state in which it returns no outcome yet:
instrukt, not the caller, tries it again through the outside gate
(`project/instrukt/design/architecture/outside-gate`), and the operator can see
that it waits and why (`project/instrukt/policy/checks-and-guards`). The caller
receives the eventual outcome without asking again.

## Use cases

### UC-CF1: An internal capability acting for the operator gets a bounded model answer without lending outside text any capability

```gherkin
Given an operator has configured an OpenAI-compatible endpoint and model for capability-free calls
And text written outside the project instructs the model to run a command, read the operator's files and send them to a web address
When an internal capability acting for the operator sends a prompt carrying that text through a capability-free call
Then it receives one bounded text answer
And the model was offered no tool, file, shell or earlier conversation
```

### UC-CF2: An operator serves capability-free calls from their local model

```gherkin
Given an operator runs an OpenAI-compatible model server on their own computer
When they configure its address and model for capability-free calls
Then later capability-free calls are answered by that model
And no API key is needed for it
But a call while the server is down waits, and the operator sees that it waits
```

## Known caveats

- A capability-free call bounds what a manipulated model can do; it does not
  make the answer trustworthy. A caller treats the answer as the judgment of a
  model that read untrusted text.
- A cloud endpoint receives the prompt, and so the untrusted text in it.
  Choosing a local endpoint keeps that text on the operator's computer.
