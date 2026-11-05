---
description: "Project-declared signed native and GitHub endpoints, scoped secret rotation and opaque event intake."
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
  native:
    secret: ${NATIVE_HOOK_SECRET}
    previous_secret: ${NATIVE_HOOK_PREVIOUS_SECRET}
  github:
    secret: ${GITHUB_HOOK_SECRET}
```

Sources sit directly under `webhooks`, with no direction or `sources` subkey.
Only `native` and `github` are admitted, at most one of each per registered
project. Project source configuration and `.env` must exist on ingress.

| Protocol | Derived POST path | Origin and subject |
| --- | --- | --- |
| Native | `/hooks/inbound/project/<slug>` | Project origin; `inbound_source:native:project:<slug>` subject |
| GitHub | `/hooks/inbound/github/project/<slug>` | Project origin; `inbound_source:github:project:<slug>` subject |

Paths derive from registration and protocol, never configurable callback URLs.
No event name, domain or secret is a URL parameter. Unsupported company/person
home declarations are explicit capability findings; they create no endpoint.
Unregistered projects and projects without that source have no endpoint.
An empty map creates none. Effective events-off sources are a plain finding and
are not served; source configuration never overrides module enablement.

### Secrets and per-request resolution

`secret` is required; `previous_secret` is optional. Both are raw `${NAME}`
references, never literal signing material. Raw-reference validation precedes
interpolation. On ingress each resolves solely from the declaring project's
`.env`, not process environment, receiver cwd or another project. A missing,
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

### Native protocol

The JSON object has exactly `domain`, `name`, `payload`. Domain is one segment;
name is one or more dot-separated segments. Each segment starts with `[a-z]`
and continues only with `[a-z0-9_]`. Payload is an object whose contents remain
opaque: instrukt interprets neither domain schema nor producer business meaning.
The signature header is `X-Hook-Signature: sha256=<hex digest>`. The emitted
type is `<domain>.<name>`; `payload` is its data. Unknown but well-formed external
names require no catalog entry.

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

### Use cases

#### UC-IF5: Use native and GitHub sources without a provider handshake

```gherkin
Given a developer has declared a native or GitHub endpoint
When a caller attempts a subscription verification handshake
Then no challenge is echoed and no event is relayed
```

#### UC-IF8: Correct malformed delivery input

```gherkin
Given a developer sends a correctly signed request with a malformed protocol body
When ingress receives it
Then it returns a client error and relays nothing
```

#### UC-IF17: Expose only declared endpoints

```gherkin
Given a developer has not registered a project or has not declared the requested source
When a request reaches its derived path
Then ingress returns not found and relays nothing
```

#### UC-IF18: Authenticate every request before processing it

```gherkin
Given a developer has a declared endpoint reachable even from loopback
When a request has a missing or invalid raw-body signature
Then it is unauthorized and no structured payload is parsed or relayed
```

#### UC-IF19: Configure signing material through a reference

```gherkin
Given a developer declares a webhook without a secret reference or with a literal secret
When configuration is validated
Then it is refused identifying the declaration without exposing signing material
```

#### UC-IF20: Add or remove a project source without restarting ingress

```gherkin
Given a developer has a running ingress
When they add, edit or remove a project native source in valid configuration
Then the next request uses that declaration without restart and a removed source has no endpoint
```

#### UC-IF21: Publish a domain fact without teaching instrukt its business schema

```gherkin
Given a developer sends a valid signed native wrapper with an undeclared domain event name and opaque payload
When ingress accepts it
Then the relayed type is domain dot name and its data is exactly the supplied payload
```

#### UC-IF22: Correct an invalid native event name

```gherkin
Given a developer sends a signed native wrapper whose domain has dots or whose name has an invalid or empty segment
When ingress checks the wrapper
Then it returns a client error and publishes no event
```

#### UC-IF23: Retry a delivery whose relay failed

```gherkin
Given a developer's source sends a verified delivery while cluster publication fails
When ingress answers
Then the answer is a transient failure and does not claim successful acceptance
```

#### UC-IF24: Diagnose an unreadable declaring project

```gherkin
Given a developer's registered source configuration or declared secret cannot be read on ingress
When a request reaches it
Then it receives a server error with no relay
And diagnostics identify the home and unresolved reference without a secret value
```

#### UC-IF25: Receive either GitHub content type

```gherkin
Given a developer sends the same GitHub document as JSON and as the sole form payload field with signatures over each original body
When ingress verifies the deliveries
Then both relay the same document and GitHub event meaning
```

#### UC-IF26: Reject an ambiguous GitHub form body

```gherkin
Given a developer sends a signed GitHub form with missing, repeated or additional payload fields or a nonobject document
When ingress receives it
Then it returns a client error and relays nothing
```

#### UC-IF27: Rotate signing secrets without dropping valid deliveries

```gherkin
Given a developer has current and previous secret references during rotation
When either secret signs a valid request
Then ingress accepts it without disclosing which secret matched
When the previous reference is removed
Then a request signed only by that old secret is rejected on the next request
```

#### UC-IF28: Keep a project signing secret isolated and redacted

```gherkin
Given two projects reuse a secret reference name with different values in their own environment files
When a developer delivers to one source and inspects configuration or diagnostics
Then only that project's value verifies and neither value is printed
And a process environment value cannot supply or override the source secret
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
