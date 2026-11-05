---
description: "One loopback receiver verifies and unconditionally relays authenticated external events to cluster Redis."
delivered_by: [instrukt-first-build]
---
# Single-Ingress Webhook Termination — Spec

## What it is

Exactly one configured ingress computer accepts external requests for the
cluster. Every other computer consumes cluster events without an inbound public
listener. The receiver verifies and publishes facts; local reactions consume the
same stream as remote reactions, without a direct request-dispatch path.

## Canonical fields

### Receiver and public edge

The global-only `ingress.computer` names the existing stable instrukt machine ID,
not a hostname inferred at runtime. Only that computer serves source endpoints.
The receiver checks placement before serving and exits when no longer ingress.
A valid enabled nonempty source set supplies receiver demand; an empty map or
disabled events does not. Installation/withdrawal mechanisms belong to S7.

The standalone receiver serves HTTP on `127.0.0.1` at global-only `ingress.port`.
A local operator-controlled edge terminates TLS, filters abuse and forwards only
the inbound prefix. A local request still needs a signature. No session-control,
work-command or general public API surface is exposed.

The admitted service manager holds the listening socket across receiver restarts
and passes it to each process. Connections queue during restart within provider
timeouts. The receiver outlives stopped/restarting `run` or scheduled tick;
it is not hosted in a new consumer daemon and has no daemon fallback. A validated
machine configuration change requiring restart is applied through receiver
lifecycle; invalid configuration leaves its last valid running configuration.
Source declarations themselves refresh per request.

### Publication and source-scoped repeat claims

Every verified, well-formed native/GitHub delivery is published to cluster Redis
regardless of matching subscribers or local reaction readiness. Its source is
the ingress computer; origin is the declaring project; subject is its typed
inbound source including protocol. It references no session, regardless of
request identity headers. Data is the producer document/native payload.
Relayed names require no per-name registration.

One cluster operation claims and appends a new authenticated delivery without
interleaving. The claim key includes protocol, declaring home, provider delivery
ID (GitHub's `X-GitHub-Delivery`, when present) or SHA-256 digest of verified raw
body, then emitted-event index. Claim lifetime is three days. A repeat claim is
accepted without a second append only when that claim represents successful
publication. Failed append leaves no false completed claim that swallows retry.
Two source homes or protocols never share a claim. Native and GitHub each emit
one event per verified document in this scope.

Ingress acceptance ends at append or recognized duplicate, never at a local
SQLite-only buffer. Unavailable cluster Redis yields transient failure. The
receiver does not invoke procedure matching or provider reactions inline.
All reactions use independent stream consumers. Protocol interpretation follows
the typed source reference, not the event type prefix: a native `github.push`
stays native. GitHub normalization is shared wherever that provider document is
translated, retaining original timestamp and payload; this does not install a
second generic handler registry or postponed conversation adapters.

### Activation reach

Each computer may read the cluster stream. The procedure-activation consumer
acts only where source equals its own computer, so these external procedures
activate on ingress. A missing declared project home there fails visibly; another
computer holding that checkout is not an implicit fallback. Other explicitly
admitted consumers retain their own source-selection contracts.

### Use cases

#### UC-SIT-1: Make external facts available throughout a private cluster

```gherkin
Given a developer has private computers consuming one cluster stream
When a provider delivers a valid signed project webhook to ingress
Then each running computer can read the resulting fact without exposing an inbound listener
```

#### UC-SIT-2: Accept a verified fact even with no local reaction

```gherkin
Given a developer has no matching procedure or runtime available on ingress
When a valid signed delivery arrives
Then ingress still publishes it to cluster Redis and acknowledges acceptance
```

#### UC-SIT-3: Reject unauthenticated external input at intake

```gherkin
Given a developer exposes only the loopback receiver through an edge
When an invalidly signed request arrives
Then it is rejected without publishing or dispatching a reaction
```

#### UC-SIT-4: Keep external listeners off other computers

```gherkin
Given a developer selects one stable ingress machine ID
When the receiver is started on another computer or its computer loses ingress placement
Then it serves no external endpoint
```

#### UC-SIT-5: Serve all declared projects and protocols through one ingress

```gherkin
Given a developer declares native and GitHub sources for multiple projects
When each sends valid signed input to its derived path
Then one ingress verifies each against its own project and publishes it
```

#### UC-SIT-6: Consume a relayed fact as an ordinary platform event

```gherkin
Given a developer receives a verified external fact
When their consumer reads it
Then it carries ingress source, declaring-project origin, typed source subject, occurrence ID, time and original data
```

#### UC-SIT-7: Subscribe to an external vocabulary without registering every name

```gherkin
Given a developer publishes an enabled matching procedure for a project
When a new well-formed external type reaches that project
Then its event name can select the procedure without adding a catalog declaration
```

#### UC-SIT-8: Avoid false session attribution from external headers

```gherkin
Given a developer receives a webhook carrying session or identity headers
When ingress publishes its event
Then no session reference is inferred from those headers
```

#### UC-SIT-10: Retain native provenance for provider-looking names

```gherkin
Given a developer sends a signed native event named github.push
When it reaches stream consumers
Then its subject still identifies the native source and it receives no GitHub provider treatment
```

#### UC-SIT-11: Distinguish the same facts received through different protocols

```gherkin
Given a developer receives the same type, project and payload through native and GitHub endpoints
When their consumer inspects the events
Then protocol-qualified source subjects keep them distinguishable and their claims do not collide
```

## Known caveats

- UC-SIT-9's WhatsApp delivery is outside this port; its ID is not reused.
- Ingress is a single intake point. Provider retry, provider timeout, cluster availability and bounded stream retention limit recovery.
- Socket queuing survives receiver restart only within the provider timeout and an admitted service manager's support; it supplies no outage store-and-forward.
- GitHub's delivery ID and event-type headers are not signed body fields. Source-scoped claim deduplication is bounded replay suppression, not independent authentication of those headers.
- A scheduled-only ingress consumer activates on its next tick within retention even while the receiver continues accepting.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](inbound-webhook-endpoint-framework.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
