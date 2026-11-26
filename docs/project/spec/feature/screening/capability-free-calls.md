---
description: One-off model calls to a configured OpenAI-compatible endpoint whose request defines no tools, so the model can change its answer and nothing else.
delivered_by:
  - capability-free-model-call
---
# Capability-Free Calls — Spec

## What it is

A capability-free call puts one prompt in front of a model and returns one
answer of a shape the caller declares. instrukt makes it as a single
chat-completions request to an OpenAI-compatible endpoint, and that request
defines no tools. The model receives text and returns text: it has no tools,
no files, no shell and no session that outlives the request. A prompt that
manipulates the model can change the answer and nothing else, so a caller can
put text written outside the project in front of a model without giving that
text any capability.

The endpoint is the operator's choice, made in configuration: a local server
such as LM Studio, or a cloud provider's endpoint. The endpoint must run only
what the request asks: a model that invokes tools of its own accord, such as a
search model that searches before it answers, is not a supported endpoint for
these calls. Its API key, when it needs one, is a protected instrukt secret
(`project/instrukt/spec/feature/configuration/owned-files`), never a value in a
configuration document; agent runtimes keep their model-provider keys in T3.

The call carries no knowledge of where its prompt came from or what its answer
is for. Untrusted-text screening
(`project/instrukt/spec/feature/screening/untrusted-text-screening`) is built
on it, and any other caller uses it the same way.

## Canonical fields

### A call

| Field | Meaning |
| --- | --- |
| Prompt | The complete text the model receives. It is the call's only input. |
| Answer shape | The fixed shape the caller declares the answer must have. |
| Answer | One answer of the declared shape, within the call's size bound. |
| Outcome | `answered` with the answer, `waiting` while the endpoint is unavailable, or `refused` with its reason. |

The request defines no tools, carries no earlier conversation and no data
beyond the prompt, and asks only for an answer of the declared shape within its
size bound.

A call is refused, and returns no answer, when no endpoint is configured, when
the answer does not have the declared shape, or when the model stopped before
finishing its answer because the size bound ran out. An answer is never
repaired, truncated or partly returned.

### The endpoint

| Setting | Meaning |
| --- | --- |
| Endpoint | The base address of an OpenAI-compatible chat-completions service, local or cloud. |
| Model | The model the endpoint serves the call with. |
| API key | A reference to a protected instrukt secret; absent for an endpoint that needs none. |

An endpoint that reports or shows that it cannot serve the request now is
unavailable, not a refusal: it cannot be reached, does not answer in time,
limits the rate, has exhausted its quota or spend limit, is overloaded, answers
with a server error, rejects the configured key or does not know the configured
model. The call waits and instrukt, not the caller, tries it again; the
operator can see that it waits and why, and the caller receives the eventual
outcome without asking again (`project/instrukt/policy/checks-and-guards`).

## Use cases

### UC-CF1: An agent gets a model's judgment of outside text without lending that text any capability

```gherkin
Given an operator has configured an OpenAI-compatible endpoint and model for capability-free calls
And text written outside the project instructs the model to run a command, read the operator's files and send them to a web address
When an agent acting for the operator asks for a capability-free call with that text and an answer shape
Then the agent receives one answer of that shape
And the model was offered no tool, file, shell or earlier conversation
```

### UC-CF2: An operator serves capability-free calls from their local model

```gherkin
Given an operator runs an OpenAI-compatible model server on their own computer
When they configure its address and model for capability-free calls
Then later calls are answered by that model
And no API key is needed for it
But a call while the server is down waits, and the operator sees that it waits
```

## Known caveats

- A capability-free call bounds what a manipulated model can do; it does not
  make the answer trustworthy. A caller treats the answer as the judgment of a
  model that read untrusted text.
- A cloud endpoint receives the prompt, and so the untrusted text in it.
  Choosing a local endpoint keeps that text on the operator's computer.
