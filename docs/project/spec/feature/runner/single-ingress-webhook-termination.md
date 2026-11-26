---
description: "One loopback receiver verifies and unconditionally relays authenticated external events to cluster Redis, and one computer reacts to them."
---
# Single-Ingress Webhook Termination — Spec

## What it is

Exactly one configured ingress computer accepts external requests for the
cluster, and exactly one computer reacts to them: the ingress, or the computer
`ingress.reactions` names. Every other computer consumes cluster events without
an inbound public listener. The receiver verifies and publishes facts; local reactions consume the
same stream as remote reactions, without a direct request-dispatch path.

## Canonical fields

### Receiver and public edge

The global-only `ingress.computer` names the existing stable instrukt machine ID,
not a hostname inferred at runtime. Only that computer serves source endpoints,
the MCP sign-in return, and the public discovery/JWKS endpoints for built-in
per-company OIDC issuers.
The receiver checks placement before serving and exits when no longer ingress.
A valid enabled nonempty source set supplies receiver demand, and so does
`ingress.url` set on the ingress computer while events is on there; an empty
map or disabled events does not. Installation/withdrawal mechanisms belong to
S7.

The standalone receiver serves HTTP on `127.0.0.1` at global-only `ingress.port`.
A local operator-controlled edge terminates TLS, filters abuse and forwards only
the inbound prefix. A local request to a source still needs a signature. The
sign-in return under the same prefix is unsigned and knows no company: it
checks only the request's form and its path, and hands a code on through
cluster Redis as a bounded, short-lived entry that is no event
(`project/instrukt/spec/feature/runner/mcp-access`). Built-in per-company OIDC
issuer metadata (`GET /hooks/inbound/issuers/<issuer-id>/.well-known/openid-configuration`
and `GET /hooks/inbound/issuers/<issuer-id>/jwks.json`) is served read-only
under the same prefix from bounded public metadata published in cluster Redis;
no private signing key ever resides on ingress or in cluster Redis. The edge
filters abuse of these paths, their rate and size, as for sources. No
session-control, work-command or general public API surface is exposed.

The admitted service manager holds the listening socket across receiver restarts
and passes it to each process. Connections queue during restart within provider
timeouts. The receiver outlives stopped/restarting `run` or scheduled tick;
it is not hosted in a new consumer daemon and has no daemon fallback. A validated
machine configuration change requiring restart is applied through receiver
lifecycle; invalid configuration leaves its last valid running configuration.
Source declarations themselves refresh per request.

### Publication and source-scoped repeat claims

Every verified, well-formed domain-source delivery is published to cluster Redis
regardless of matching subscribers or local reaction readiness. Its source is
the ingress computer; origin is the declaring project or company; subject is
its typed inbound source including domain. It references no session, regardless of
request identity headers. Data is the producer document/native payload.
Relayed names require no per-name registration.

One cluster operation claims and appends a new authenticated delivery without
interleaving. The claim key includes domain, verification scheme, declaring home,
provider delivery ID (GitHub's `X-GitHub-Delivery`, when present) or SHA-256 digest of verified raw
body, then emitted-event index. Claim lifetime is three days. A repeat claim is
accepted without a second append only when that claim represents successful
publication. Failed append leaves no false completed claim that swallows retry.
Two source domains, homes or verification schemes never share a claim. All
admitted sources emit one event per verified document in this scope.

Ingress acceptance ends at append or recognized duplicate, never at a local
SQLite-only buffer. Unavailable cluster Redis yields transient failure. The
receiver does not invoke procedure matching or provider reactions inline.
All reactions use independent stream consumers. Protocol interpretation follows
the retained `inbound_verify` extension, not the event type prefix: a native
`github.push` stays native. GitHub normalization is shared wherever that provider document is
translated, retaining original timestamp and payload; this does not install a
second generic handler registry or postponed conversation adapters.

### Activation reach

An external fact is an event the receiver publishes from a verified delivery;
its subject is an inbound source (`inbound_source:…`). Each computer may read
the cluster stream. Exactly one computer, the reacting computer, activates
procedures for external facts. The global-only `ingress.reactions` names it by
its stable instrukt machine ID; left out, the ingress computer reacts. All
participating computers agree on it, as on `ingress.computer`. The reacting
computer's procedure-activation consumer selects external facts by their
inbound-source subject, and their source stays the ingress computer. Every
other computer's consumer, the ingress's included when it is not the reacting
computer, activates nothing for them. An event a computer emits itself
activates only on that computer, whatever `ingress.reactions` names.

When the ingress and the reacting computer differ, they need different things:

<!-- planned-change:company-home-generation -->
| Computer | Needs | Does not need |
| --- | --- | --- |
| Ingress | What verification reads: each declaring project adopted and registered with its checkout, each declaring company home's `instrukt.yaml`, and the signing values in the declaring home's owner-only secrets file, with events enabled for the declaring home | A runtime (`project/instrukt/spec/feature/runner/session-continuity`), or any procedure run for an external fact |
| Reacting computer | The homes the procedures run in: the registered project checkout or company home the fact's origin names, with its published procedures, the chosen runtime and events enabled for that home | Signing values, or a receiver |
<!-- change:company-home-generation -->
| Computer | Needs | Does not need |
| --- | --- | --- |
| Ingress | What verification reads: each declaring project adopted and registered with its checkout, each declaring company home's `company.yaml`, and the signing values in the declaring home's owner-only secrets file, with events enabled for the declaring home | A runtime (`project/instrukt/spec/feature/runner/session-continuity`), or any procedure run for an external fact |
| Reacting computer | The homes the procedures run in: the registered project checkout or company home the fact's origin names, with its published procedures, the chosen runtime and events enabled for that home | Signing values, or a receiver |
<!-- /planned-change:company-home-generation -->

Changing `ingress.reactions` or `ingress.computer` is a change the operator
coordinates while nothing is in flight. It is supported only when no
participating computer's procedure consumer, the computer that reacted so far
and the one that will react included, has entries from before the change still
to read or settle, and the computer that reacted so far has settled what it
accepted. Stopping intake keeps new entries from arriving; it does not satisfy
the condition by itself. The setting is then changed on every participating
computer and intake resumes. An
activation a computer has accepted stays with that computer and runs there. A
fact read after the change is handled under the new setting.

A home missing on the reacting computer fails visibly at the accepted
activation's execution boundary. Another home or computer never substitutes
for it, and no fact is routed per project or per home. Other explicitly
admitted consumers retain their own source-selection contracts.

## Use cases

### UC-SIT-1: Make external facts available throughout a private cluster

```gherkin
Given a developer has private computers consuming one cluster stream
When a provider delivers a valid signed project webhook to ingress
Then each running computer can read the resulting fact without exposing an inbound listener
```

### UC-SIT-2: Accept a verified fact even with no local reaction

```gherkin
Given a developer has no matching procedure or runtime available on ingress
When a valid signed delivery arrives
Then ingress still publishes it to cluster Redis and acknowledges acceptance
```

### UC-SIT-3: Reject unauthenticated external input at intake

```gherkin
Given a developer exposes only the loopback receiver through an edge
When an invalidly signed request arrives
Then it is rejected without publishing or dispatching a reaction
```

### UC-SIT-4: Keep external listeners off other computers

```gherkin
Given a developer selects one stable ingress machine ID
When the receiver is started on another computer or its computer loses ingress placement
Then it serves no external endpoint
```

### UC-SIT-5: Serve all declared projects and protocols through one ingress

```gherkin
Given a developer declares native and GitHub sources for multiple projects
When each sends valid signed input to its derived path
Then one ingress verifies each against its own project and publishes it
```

### UC-SIT-6: Consume a relayed fact as an ordinary platform event

```gherkin
Given a developer receives a verified external fact
When their consumer reads it
Then it carries ingress source, declaring-project origin, typed source subject, occurrence ID, time and original data
```

### UC-SIT-7: Subscribe to an external vocabulary without registering every name

```gherkin
Given a developer publishes an enabled matching procedure for a project
When a new well-formed external type reaches that project
Then its event name can select the procedure without adding a catalog declaration
```

### UC-SIT-8: Avoid false session attribution from external headers

```gherkin
Given a developer receives a webhook carrying session or identity headers
When ingress publishes its event
Then no session reference is inferred from those headers
```

### UC-SIT-10: Retain native provenance for provider-looking names

```gherkin
Given a developer sends a signed native event named github.push
When it reaches stream consumers
Then its domain-qualified subject and verification metadata still identify the native source and it receives no GitHub provider treatment
```

### UC-SIT-11: Distinguish the same facts received through different protocols

```gherkin
Given a developer receives the same type, project and payload before and after changing a domain source from native to GitHub verification
When their consumer inspects the events
Then verification metadata keeps them distinguishable and their claims do not collide
```

### UC-SIT-12: React to external facts on a computer other than the ingress

```gherkin
Given a developer's server is the ingress and names their other computer as the reacting computer
And the other computer holds the project, its published procedure and a chosen runtime
When a valid signed delivery for that project matches the procedure
Then the procedure runs on the other computer in that project's checkout
And the server publishes the fact and runs no procedure for it
```

## Known caveats

- UC-SIT-9's WhatsApp delivery is outside this port; its ID is not reused.
- Ingress is a single intake point. Provider retry, provider timeout, cluster availability and bounded stream retention limit recovery.
- Socket queuing survives receiver restart only within the provider timeout and an admitted service manager's support; it supplies no outage store-and-forward.
- GitHub's delivery ID and event-type headers are not signed body fields. Source-scoped claim deduplication is bounded replay suppression, not independent authentication of those headers.
- A scheduled-only reacting computer activates on its next tick within retention even while the receiver continues accepting.
- Changing `ingress.reactions` or `ingress.computer` other than as a coordinated change while nothing is in flight is not supported, including a change made while any participating procedure consumer still has entries from before it to read or settle: a fact may then find no reacting computer or two. No work moves between computers.
- Activation keys are kept per computer. instrukt does not detect two computers that disagree on `ingress.computer` or `ingress.reactions`: then no computer reacts to external facts, the wrong one does, or two do, and nothing reports it. Agreement is established when the computers are configured (`project/instrukt/spec/feature/configuration/configuration-skill`); `instrukt events status` on each computer shows its machine ID and both settings as it reads them, for a person or an agent to compare.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](inbound-webhook-endpoint-framework.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
