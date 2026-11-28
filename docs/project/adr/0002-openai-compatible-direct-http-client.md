---
description: Send capability-free OpenAI-compatible chat-completions calls directly through instrukt-outside's reqwest client with minimal local serde structs instead of adopting async-openai, genai, or openai-api-rs.
number: 2
date: 2026-11-28
---

# Direct OpenAI-Compatible HTTP Client Under the Outside Gate — ADR

## Context

`instrukt-screening` calls a single OpenAI-compatible HTTP endpoint (`POST {url}/chat/completions`) to run capability-free single-message prompts (`tools` omitted, `max_completion_tokens` bounded, `finish_reason == "stop"` required) against local servers (such as LM Studio, Ollama, or vLLM) and cloud endpoints. Under `sdlc/policy/substrate-anchoring`, external API clients adopt a maintained off-the-shelf SDK unless concrete incompatibilities against governing contracts are proven in the candidate SDKs' source.

`instrukt-screening` must simultaneously uphold four governing contracts:

1. **Single retry layer with `Retry-After` preservation (`project/instrukt/design/architecture/outside-gate`):** Every try routes through `instrukt-outside::Gate::call_async` and `instrukt_outside::http::judge_response` / `judge_reqwest_error` with zero secondary retry loops and no SDK-fighting middleware shim.
2. **Pre-body status/header judgment and 1 MiB streaming body cap (`sdlc/policy/security-awareness`, `project/instrukt/spec/domain/screening`):** `(status, headers, received_at)` is judged before reading any response body, non-`200` bodies stay unread, and `200 OK` bodies stream under a 1 MiB cap before buffering so oversized or stalled payloads cannot exhaust memory or distort status classification.
3. **Zero raw-body diagnostic logging and strict secret isolation (`sdlc/policy/security-awareness`, `sdlc/policy/observability`, `project/instrukt/spec/feature/configuration/owned-files`):** Raw endpoint response bodies (which may carry reflected credentials or untrusted text) must never be logged at `WARN`/`ERROR` — including for in-process daemon callers where third-party loggers are not clamped to `CRITICAL` — and process environment variables must never supply or override credentials.
4. **Structural tool omission and universal OpenAI-compatible interoperability (`project/instrukt/spec/feature/screening/capability-free-calls`):** Requests omit `tools` by construction, send `max_completion_tokens` without model-name heuristics, and tolerate OpenAI-compatible local endpoints that omit optional envelope metadata.

Direct source inspection of the three maintained Rust OpenAI SDKs on crates.io establishes concrete structural incompatibilities in each:

- **`async-openai` `0.43.0` (MIT, `reqwest ^0.13`):**
  1. **Unbounded body buffering and no pre-body status/header inspection (`src/client.rs:630-639, 729-766`):** `Client::execute_response` routes every non-`2xx` into `read_error_response`, which buffers the entire error body unbounded via `response.bytes().await` (`:737`); `read_response` buffers the entire `200` body unbounded (`:731`). Neither supports a streaming 1 MiB cap before buffering.
  2. **`Retry-After` discarded and HTTP status lost on non-JSON `4xx` (`src/client.rs:735-766`, `src/error.rs:122-127`):** `ApiErrorResponse` holds `{ status_code, api_error }` with no `HeaderMap`, discarding RFC 9110 `Retry-After` before `instrukt_outside::http::judge_response` can read it. When a non-`5xx` error body is not OpenAI's `{"error": ...}` JSON wrapper (such as plain-text or HTML `429`, `400`, `413`, or `422` from a local gateway or proxy), `read_error_response:764` falls through to `map_deserialization_error`, returning `OpenAIError::JSONDeserialize` and discarding `status_code` entirely.
  3. **Unredacted raw response bodies logged at `WARN` and `ERROR` (`src/client.rs:745`, `src/error.rs:148`):** `read_error_response:745` emits `tracing::warn!("Server error: {status} - {message}")` and `map_deserialization_error:148` emits `tracing::error!("failed deserialization of: {}", json_content)`, leaking raw unbounded endpoint bodies into diagnostics.
  4. **Hardcoded second retry loop (`src/executor.rs:138-150`, `src/middleware/retry/openai.rs`):** `Client::build` and `Client::with_http_client` unconditionally wrap `reqwest::Client` in `OpenAIRetryLayer::default()`, running a second retry/backoff loop (which consumes `429` bodies internally) outside `instrukt-outside::Gate`. Even enabling feature `middleware` to pass `ReqwestService::new(http_client)` via `Client::with_http_service` still runs `Client::execute_response`, `read_error_response`, and `read_response` on top of it, leaving gaps 1–3 intact unless a custom `tower::Service` short-circuits all non-`200` responses into `OpenAIError::Boxed` and caps `200` bodies itself — replacing the SDK's entire execution and error path with a fighting shim that `outside-gate` forbids.
  5. **Stock `OpenAIConfig` environment and auth coupling (`src/config.rs:57-110, 203-209`):** `OpenAIConfig::default()` reads `OPENAI_API_KEY` and `OPENAI_BASE_URL` from process environment, and `OpenAIConfig::headers()` unconditionally emits `Authorization: Bearer ` even for unkeyed local endpoints with `.parse().unwrap()`. (Avoidable via a custom `Config` implementation, unlike gaps 1–4.)
- **`genai` `0.7.6` (MIT OR Apache-2.0, `reqwest ^0.13`):**
  1. **Unbounded buffering before status/header return (`src/webc/web_client.rs:93-114`):** `WebResponse::from_reqwest_response` awaits `res.text().await?` unbounded on both error (`:98`) and success (`:113`) responses before returning status or headers, with no streaming size cap.
  2. **Raw body embedded in error `Display` and strict content-type gating (`src/webc/error.rs:11-22`, `src/webc/web_client.rs:115`):** `ResponseFailedStatus`, `ResponseFailedInvalidJson`, and `ResponseFailedNotJson` format the full raw response body in `Display`, and valid JSON bodies without an `application/json` `Content-Type` prefix are rejected.
  3. **Model-name prefix heuristics (`src/adapter/adapters/openai/adapter_shared.rs:254-262`):** Switches between `max_completion_tokens` and `max_tokens` based on `model_name` prefixes (`gpt-5`, `gpt-6`, `o`-series), conflicting with sending `max_completion_tokens` unconditionally without inspecting model names.
- **`openai-api-rs` `10.0.2` (MIT):** Pins `reqwest ^0.12`, incompatible with the workspace's `reqwest 0.13` and `instrukt_outside::http::build_client`.

## Decision

Implement `instrukt-screening`'s single-endpoint `POST {url}/chat/completions` transport directly over `instrukt_outside::http::build_client` (`reqwest` 0.13, `follow_redirects: false`) and `instrukt_outside::Gate::call_async`, with minimal local `serde` request and response structs where `tools` is structurally absent from the request type.

Each try builds one `POST {url}/chat/completions` JSON request (`model`, single `user` message, `max_completion_tokens`, optional `Authorization: Bearer` header only when configured), stamps `received_at = gate.now()` on response header receipt, judges `(status, headers, received_at)` before reading any body (`400`/`413`/`422` as endpoint rejection, other `>= 400` via `instrukt_outside::http::judge_response`, `3xx`/non-`200` `2xx` as defect), streams only `200 OK` bodies under a 1 MiB cap, verifies `choices[0].finish_reason == Some("stop")` before accepting non-empty `choices[0].message.content`, and scrubs all errors via `reqwest::Error::without_url()` without ever logging raw bodies.

## Alternatives Considered

- **Adopt `async-openai` `0.43.0` (`chat-completion` + `middleware` features) with a custom `Config` and custom `tower::Service`:** Rejected because `Client::execute_response` (`src/client.rs:630-639`) still runs `read_error_response` and `read_response` above the service, dropping `Retry-After`, losing `status_code` on non-JSON `4xx`, buffering bodies unbounded, and logging raw bodies at `WARN`/`ERROR`. Short-circuiting every non-`200` response inside a custom Tower service to avoid `read_error_response` bypasses the SDK's transport and error pipeline altogether, creating a fighting shim forbidden by `outside-gate`.
- **Adopt `async-openai` `0.43.0` types only (`default-features = false, features = ["chat-completion-types"]`):** Rejected because `CreateChatCompletionResponse` (`src/types/chat/chat_.rs:1185, 1231, 1250`) mandates OpenAI-strict metadata fields (`id`, `created: u32`, `model`, `object`, `index: u32`) and a closed 5-variant `FinishReason` enum with no unknown-variant catch-all (rejecting OpenAI-compatible local servers that omit optional metadata or emit other finish reasons), while `CreateChatCompletionRequest` (`:889, 1084`) exposes `pub tools: Option<Vec<ChatCompletionTools>>` and dozens of unused fields via `derive_builder` rather than making tool omission structurally unrepresentable.
- **Adopt `genai` `0.7.6`:** Rejected because `WebResponse::from_reqwest_response` buffers error and success bodies unbounded before returning status/headers, embeds raw response bodies in `Error::Display`, and inspects model-name prefixes to choose between `max_completion_tokens` and `max_tokens`.
- **Adopt `openai-api-rs` `10.0.2`:** Rejected because it pins `reqwest ^0.12`, incompatible with the workspace's `reqwest 0.13` and `instrukt_outside::http::build_client`.

## Consequences

- `instrukt-screening` keeps a small, self-contained (~120-line) HTTP client over `instrukt_outside::http::build_client` with zero additional third-party dependencies and zero `deny.toml` advisory exceptions.
- `instrukt-outside::Gate` remains the single retry layer with full `Retry-After` preservation and pre-body status judgment; `200 OK` payloads are strictly capped at 1 MiB while streaming; raw endpoint bodies never enter logs; and `tools` cannot be populated on a capability-free request at compile time.
- `instrukt-screening` owns its minimal `/chat/completions` wire structs and must update them directly if the OpenAI-compatible chat-completions envelope changes (a stable surface of three request fields and two response fields).
- If a future `async-openai` release supports pre-body response hooks (preserving `HeaderMap` and `StatusCode` without buffering or logging raw bodies), bounded streaming reads, and non-retrying execution without Tower shims, or if `instrukt` later requires broader multi-endpoint OpenAI API coverage (such as Responses, Audio, or Embeddings), this ADR can be superseded and the transport migrated onto that SDK.

## See Also

- [project/instrukt/design/architecture/outside-gate](../design/architecture/outside-gate.md)
- [project/instrukt/design/architecture/overview](../design/architecture/overview.md)
- [project/instrukt/adr/0001-slack-socket-mode-transport](0001-slack-socket-mode-transport.md)
