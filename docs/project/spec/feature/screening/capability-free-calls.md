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
on it, and other internal callers use it the same way.

The endpoint is chosen per purpose under global `llm` — such as
`llm.screening` — in
`project/instrukt/spec/feature/configuration/layered-configuration`, local or
cloud. The capability-free guarantee holds for a plain model, one with no
built-in search or browsing that runs nothing beyond what the request asks.
instrukt does not check which model is configured: the configuration's
description and the setup documentation say what to bring, and a model that
searches on its own is outside the guarantee. An endpoint that needs an API key
references a protected machine-local secret
(`project/instrukt/spec/feature/configuration/owned-files`); where agent
runtimes keep their model-provider keys belongs to
`project/instrukt/spec/feature/configuration/agent-selection`.

The call carries no knowledge of where its prompt came from or what its answer
is for.

## Canonical fields

### A call

| Field | Meaning |
| --- | --- |
| Prompt | The complete nonempty text the model receives. It is the call's only input. |
| Answer | One bounded text answer returned by the model. |
| Outcome | `answered` with the bounded text answer, `refused` with its reason, or `waiting` while the endpoint is unavailable (returning the outside gate's wait when a caller's wait bound expires). |

The request omits `tools`, carries no earlier conversation and no data beyond
the prompt, and bounds the answer's size.

Before a call sends a request, an empty or whitespace-only prompt, an
unconfigured, incomplete or invalid endpoint entry (including a malformed
address, an address embedding credentials, or a non-loopback plaintext `http`
endpoint carrying an API key), or a referenced API key secret that cannot be
resolved under `project/instrukt/spec/feature/configuration/owned-files` is
`refused` at entry (`general/principle/trust-and-guardrails`), naming the
refusal and its door without disclosing secret or embedded credential material. A call is also
`refused`, and returns no answer, when the model stopped before finishing its
answer because the size bound ran out, or when the endpoint rejects the request
as one it cannot serve, such as one too large for the model. An answer is never
repaired, truncated or partly returned. A failure that is neither an entry
refusal, an unavailable endpoint nor one of these post-call refusals — such as
a malformed request or a client or protocol failure — is a defect
(`sdlc/principle/design-fundamentals`): the call fails without retrying
and returns no answer.

### The endpoint

| Setting | Meaning |
| --- | --- |
| Endpoint | The base address of an OpenAI-compatible chat-completions service, local or cloud, configured per purpose under global `llm`, such as `llm.screening` (`project/instrukt/spec/feature/configuration/layered-configuration`). Non-loopback endpoints carrying an API key require `https`, and endpoint addresses never embed credentials. |
| Model | The model the endpoint serves the call with. |
| API key | Optional secret reference resolved from the machine's secrets (`project/instrukt/spec/feature/configuration/owned-files`); absent for an endpoint that needs none. |
| Timeout | Optional per-request time bound for one try against the endpoint. |

Every try to the endpoint runs through the outside gate
(`project/instrukt/design/architecture/outside-gate`), which judges temporary
unavailability, paces retries and reports waits. While the endpoint is
unavailable, the call is `waiting`: an unbounded caller waits until the
endpoint answers, while a caller with a bounded wait retries within that bound
and receives the wait, a bound of nothing being a single try
(`project/instrukt/design/architecture/outside-gate`), with no `answered` outcome.

## Use cases

### UC-CF2: An operator serves capability-free calls from their local model

```gherkin
Given an operator runs an OpenAI-compatible model server on their own computer
When they configure its address and model for capability-free calls
Then later capability-free calls are answered by that model
And no API key is needed for it
But a call while the server is down waits
```

## Known caveats

- A capability-free call bounds what a manipulated model can do; it does not
  make the answer trustworthy. A caller treats the answer as the judgment of a
  model that read untrusted text.
- A cloud endpoint receives the prompt, and so the untrusted text in it.
  Choosing a local endpoint keeps that text on the operator's computer.
