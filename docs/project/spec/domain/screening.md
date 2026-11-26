---
description: The screening bounded context — capability-free calls to the configured OpenAI-compatible endpoint, the screening of untrusted text built on them, and the requests it keeps while the endpoint is unavailable or while a text is held for a person.
delivered_by:
  - capability-free-model-call
---
# Screening — Spec

## What it is

The screening context puts text in front of a model without lending that text
any capability, and decides whether untrusted text may reach a tool-capable
agent. It owns the capability-free call to the endpoint the operator
configures (`project/instrukt/spec/feature/screening/capability-free-calls`),
the screening of untrusted text through such calls
(`project/instrukt/spec/feature/screening/untrusted-text-screening`), and the
requests it keeps: those waiting on an unavailable endpoint and those holding a
text for a person. It knows nothing of where a text comes from or what its
caller does with the outcome.

## Ubiquitous language

**Capability-free call**
: One chat-completions request to the endpoint that defines no tools and
  carries one user message, the prompt, with a strict JSON-schema response
  format and a cap on answer tokens.

**Endpoint**
: The OpenAI-compatible chat-completions service and the model the machine's
  `screening` configuration names.

**Answer shape**
: The JSON Schema a caller declares. An answer has the shape when the model
  finished, its content is JSON and that JSON is valid against the schema.

**Answer bound**
: The most tokens an answer may take: 1024 unless the caller names another,
  at most 4096.

**Unavailable**
: The endpoint cannot be reached, gives no complete answer within
  `screening.timeout`, or answers with an unsuccessful status other than 400,
  413 and 422. A 401, 403, 404, 429 or 5xx is unavailability: a rejected key,
  an unknown model, a rate, quota or spend limit, an overload.

**Rejected**
: The endpoint answers 400, 413 or 422, as for a request too large for the
  model. A rejected request has no answer of its shape: a call is refused and
  a screening holds the text.

**Fragment**
: Between three and eight consecutive words of a text, words being the runs
  of characters between whitespace. A text of fewer than three words has no
  fragment.

**Classification**
: One capability-free call whose prompt is screening's fixed instruction
  followed by the text's fragments, shuffled, as a JSON array of strings. Each
  screening runs two, one after the other, over one shuffle.

**Verdict**
: A classification's answer: `benign` with no reason, or `flagged` with one or
  more reasons from `instructs_reader`, `claims_authority`,
  `seeks_secrets_or_data`, `seeks_actions`, `hides_content` and
  `other_manipulation`.

**Size screening judges**
: From three words to `screening.max_text_chars` characters.

**Request**
: A call or a screening screening keeps: one still waiting when its command's
  wait ends, or a screening that held its text.

**Reference**
: A request's identity: 128 random bits written as 32 lowercase hexadecimal
  characters. It is the receipt a waiting caller holds.

**Held text**
: The complete text of a held screening, kept with its verdicts and reasons
  for a person.

**Pending pass**
: The pass that tries waiting requests again and delivers their outcomes. The
  runner hosts it in every invocation.

**Delivery**
: Sending a settled request's outcome, as one input, to the session that
  asked for it.

## Ownership boundary

- **Owns:** the request a capability-free call sends and what makes an answer
  acceptable; which endpoint answers are unavailability and which are
  rejection; fragments, the classification instruction and count, the verdict
  vocabulary and the release-or-hold decision; requests, their store and
  their lifecycle; that held text reaches only a caller outside any session,
  and that only such a caller discards a request; that a settled outcome is
  delivered to the session that asked, and its form.
- **Does not own:** the `screening` settings' layer, validation and secret
  resolution (`project/instrukt/spec/domain/configuration`); which caller is a
  session, the runtime adapter's caller identity read through core's
  caller-identity lookup; sending input to a session, the runtime's acceptance
  of a command identity and the scheduled wake
  (`project/instrukt/spec/domain/runner`); which text is untrusted, who is
  outside and bringing a held text before a responsible person, which belong
  to each consumer; command admission and its action catalog
  (`project/instrukt/spec/feature/configuration/roles-and-capabilities`).
- **Published contracts:** `instrukt screening call --shape <file>
  [--max-tokens <n>] <prompt-file|->`, `instrukt screening screen
  <file|->`, `instrukt screening show <reference>`, `instrukt screening list`
  and `instrukt screening discard <reference>`, each with `--json`; the call
  and the screening as functions of `instrukt-screening` for code that runs
  in instrukt's own process; the pending pass, implementing the interface the
  runner declares; the admission action `screening`.
- **Allowed dependencies:** configuration supplies the effective `screening`
  settings and resolves the key reference; the runner supplies caller
  recognition, delivery to a session under a command identity and whether
  this computer's scheduled wake is installed, and hosts the pending pass. The
  relationships are in the context map position below.
- **Implementation roots:** `crates/instrukt-screening/`, which depends on
  `instrukt-runner` and `instrukt-core`; `instrukt-cli` wires it.

## Canonical fields

### Aggregates

#### Request

- **Root entity:** the request.
- **Contains:** its kind, `call` or `screening`; its input, a prompt with its
  answer shape and answer bound, or a text; the caller's session, absent for a
  caller outside any session; its status, `waiting` (a call) or `pending` (a
  screening), `answered`, `refused`, `released` or `held`; for a waiting or
  pending request the latest unavailability and since when; for a held
  screening its verdicts and the reasons it is held; for a settled request its
  outcome; the delivery's command identity and whether the runtime accepted
  it or the session no longer exists.
- **Invariants:**
  - A held request keeps its complete text. Only a caller outside any session
    receives that text, and only such a caller's discard removes the request.
  - A request settles once. A pass or command that settles a request another
    already settled leaves it as it is.
  - A delivery's command identity is recorded in the transaction that settles
    the request, before anything is sent, and every attempt to deliver it
    carries that identity.
- **Identifier:** the reference.
- **Lifecycle:** a request is created when its command's wait ends with the
  endpoint unavailable, or when a screening holds its text. A call or
  screening that is answered, refused or released within its command's wait
  leaves no request. The pending pass settles a waiting or pending request. A
  held request lasts until a caller outside any session discards it, which a
  waiting or pending request also allows. A settled request that is not held is
  retired by the pending pass seven days after it settled, and, when it has a
  session, only once its delivery was accepted or the session no longer
  exists.

### What a caller waits for

| Operation | Waits for | Returns |
| --- | --- | --- |
| `screening call` | one request to the endpoint, at most `screening.timeout` | `answered` with the answer, `refused` with its reason, or `waiting` with its reference and the unavailability |
| `screening screen` | up to two classifications, one after the other, each at most `screening.timeout`; a flagged or rejected classification ends it | `released` with the text, `held` with the verdicts, reasons and reference, or `pending` with its reference and the unavailability |
| `screening show`, `list`, `discard` | the request store alone | the request or requests; held text only outside any session |

No command waits on the pending pass, and no transaction on the request store
is open while a request to the endpoint or a delivery is under way.

A waiting caller's reply names its reference, the unavailability, and where
the outcome arrives: as an input to the session that asked, for a caller in a
session, and in `instrukt screening show <reference>` for every caller.

### Entry checks

Before a call or screening takes its input, its command refuses, naming what
is missing and the door, when:

- the effective configuration names no `screening.url` or no
  `screening.model`;
- `screening.api_key` is set and its secret does not resolve
  (`project/instrukt/spec/feature/configuration/owned-files`);
- this computer has no scheduled wake installed (`instrukt ticks status`), the
  door being `instrukt install`;
- the answer shape is not a JSON Schema object, the answer bound is outside 1
  to 4096, or the prompt or text is empty.

A text outside the size screening judges is held, not refused.

### Request store

| Property | Value |
| --- | --- |
| Holds | Each request with its reference, kind, input, caller's session, status and its facts, outcome, delivery identity and delivery state, and when it was created, last tried and settled. |
| Nature | Authoritative: the only copy of each held text and of each waiting caller's request. |
| Owner | The screening context. Its writers are the `screening call` and `screening screen` commands, which create requests, `screening discard`, which removes one, and the pending pass, which settles requests, records deliveries and retires settled requests. |
| Loss cost | A held text lost drops an outside contribution, and a waiting request lost leaves its caller without its outcome: lost intent. A lost delivery record would repeat a delivery the runtime already accepted, which the command identity prevents. |
| Medium | `state/screening.db`, a SQLite database of its own with its own migrations: held texts grow without a fixed bound, a request's status changes in place, and two kinds of process write it. |
| Write isolation | Its own lock, `state/screening.lock`, held for each transaction and never across a request to the endpoint, a delivery or another store's lock. Every transaction writes one request. The pending pass, a background writer, shares this unit with the commands, whose wait it can lengthen by one such transaction. No other context's state is in it. |
| Recording | A request is written once its command's wait has ended, never before an endpoint is asked. A settlement records the delivery's command identity in the same transaction. |
| Scope | One computer: the selected home's `state/`. A request moves with no responsibility between computers. |
| Lifecycle | Held requests are current until discarded; waiting and pending requests until settled or discarded; settled requests that are not held are retired as the Request lifecycle says. A request of an earlier schema is migrated, never discarded. |
| Rebuild | None: nothing re-creates the store. |

### Domain events emitted

None.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events.

## Use cases

- **capability-free-calls** — `project/instrukt/spec/feature/screening/capability-free-calls` — put outside text in front of a model whose request defines no tools, and receive one answer of a declared shape.
- **untrusted-text-screening** — `project/instrukt/spec/feature/screening/untrusted-text-screening` — release untrusted text only on every classifier's benign verdict over shuffled fragments, and keep any other text for a person.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — the crate, the request
  store in the home, the model endpoint as an external surface and the
  pending pass in each runner invocation.

## Context map position

- **configuration → screening — Customer / Supplier:** the global-only
  `screening` settings and the resolution of `screening.api_key` from the
  machine's secrets.
- **runner → screening — Customer / Supplier:** caller recognition through
  core's caller-identity lookup, delivery of one input to a named session
  under a command identity, and whether this computer's scheduled wake is
  installed.
- **screening → runner — Customer / Supplier:** the pending pass, declared by
  the runner, implemented by screening and wired by the CLI.
- **screening → CLI admission — Published language:** the action `screening`
  in the code-owned catalog.

## Known caveats

- Held text is withheld from a caller the runtime recognises as a session.
  An agent that removes that recognition from its own environment, or reads
  the operator's files directly, is not stopped: the rule keeps instrukt's
  surfaces from handing held text to an agent, and confines no agent working
  under the operator's own account.
- Two classifications of one shuffle by one model are separate calls, not
  different judges.
- A text of fewer than three words, or in a script that does not separate
  words with whitespace, has no fragments of three words and is held.
- An endpoint that cuts a long prompt to fit its model's context judges part
  of it. `screening.max_text_chars` is set below what the model takes.
- An endpoint that never answers within `screening.timeout` keeps its
  requests waiting. The operator sees why in `instrukt screening list`, and
  raises the timeout or discards them.
