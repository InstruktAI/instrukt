---
description: The screening bounded context — capability-free calls to the configured OpenAI-compatible endpoint, the screening of untrusted text built on them, and the texts it holds for a person.
delivered_by:
  - capability-free-model-call
---
# Screening — Spec

## What it is

The screening context puts text in front of a model without lending that text
any capability, and decides whether untrusted text may reach a tool-capable
agent. A text's authority comes from the channel that brought it, never from
what it says: text written outside the project instructs no one, so a model
that reads it is given nothing to act with, and an agent working from
instrukt's briefs receives it only once screening released it. The context
owns the capability-free call to the endpoint the operator configures
(`project/instrukt/spec/feature/screening/capability-free-calls`), the
screening of untrusted text through such calls
(`project/instrukt/spec/feature/screening/untrusted-text-screening`), and the
texts it holds for a person. It knows nothing of where a text comes from or
what its caller does with the outcome.

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

**Waiting**
: The state of a call or screening whose endpoint is unavailable: its command
  tries again until the endpoint answers, and neither refuses, releases nor
  holds anything because of the unavailability.

**Fragment**
: Between three and eight consecutive words of a text, words being the runs
  of characters between whitespace.

**Shuffle**
: The fragments of a text in a random order that differs from their order in
  the text.

**Classification**
: One capability-free call whose prompt is screening's fixed instruction
  followed by the text's shuffle as a JSON array of strings. Each screening
  runs two, one after the other, over one shuffle.

**Verdict**
: A classification's answer: `benign` with no reason, or `flagged` with one or
  more reasons from `instructs_reader`, `claims_authority`,
  `seeks_secrets_or_data`, `seeks_actions`, `hides_content` and
  `other_manipulation`. A `benign` verdict that carries a reason is not of the
  verdict's shape.

**Size screening judges**
: From six words, the fewest that make two fragments, to
  `screening.max_text_chars` characters.

**Held text**
: The complete text of a screening that did not release it, kept with its
  verdicts and the reasons it is held, under a reference.

**Reference**
: A held text's identity: 128 random bits written as 32 lowercase hexadecimal
  characters.

**Brief-governed agent**
: An agent instrukt started or assigned through the selected runtime. Its
  runtime gives it a caller identity, which the CLI recognises as a session.

**Caller outside any session**
: A caller without that identity: a person at their terminal, or an agent the
  operator drives directly, which is the operator's own.

## Ownership boundary

- **Owns:** the request a capability-free call sends and what makes an answer
  acceptable; which endpoint answers are unavailability and which are
  rejection; waiting and trying again; fragments, the shuffle, the
  classification instruction and count, the verdict vocabulary and the
  release-or-hold decision; held texts, their store and lifecycle; that a held
  text reaches only a caller outside any session, and that only such a caller
  discards one.
- **Does not own:** the `screening` settings' layer, validation and secret
  resolution (`project/instrukt/spec/domain/configuration`); recognising the
  caller, which the CLI does through core's caller-identity lookup and passes
  to screening as a session or a caller outside any session; which text is
  untrusted, who is outside and bringing a held text before a responsible
  person, which belong to each consumer; command admission and its action
  catalog (`project/instrukt/spec/feature/configuration/roles-and-capabilities`).
- **Published contracts:** `instrukt screening call --shape <file>
  [--max-tokens <n>] <prompt-file|->`, `instrukt screening screen
  <file|->`, `instrukt screening show <reference>`, `instrukt screening list`
  and `instrukt screening discard <reference>`, each with `--json`; the call
  and the screening as functions of `instrukt-screening`, each taking the
  caller and returning its outcome when the endpoint has answered.
- **Allowed dependencies:** configuration supplies the effective `screening`
  settings and resolves the key reference; core supplies the caller-identity
  lookup the CLI reads. The relationships are in the context map position
  below.
- **Implementation roots:** `crates/instrukt-screening/`, which depends on
  `instrukt-core`; `instrukt-cli` wires it.

## Canonical fields

### Aggregates

#### Held text

- **Root entity:** the held text.
- **Contains:** the complete text; each verdict given, with its reasons; the
  reasons it is held (`flagged`, `not_of_shape`, `cut_off`, `rejected`,
  `outside_size`); when it was held; when and whether a person discarded it.
- **Invariants:**
  - The complete text is kept as handed over, never truncated.
  - Only a caller outside any session receives the text. A session receives
    the reference, the verdicts and the reasons, never the text.
  - Only a caller outside any session discards it. A discarded held text stays
    readable by such a caller until it is purged.
  - Screening releases no held text.
- **Identifier:** the reference.
- **Lifecycle:** written by `instrukt screening screen` before it replies
  `held`; current until a caller outside any session discards it; a discarded
  held text is purged thirty days after it was discarded, by the next
  `instrukt screening` command that writes the store.

### What a caller waits for

A call or screening returns its outcome once the endpoint has answered. While
the endpoint is unavailable the command waits: it tries again after 5 seconds,
doubling the pause up to 60 seconds, for as long as it runs. Each try waits at
most `screening.timeout`. Standard error shows that it waits, why and since
when, and the log records when it starts waiting and when the endpoint answers
again. A caller that stops the command while it waits receives nothing, and
nothing of its text is kept.

| Operation | Returns |
| --- | --- |
| `screening call` | `answered` with the answer, or `refused` with its reason |
| `screening screen` | `released` with the text, or `held` with the verdicts, the reasons and the reference |
| `screening show` | a held text's reference, verdicts, reasons and dates, and its text to a caller outside any session |
| `screening list` | every held text that is not discarded, without its text |
| `screening discard` | the discarded held text's reference, to a caller outside any session |

A caller whose own wait is shorter than an outage runs the command as
background work of its runtime, which tells it when the command ends. Code
that calls the functions in instrukt's process calls them outside every lock
another operation waits on.

### Entry checks

Before a call or screening takes its input, its command refuses, naming what
is missing and the door, when:

- the effective configuration names no `screening.url` or no
  `screening.model`;
- `screening.api_key` is set and its secret does not resolve
  (`project/instrukt/spec/feature/configuration/owned-files`);
- the answer shape is not a JSON Schema object, the answer bound is outside 1
  to 4096, or the prompt or text is empty.

A nonempty text outside the size screening judges is held, not refused. A
running command uses the settings it admitted.

### Held-text store

| Property | Value |
| --- | --- |
| Holds | Each held text with its reference, verdicts, reasons and dates; discarded held texts in a table of their own until purged. |
| Nature | Authoritative: the only copy of each held text. |
| Owner | The screening context, written only by the `instrukt screening` commands: `screen` writes a held text, `discard` moves one to the discarded table, and a command that writes the store purges discarded held texts past thirty days. |
| Loss cost | Lost intent: a held text lost drops an outside contribution. |
| Medium | `state/screening.db`, a SQLite database of its own with its own migrations: held texts grow without a fixed bound and change from held to discarded. |
| Write isolation | Its own lock, `state/screening.lock`, taken only by the `instrukt screening` commands for each transaction, never while a request to the endpoint is under way. No background writer takes it, and no other context's state is in it. Readers take no lock. |
| Scope | One computer: the selected home's `state/`. A held text moves with no responsibility between computers. |
| Lifecycle | Held texts are current until discarded; discarded ones are history, purged thirty days after discard. A held text of an earlier schema is migrated, never discarded. |
| Rebuild | None: nothing re-creates the store. |

A call or screening that is answered, refused or released writes nothing, and
a waiting one keeps its input only in its own process.

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

- `project/instrukt/design/architecture/overview` — the crate, the held-text
  store in the home and the model endpoint as an external surface.

## Context map position

- **configuration → screening — Customer / Supplier:** the global-only
  `screening` settings and the resolution of `screening.api_key` from the
  machine's secrets.
- **CLI admission → screening — Customer / Supplier:** the caller the CLI
  recognised through core's caller-identity lookup, passed as a session or a
  caller outside any session.
- **screening → CLI admission — Published language:** the action `screening`
  in the code-owned catalog.

## Known caveats

- The held-text rule keeps instrukt from handing held or unscreened outside
  text to brief-governed agents. Agents are trusted; the rule protects them
  from receiving alien instructions through instrukt, and is no defence
  against an agent. An agent the operator drives directly is the operator's
  own and receives what the operator would.
- Two classifications of one shuffle by one model are separate calls, not
  different judges.
- A text of fewer than six words, or in a script that does not separate words
  with whitespace, makes fewer than two fragments and is held.
- An endpoint that cuts a long prompt to fit its model's context judges part
  of it. `screening.max_text_chars` is set below what the model takes.
- A request the endpoint never answers within `screening.timeout` waits for as
  long as its command runs; its caller sees why and stops it, or the operator
  raises the timeout.
