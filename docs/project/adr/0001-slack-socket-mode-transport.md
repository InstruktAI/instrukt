---
description: Keep Slack Socket Mode WebSocket framing on single-shot tokio-tungstenite under instrukt-outside::Gate while using slack-morphism for Slack Web API calls, connection opening, and Events API payload types.
number: 1
date: 2026-11-27
---

# Slack Socket Mode Single-Shot Transport — ADR

## Context

`instrukt-connector-slack` adopts `slack-morphism` (`2.34.0`) for Slack Web API calls, `apps.connections.open`, and `SlackEventCallbackBody` decoding. For the live Socket Mode listener, `sdlc/policy/substrate-anchoring` prefers adopting the SDK's own Socket Mode listener (`SlackClientSocketModeListener` / `SlackTungsteniteWssClient`) unless the SDK cannot satisfy project contracts.

Inspection of locked `slack-morphism-2.34.0` shows three structural incompatibilities in its built-in Socket Mode client (`src/hyper_tokio/socket_mode/tungstenite_wss_client.rs`, `src/lib.rs`, `src/models/mod.rs`):

1. **Ticket secret logging (`tungstenite_wss_client.rs:110-137`):** `SlackTungsteniteWssClient::try_to_connect` logs the raw Socket Mode URL — which carries Slack's single-use `?ticket=` query parameter — at `tracing::error!` on failed handshakes and at `tracing::debug!` on connect. This violates `sdlc/policy/security-awareness` and `project/instrukt/spec/feature/company/slack-connector` secret hygiene.
2. **Unmanaged second retry loop (`tungstenite_wss_client.rs:142-182`):** `try_to_connect` collapses `apps.connections.open` and handshake failures into `None`, and `connect_with_reconnections` retries indefinitely on a fixed `reconnect_timeout_in_seconds` timer. This duplicates retry ownership outside `instrukt-outside::Gate` (`project/instrukt/design/architecture/outside-gate`) and prevents `connector_host` from observing failure causes or transitioning the connector to `ConnectionState::Failed`.
3. **No outbound socket-origin admission hook and shadowed envelope module (`tungstenite_wss_client.rs:105`, `src/lib.rs:145`, `src/models/mod.rs:21`):** `try_to_connect` dials the returned URL directly without admitting its host/scheme (`admits_socket`), and `slack_morphism::models::socket_mode` is shadowed by `pub mod socket_mode`, so callers cannot decode Socket Mode wrapper envelopes through public SDK types without running the stock listener.

## Decision

Keep Slack Web API calls (`chat.postMessage`, `assistant.threads.setStatus`, `users.info`, `apps.connections.open`, conversation history/replies reads) and `SlackEventCallbackBody` deserialization on `slack-morphism`, and run the single-shot Socket Mode WebSocket connection in `crates/instrukt-connector-slack/src/_socket_mode.rs` directly over `tokio-tungstenite` judged by `instrukt-outside::Gate`.

Each connection attempt acquires one URL via `slack-morphism`'s `apps_connections_open`, validates it with `admits_socket`, suppresses tracing/log output during the WebSocket handshake so `?ticket=` never reaches logs, reads frames until `hello`, retains addressed `SlackEventCallbackBody` events durably before writing `{"envelope_id": ...}`, and returns on drop, disconnect, or shutdown so `connector_host` and `instrukt-outside::Gate` own all reconnect pacing and failure reporting.

## Alternatives Considered

- **Use `SlackClientSocketModeListener` with stock `SlackTungsteniteWssClient`:** Rejected because it violates secret hygiene (`?ticket=` logged at `error!`/`debug!`), runs a second unmanaged reconnect loop outside `instrukt-outside::Gate`, hides `apps.connections.open` failures from `ConnectionState::Failed`, and skips `admits_socket` validation.
- **Implement a custom `SlackSocketModeClientsManagerFactory` inside `SlackClientSocketModeListener`:** Rejected because the custom manager would still need to replace `SlackTungsteniteWssClient`'s connection, reconnect, and envelope loop verbatim while adding trait-wrapper boilerplate on top of the same `tokio-tungstenite` socket loop.

## Consequences

- `instrukt-connector-slack` keeps a bounded (~300-line) Socket Mode WebSocket loop in `crates/instrukt-connector-slack/src/_socket_mode.rs` while delegating Slack HTTP API and Events API body models to `slack-morphism`.
- `instrukt-outside::Gate` owns reconnect pacing, `Retry-After` handling, and outage reporting; `connector_host` owns connection lifecycle and `ConnectionState::Failed` reporting; and `instrukt-connector-slack` owns outbound `admits_socket` validation, handshake `?ticket=` log suppression, and durable retain-before-ack.
- If a future `slack-morphism` release exposes single-shot, ticket-redacted, origin-validated Socket Mode connections without an internal reconnect loop, this ADR can be superseded and `_socket_mode.rs` moved onto that SDK surface.

## See Also

- [project/instrukt/spec/feature/company/slack-connector](../spec/feature/company/slack-connector.md)
- [project/instrukt/design/architecture/outside-gate](../design/architecture/outside-gate.md)
