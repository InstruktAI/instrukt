---
description: "Domain-keyed signed project endpoints, scoped secret rotation and opaque event intake."
delivered_by: [instrukt-first-build]
---
# Inbound Webhook Endpoint Framework — Spec

## What it is

A project opens an inbound source by declaring `webhooks:` directly in its
`instrukt.yaml`. The designated ingress reads that declaration for each request,
verifies the original bytes and publishes ordinary cluster events. Producers
of ours map their own domain vocabulary; GitHub uses the built-in normalizer.

## Canonical fields

### Declaration and endpoint identity

```yaml
webhooks:
  erp: {secret: ${ERP_HOOK_SECRET}, previous_secret: ${ERP_HOOK_PREVIOUS_SECRET}}
  github: {verify: github, secret: ${GITHUB_HOOK_SECRET}}
  billing: {verify: hmac-sha256, header: Billing-Signature, secret: ${BILLING_SECRET}, name_from: type}
  sentry: {verify: hmac-sha256, header: Sentry-Hook-Signature, secret: ${SENTRY_SECRET}, name: issue_alert}
```

Sources sit directly under `webhooks`, keyed by the event domain, with no
direction or `sources` subkey. Domain matches `[a-z][a-z0-9_]*`. Each registered
project can declare several domains, each selecting its own verification scheme,
secret references and event-name source. Project configuration and machine-local secrets must
exist on ingress.

| Source | Derived POST path | Origin and subject |
| --- | --- | --- |
| Any declared domain | `/hooks/inbound/project/<slug>/<domain>` | Project origin; `inbound_source:<domain>:project:<slug>` subject |
| GitHub example | `/hooks/inbound/project/<slug>/github` | Project origin; `inbound_source:github:project:<slug>` subject |

The general path is `/hooks/inbound/<kind>/<slug>/<domain>`: kind and slug
identify the origin home; domain selects its declared source and event domain.
Only project homes execute. Company/person homes are a stated capability limit
and have no endpoint. No other path segment or query parameter is accepted;
secrets never occur in URLs. An undeclared domain or unregistered project has
no endpoint, even with valid signing material. An empty map creates none.
Effective events-off sources are a plain finding and are not served; source
configuration never overrides module enablement.

`verify` defaults to `native`; supported values are `native`, `github` and
`hmac-sha256`. An unknown scheme such as `stripe` is a validation finding naming
the scheme, never an unsigned fallback. Generic HMAC requires a declared valid
HTTP `header` name; native and GitHub use their built-in headers and reject a
header override. Missing secrets, invalid domains, unknown source fields,
conflicting name sources, invalid fixed names and empty `name_from` fields are
validation findings. Optional fields cannot be null.

### Secrets and per-request resolution

`secret` is required; `previous_secret` is optional. Both are raw `${NAME}`
references, never literal signing material. Raw-reference validation precedes
interpolation. On ingress each resolves solely from the owner-only machine-local
`~/.instrukt/secrets.env`, with permission checks on read under
`project/instrukt/spec/feature/configuration/owned-files`. Project `.env`,
process environment and receiver cwd supply no fallback. A missing,
blank or unreadable required value (including a declared previous value) prevents
that declaration from validating on ingress. Other computers validate reference
shape without needing the values. Responses, configuration reads and logs expose
reference names and declaring project, never expanded values.

Added, changed, removed and rotated declarations take effect on the next request,
without restarting ingress. HMAC-SHA256 verification uses constant-time comparison
over the exact original body bytes. Current and previous secrets both verify
during overlap, without reporting which matched; removing previous immediately
ends its acceptance. The signature is verified before any structured body parse.
No request header naming a session supplies caller attribution.

### Verification and event names

Native verification requires `X-Hook-Signature: sha256=<hex digest>` over raw
bytes. Generic `hmac-sha256` reads its declared header and accepts a hex digest
with or without the `sha256=` prefix. Both use the same current/previous-secret
overlap. All sources require verification before decoding.

Exactly one name source applies:

- Without `name`, `name_from` or `verify: github`, the JSON object has exactly
  `name` and `payload`. Name is one or more dot-separated segments, each matching
  `[a-z][a-z0-9_]*`. Payload is an opaque object; a body `domain` field is refused.
- `name_from: <field>` reads that top-level JSON object's string field, lowercases
  ASCII letters and replaces each character other than ASCII letters, digits,
  underscore or dot with underscore. The result must match the name grammar;
  absent/nonstring fields and invalid results are client errors. The whole
  object, including the original name field, is data.
- Fixed `name:` must already match the name grammar. The whole JSON object is data.
- `verify: github` selects the built-in normalizer and admits neither name setting.

Every event type is `<domain>.<name>`, with domain solely from the path and
matching declaration. Unknown well-formed event names require no catalog entry.
The envelope extension `inbound_verify` records the verification scheme so
consumers retain protocol provenance independently of the domain-named subject.

### GitHub protocol

`X-Hub-Signature-256: sha256=<hex digest>` signs the raw bytes. The body is a JSON
object or `application/x-www-form-urlencoded` (media-type parameters allowed)
with exactly one `payload` field containing that JSON object. Duplicate fields,
additional fields, missing payload, malformed form/JSON or a nonobject fail.
Both encodings carry identical event data after verification; signatures remain
over their different original bytes. The GitHub normalizer owns type and property
extraction. Neither native nor GitHub implements a subscription handshake.

### Responses and acceptance

| Condition | Response and effect |
| --- | --- |
| Endpoint absent | 404, no relay |
| Missing/invalid signature | 401, no parse or relay |
| Malformed verified body | 400, no relay |
| Unsupported handshake method | Method refusal, no handshake or relay |
| Declaring configuration/secret unreadable | 500, no relay; diagnostic names project/reference safely |
| Cluster claim/publication fails | Transient 502; sender must retry, no local acceptance buffer |
| Appended or recognized duplicate claim | Successful accepted response |

Acceptance requires stream append or recognition of an already published
source-scoped delivery. It is independent of subscription matches or runtime
availability. Signature validity authenticates a source; event content remains
untrusted data, not authorization to mutate work.

### External state declarations

External names default to occurrence. A project `webhooks.<domain>` source may
set `state: [<name>, …]` and `subject_from: <dotted-payload-field>`. Names omit
the domain prefix and follow the ordinary event-name grammar. State names
require a nonempty subject field declaration. After signature verification,
the field is read from the normalized payload and must be a nonempty string
or number. Its UTF-8 percent-encoded value qualifies the source's ordinary
`inbound_source:<domain>:project:<slug>` subject. Missing or invalid values
produce a client error with no relay. Other names remain occurrences. Ingress
records `inbound_delivery: state` as producer metadata independently of payload
text. Pending-state replacement is owned by procedure-event-delivery.

## Use cases

### UC-IF5: Use native and GitHub sources without a provider handshake

```gherkin
Given a developer has declared a native or GitHub endpoint
When a caller attempts a subscription verification handshake
Then no challenge is echoed and no event is relayed
```

### UC-IF8: Correct malformed delivery input

```gherkin
Given a developer sends a correctly signed request with a malformed protocol body
When ingress receives it
Then it returns a client error and relays nothing
```

### UC-IF17: Expose only declared endpoints

```gherkin
Given a developer has not registered a project or has not declared the requested source
When a request reaches its derived path
Then ingress returns not found and relays nothing
```

### UC-IF18: Authenticate every request before processing it

```gherkin
Given a developer has a declared endpoint reachable even from loopback
When a request has a missing or invalid raw-body signature
Then it is unauthorized and no structured payload is parsed or relayed
```

### UC-IF19: Configure a valid signed source

```gherkin
Given a developer declares a webhook with missing or literal secret references, an unsupported verification scheme or invalid domain, signature-header or name-source settings
When configuration is validated
Then it is refused identifying the declaration or unsupported scheme without exposing signing material
```

### UC-IF20: Add or remove a project source without restarting ingress

```gherkin
Given a developer has a running ingress
When they add, edit or remove a project domain source in valid configuration
Then the next request uses that declaration without restart and a removed source has no endpoint
```

### UC-IF21: Publish a domain fact without teaching instrukt its business schema

```gherkin
Given a developer declares a domain source and sends a valid signed native wrapper with a new event name and opaque payload
When ingress accepts it
Then the relayed type is domain dot name and its data is exactly the supplied payload
```

### UC-IF22: Correct an invalid native event name

```gherkin
Given a developer sends a signed native wrapper whose name has an invalid or empty segment
When ingress checks the wrapper
Then it returns a client error and publishes no event
```

### UC-IF23: Retry a delivery whose relay failed

```gherkin
Given a developer's source sends a verified delivery while cluster publication fails
When ingress answers
Then the answer is a transient failure and does not claim successful acceptance
```

### UC-IF24: Diagnose an unreadable declaring project

```gherkin
Given a developer's registered source configuration or declared secret cannot be read on ingress
When a request reaches it
Then it receives a server error with no relay
And diagnostics identify the home and unresolved reference without a secret value
```

### UC-IF25: Receive either GitHub content type

```gherkin
Given a developer sends the same GitHub document as JSON and as the sole form payload field with signatures over each original body
When ingress verifies the deliveries
Then both relay the same document and GitHub event meaning
```

### UC-IF26: Reject an ambiguous GitHub form body

```gherkin
Given a developer sends a signed GitHub form with missing, repeated or additional payload fields or a nonobject document
When ingress receives it
Then it returns a client error and relays nothing
```

### UC-IF27: Rotate signing secrets without dropping valid deliveries

```gherkin
Given a developer has current and previous secret references during rotation
When either secret signs a valid request
Then ingress accepts it without disclosing which secret matched
When the previous reference is removed
Then a request signed only by that old secret is rejected on the next request
```

### UC-IF28: Resolve machine-local signing references without disclosure

```gherkin
Given two projects declare distinct signing references in the same machine-local protected secrets file
When a developer delivers to one source and inspects configuration or diagnostics
Then only that source's referenced value verifies and neither value is printed
And neither a project environment file nor process environment can supply or override the source secret
```

### UC-IF29: Receive a payload-only sender's event name

```gherkin
Given a developer declares a signed source with name_from naming a top-level payload field
When its JSON object supplies a string that normalizes to a valid event name
Then ingress publishes domain dot normalized name and preserves the whole body as data
But when the field is absent, nonstring or normalizes to an invalid name
Then ingress returns a client error and relays nothing
```

### UC-IF30: Receive a single-kind sender's payload

```gherkin
Given a developer declares a signed source with a fixed valid name
When a verified JSON object arrives
Then ingress publishes domain dot configured name and preserves the whole body as data
```

### UC-IF31: Keep undeclared domains closed

```gherkin
Given a developer declares one domain source for a project
When a correctly signed request targets another undeclared domain
Then ingress returns not found and relays nothing
```

### UC-IF32: Verify a generic HMAC sender

```gherkin
Given a developer declares hmac-sha256 with its signature header and current and previous secrets
When either secret signs the original bytes with a hex digest optionally prefixed by sha256=
Then ingress accepts the verified delivery
But when the signature is absent, malformed, duplicated or incorrect
Then ingress rejects it without relay
```

### UC-IF33: Isolate sources by their declared domains

```gherkin
Given a developer declares several domains with their own signing references in one project
When identical body bytes arrive at each source
Then each endpoint verifies only its domain's secret and publishes its own domain-qualified type and subject
And source-scoped claims do not suppress another domain's delivery
```

### UC-IF34: Declare external state and its subject

```gherkin
Given a developer declares state names and a payload subject field on a signed webhook source
When verified requests concern different subjects
Then their state classification and distinct typed subjects reach the event stream
And other names remain occurrences
But a missing subject value is refused without relay
```

## Known caveats

- Project sources are executable; company/person source management is unsupported, so TeleClaude UC-IF16 is not imported as a company-registration feature.
- WhatsApp handshakes and multi-message delivery (UC-IF3, UC-IF4, UC-IF12) are outside this port and their IDs are not repurposed.
- Ingress needs the project's declaration and secret values locally. Producer registration, TLS and edge abuse protection are operator prerequisites.
- Retransmission is protected only within the three-day ingress claim window; consumers own business idempotency.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](single-ingress-webhook-termination.md)
- [project/instrukt/spec/feature/runner/github-webhook-normalization](github-webhook-normalization.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
