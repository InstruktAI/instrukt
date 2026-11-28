---
description: The screening bounded context — internal stateless capability-free calls to a configured OpenAI-compatible endpoint (`llm.screening`) and the stateless `instrukt is-safe` safety check over shuffled fragments.
delivered_by:
  - capability-free-model-call
---
# Screening — Spec

## What it is

The screening context puts untrusted text in front of a model without lending
that text any capability, and decides whether untrusted text is safe (free of
prompt injection or manipulation) before a brief-governed agent reads it. A
text's authority comes from the channel that brought it, never from what it
says: text written outside the project instructs no one, so a model that reads
it is given nothing to act with.

The context owns:

- the internal reusable capability-free model client
  (`project/instrukt/spec/feature/screening/capability-free-calls`), which sends
  a single tool-less chat-completions request to an OpenAI-compatible endpoint
  configured under global `llm` (starting with `llm.screening`) and returns one
  bounded text answer; and
- the stateless safety check (`instrukt is-safe` and its matching internal Rust
  function, `project/instrukt/spec/feature/screening/untrusted-text-screening`),
  which screens untrusted text as shuffled 3-to-8-word fragments through more
  than one independent capability-free check and returns a pure boolean verdict
  (`safe` or `unsafe` with a short bounded reason).

Screening is completely stateless: it creates and writes no local database,
lock file, retention state or duplicate copy of screened text, and exposes no
standalone raw prompt-forwarding CLI command. It knows nothing of where a text
comes from or what its caller does with an unsafe verdict.

## Ubiquitous language

**Capability-free call**
: One OpenAI-compatible `POST {url}/chat/completions` request that omits
  `tools` and carries a single `user` message (the prompt) with no earlier
  conversation, no other data and a `max_completion_tokens` cap on the answer.
  Internal Rust capability only; never exposed as a standalone prompt-forwarding
  CLI command.

**Endpoint**
: The OpenAI-compatible chat-completions service (`url`, `model`, optional
  `${NAME}` `api_key`, optional `timeout`) admitted from global
  `llm.screening` (`project/instrukt/spec/feature/configuration/layered-configuration`).

**Answer bound**
: The maximum number of completion tokens a capability-free call requests:
  default `1024`, allowed range `1` to `4096`.

**Unavailable**
: Transient endpoint or transport unavailability as judged by the outside gate's
  standard HTTP adapter (`project/instrukt/design/architecture/outside-gate`) —
  no custom screening service rules turn rejected credentials (`401`/`403`),
  spend/quota refusals (`402`) or unknown models (`404`) into waits.

**Endpoint rejection**
: The endpoint answers `400`, `413` or `422`, rejecting a request as one it
  cannot serve (such as one too large for the model). Not transient
  unavailability and not retried: an internal call returns `refused` and
  `is-safe` returns `unsafe` with a fixed short reason. Any other non-temporary
  `4xx` (including `401`, `402`, `403`, `404`), any `3xx` or non-`200` `2xx`,
  or any non-temporary client/TLS/protocol failure is a defect that fails
  without retrying.

**Waiting**
: The state of a call or screening while the outside gate paces retries against
  a temporarily unavailable endpoint; no `answered`, `safe` or `unsafe` outcome
  is produced meanwhile.

**Fragment**
: For nonempty text of six or more words (whitespace-delimited runs), a
  consecutive run of random length from 3 to 8 words summing to the text's
  words (at least two fragments). For nonempty text of one to five words, the
  complete text as a single fragment.

**Shuffle**
: For six or more words, the text's fragments in a CSPRNG random order
  guaranteed to differ from their original order, formatted as a JSON array of
  strings with no original positions. For one to five words, the single-element
  JSON array.

**Check**
: One independent capability-free call over the text's shuffle asking whether
  the fragments look manipulative or malicious, expecting a simple JSON verdict
  deserialized with `serde_json` (no generic JSON-Schema validator). Each
  screening runs two independent checks sequentially over one shuffle through
  the same outside gate.

**Verdict**
: A check's parsed JSON answer: `benign` (with no non-empty reason), or
  `flagged` with a bounded short non-empty `reason` summarizing the concern
  without quoting the screened text. Any `benign` verdict carrying a non-empty
  reason, any `flagged` verdict lacking a reason, or any unparseable answer is
  an invalid verdict and fails safe as `unsafe`. A model-authored `reason` is
  untrusted model output (a residual second-order prompt-injection surface)
  that tool-capable callers must treat as untrusted data, never instructions.

## Ownership boundary

- **Owns:** the capability-free chat-completions request shape and response
  admissibility (`200 OK` headers judged before reading the bounded 1 MiB body,
  `finish_reason == "stop"`, `400`/`413`/`422` endpoint rejection vs defect);
  3-to-8-word fragmentation, CSPRNG shuffling, classification prompt, two-check
  execution rule, simple verdict parsing and pure boolean safety decision;
  entry validation of prompts, screening text and answer bounds; and the
  stateless `instrukt is-safe` command semantics.
- **Does not own:** the `llm` / `llm.screening` configuration layer rules,
  schema validation, URL admission rules (`admitted_address` + no embedded
  credentials), withheld-only admission and `${NAME}` secret resolution
  (`project/instrukt/spec/domain/configuration`,
  `project/instrukt/spec/feature/configuration/layered-configuration`,
  `project/instrukt/spec/feature/configuration/owned-files`); standard HTTP
  unavailability judgment, `Retry-After` handling, backoff schedules, per-key
  wait state and one-report-per-wait operator reporting
  (`project/instrukt/design/architecture/outside-gate`), constructed by the
  host and handed down; command admission and its action catalog
  (`project/instrukt/spec/feature/configuration/roles-and-capabilities`);
  which external text is untrusted, who is an outside author, withholding
  unsafe text from brief-governed agents or pointing a responsible person at
  the source (each consumer's own context); or durable safety-verdict history
  (deferred to the domain event ledger).
- **Published contracts:**
  - CLI command: `instrukt is-safe [<file>] [--json]` (reads stdin by default
    or when `<file>` is `-`; reads `<file>` when supplied; stateless, read-only).
  - Internal Rust functions of `instrukt-screening`:
    - `call(&LlmAdmission, prompt, max_tokens, &Gate, CallerWait, stop)` ->
      `Result<CallOutcome, CallDefect>` (internal reusable client; no CLI).
    - `is_safe(&LlmAdmission, text, &Gate, CallerWait, stop)` ->
      `Result<SafeOutcome, CallDefect>` (in-memory safety check; neither
      function takes a `caller` parameter).
- **Allowed dependencies:** `instrukt-core` (global `llm.screening` admission
  with resolved `SecretValue`, `admitted_address` URL boundary) and
  `instrukt-outside` (`Gate::call_async`, `CallerWait`, `Key`, `Settings`,
  `http::build_client`, `http::judge_response`, `http::judge_reqwest_error`,
  `runtime::block_on`), with `Gate` constructed by the host and handed down.
- **Implementation roots:** `crates/instrukt-screening/`, which depends on
  `instrukt-core` and `instrukt-outside`; `instrukt-cli` wires
  `instrukt is-safe`, constructs the foreground `Gate` and suppresses
  third-party HTTP loggers (`project/instrukt/spec/feature/configuration/logging`).

## Canonical fields

### Aggregates

None. Screening is completely stateless and creates no local database, lock
file or durable state (`sdlc/policy/state-storage`). Endpoint configuration
and optional API key secrets are owned by configuration
(`project/instrukt/spec/domain/configuration`). Active endpoint waits are
transient in-process state inside the host-constructed `instrukt-outside::Gate`.

### What a caller waits for

Every HTTP try runs through a `reqwest::Client` built by
`instrukt_outside::http::build_client` with `follow_redirects: false`, a
10-second connect timeout, the admitted `llm.screening.timeout` per-request
timeout, and zero HTTP-client retries so `instrukt-outside::Gate` is the single
retry layer. Every try uses `Gate::call_async` with
`Key::new("screening", [host_port]).item(model)` (using the credential-free
`host[:port]` from `llm.screening.url`), `Settings::default()` (standard HTTP
unavailability rules and `Backoff::DEFAULT`), the caller's `CallerWait`
(`CallerWait::UntilReturns` for foreground `instrukt is-safe`, or a bounded
`CallerWait::Within(duration)` — including `Duration::ZERO` for a single try —
for bounded internal callers), and a `stop` check.

`instrukt-screening` checks `stop()` before every `Gate::call_async` call
(before try 1 and before check 2) and after `Gate::call_async` returns before
emitting any outcome. Foreground `instrukt is-safe` constructs a command-scoped
`Gate` with `Report::foreground(stderr)` (no process-wide global) and cancels
in-flight HTTP work on `SIGINT`/`SIGTERM` so an interrupted command emits
nothing on stdout and exits with the signal's status.

A caller whose own wait is shorter than an outage either passes a bounded
`CallerWait::Within(...)` (for in-process callers) or runs `instrukt is-safe`
as background work of its runtime. In-process callers invoke
`instrukt-screening` outside every lock another operation waits on.

### Response judgment order

For each try, `(status, headers, received_at)` (with `received_at = gate.now()`
stamped on header receipt) is judged BEFORE reading any response body:

1. **`200 OK`:** the only status whose body is read, streamed with a 1 MiB cap.
   Mid-body transport errors route through
   `instrukt_outside::http::judge_reqwest_error`. Body overflow (> 1 MiB),
   invalid UTF-8, invalid JSON or missing `choices[0]` produce
   `CallOutcome::Refused(MalformedAnswer)` without retrying.
   `choices[0].finish_reason == Some("stop")` is checked before accepting
   `choices[0].message.content`; any other or missing `finish_reason` produces
   `CallOutcome::Refused(CutOff)` without retrying.
2. **`400`, `413`, `422`:** body is unread; produces
   `CallOutcome::Refused(EndpointRejected { status })` without retrying.
3. **`3xx` or non-`200` `2xx`:** body is unread; produces `CallDefect` without
   retrying.
4. **Any other `>= 400` status** (including `401`, `402`, `403`, `404`, `408`,
   `425`, `429`, `5xx`): body is unread; judged via
   `instrukt_outside::http::judge_response` with `Settings::default()`.
   Transient statuses (`408`, `425`, `429`, `5xx`) return `Tried::Unavailable`
   to the gate; non-temporary `4xx` (`401`, `402`, `403`, `404`, etc.) return
   `Tried::Defect` immediately without retrying.
5. **Pre-response transport error:** judged via
   `instrukt_outside::http::judge_reqwest_error` with `Settings::default()`.

### Two-check screening rule

Each screening runs two independent checks over one shuffle sequentially
through the same `&Gate`. If check 1 returns `Waiting`, `Stopped`, `Defect` or
`EndpointRejected`, screening stops at once with that outcome; otherwise it
checks `stop()` and runs check 2. Text is `Safe` only when both checks return
`Verdict::Benign`. If either check returns `Verdict::Flagged`, checks disagree,
a check's answer is an invalid verdict, or a check ends in `CutOff` or
`MalformedAnswer`, screening returns `Unsafe { reason }` carrying the model's
bounded short reason (when flagged, structurally capped in length with no
code-side free-text quote scanner) or a fixed short reason otherwise.

### Entry checks

Before any HTTP request is sent, `call` and `is_safe` refuse at entry (naming
the cause and, for missing or invalid prerequisites, the repair door, without
disclosing any secret or embedded credential material) when:

- `llm.screening` is unconfigured (`LlmAdmission::Unconfigured`) or invalid
  (`LlmAdmission::Withheld`, including malformed URL, embedded URL credentials,
  non-loopback plaintext `http` URL, missing `url`/`model`, invalid `timeout`,
  unknown keys or non-`${NAME}` `api_key`);
- `llm.screening.api_key` is set and its secret fails resolution in
  `~/.instrukt/secrets.env`
  (`project/instrukt/spec/feature/configuration/owned-files`);
- the prompt (`call`) or text (`is_safe`) is empty or whitespace-only (zero
  words), or `max_tokens` (`call`) is outside `1..=4096`;
- for CLI `instrukt is-safe`, `<file>` or standard input is missing, unreadable
  or not valid UTF-8.

Nonempty text of one to five words passes entry checks and is screened as a
single fragment.

### Outcomes

| Surface / Outcome | Library return | CLI exit | CLI stdout | CLI stderr |
| --- | --- | --- | --- | --- |
| **Safe** | `Ok(SafeOutcome::Safe)` | `0` | `true` (or `{"safe":true}` with `--json`) | none (except prior outside-gate wait notices if recovered) |
| **Unsafe** | `Ok(SafeOutcome::Unsafe { reason })` | `1` | `false` (or `{"safe":false,"reason":"..."}` with `--json`) | none (the unsafe verdict is the command's report) |
| **Entry refusal** | `Ok(SafeOutcome::Refused(EntryRefusal))` / `Ok(CallOutcome::Refused(Entry(_)))` | `1` (usage error: `2`) | nothing | one line: `instrukt: <cause>[; <door>]` (scrubbed of secrets/credentials) |
| **Waiting** (bounded wait expired) | `Ok(SafeOutcome::Waiting(Wait))` / `Ok(CallOutcome::Waiting(Wait))` | — (CLI uses `CallerWait::UntilReturns`) | — | — |
| **Defect** | `Err(CallDefect)` | `1` | nothing | one line: `instrukt: <scrubbed cause>` (no secrets, credentials, prompts or raw bodies) |
| **Interrupted (`SIGINT`/`SIGTERM`)** | `Ok(SafeOutcome::Stopped)` / `Ok(CallOutcome::Stopped)` | signal default (`130`/`143`) | nothing | any prior outside-gate wait notice only |

`call` additionally returns `Ok(CallOutcome::Answered(String))` or
`Ok(CallOutcome::Refused(CutOff | EndpointRejected { status } | MalformedAnswer))`.

### Domain events emitted

None.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events. Durable recording of safety verdicts is outside
this context and belongs to the future domain event ledger.

## Use cases

- **capability-free-calls** — `project/instrukt/spec/feature/screening/capability-free-calls` — internal tool-less single-message calls to the configured OpenAI-compatible endpoint returning one bounded text answer.
- **untrusted-text-screening** — `project/instrukt/spec/feature/screening/untrusted-text-screening` — stateless boolean safety screening of untrusted text over shuffled 3-to-8-word fragments via `instrukt is-safe` and its matching internal function.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — the `instrukt-screening`
  crate, `secrets.env` scope and the configured model endpoint as an external
  surface.
- `project/instrukt/design/architecture/outside-gate` — the outside gate
  through which screening judges transient endpoint unavailability, paces
  retries and reports waits to the operator.

## Context map position

- **configuration → screening — Customer / Supplier:** the global-only
  `llm.screening` admission (`url`, `model`, optional resolved `api_key` from
  `secrets.env`, optional `timeout`), withheld alone when invalid so other
  capabilities run unchanged.
- **Configured model endpoint — Anti-corruption layer:** translates the
  configured OpenAI-compatible chat-completions endpoint into capability-free
  calls, bringing every try through the host-supplied outside gate
  (`project/instrukt/design/architecture/outside-gate`).

## Known caveats

- The capability-free guarantee holds for a plain model with no built-in search
  or browsing; instrukt performs no runtime check on which model is configured,
  and a model that searches on its own is outside the guarantee.
- Two independent checks of one shuffle by one model are separate tool-less
  calls, not different judges.
- Nonempty texts of one to five words, or texts in scripts that do not separate
  words with whitespace, cannot be split into multiple 3-to-8-word fragments
  and are screened as a single fragment; containment still holds because each
  check is capability-free.
- A flagged verdict's short `reason` is authored by a model that read untrusted
  text and remains a residual second-order prompt-injection surface; callers
  must treat it as untrusted data, never as instructions.
- An endpoint that silently truncates prompts exceeding its context window
  instead of returning `400`/`413`/`422` judges only what it saw.
