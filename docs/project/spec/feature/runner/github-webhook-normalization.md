---
description: "Canonical GitHub delivery type and properties, preserving project source provenance and raw domain facts."
delivered_by: [instrukt-first-build]
---
# GitHub Webhook Normalization — Spec

## What it is

The built-in GitHub normalizer translates verified GitHub input through one
shared mapping. Projects declare their own GitHub source; procedures consume
ordinary `<domain>.<event>` platform facts (`github.<event>` for the `github`
source). Producers of ours use native mapping.

## Canonical fields

### Type, data and properties

Header names are case-insensitive. `X-GitHub-Event` supplies the lowercased event
type, with `unknown` for missing or malformed values. Body shape never guesses
or corrects that header. A normalized provider view uses `source=github`,
`type=<event>` and the original relayed timestamp. This provider label is not
the canonical envelope's emitting-computer `source`.

| Property | Extraction |
| --- | --- |
| `repo` | String `repository.full_name`, otherwise null |
| `sender` | String `sender.login`, otherwise null |
| `action` | String `action`, otherwise null |
| `ref` | String `ref`, otherwise null |

Those four slots are present even when null. The full parsed JSON object is
preserved as canonical event data and provider-view payload; properties are a
summary for a receiving reaction, not additional envelope routing criteria.
For `ping` only, string `zen` and integer/string `hook_id`, when present, are
passed as metadata. No built-in ping handler or installation updater is created.

The source declares `verify: github` at `/hooks/inbound/project/<slug>/<domain>`.
JSON and the sole form `payload` encoding produce the same `<domain>.<event>`
fact after raw-body verification. Its origin is the declaring project, subject
is `inbound_source:<domain>:project:<slug>` and source is ingress.
The `inbound_verify: github` extension selects provider interpretation even when
the domain is not `github`. A derived provider view names the repository and retains source provenance; it does not replace the
platform origin with the repository or enable property-based procedure routing.

## Use cases

### UC-GH1: Read the canonical facts of a GitHub push

```gherkin
Given a developer receives a verified push document
When their reaction reads its normalized provider view
Then source is github, type is push, repo, sender, action and ref are extracted and the full payload remains available
```

### UC-GH2: Read the canonical facts of a GitHub release

```gherkin
Given a developer receives a verified release document
When their reaction reads the normalized view
Then type is release and its repo, sender, action and nullable ref follow the same extraction as other events
```

### UC-GH3: Receive GitHub registration metadata without a built-in reaction

```gherkin
Given a developer receives a verified ping with zen and hook_id
When it is normalized
Then those metadata are retained with the common properties and no built-in ping handler is invoked
```

### UC-GH4: Keep an absent or malformed type explicit

```gherkin
Given a developer receives a verified delivery with no valid event-type header
When it is normalized
Then its type is unknown rather than inferred from the body
```

### UC-GH5: Accept GitHub header casing variants

```gherkin
Given a developer receives equivalent deliveries with differently cased header names and event values
When they are normalized
Then both yield the same lowercased GitHub event type
```

### UC-GH6: Handle incomplete repository and sender facts

```gherkin
Given a developer receives a valid delivery with malformed repository or sender structures and absent action or ref
When it is normalized
Then all four property slots exist and unavailable string facts are explicit nulls
```

### UC-GH8: Retain the project source of a verified GitHub delivery

```gherkin
Given a developer has declared a project GitHub source
When its verified delivery is relayed
Then the ordinary platform event has declared domain dot header type, that project origin, its typed GitHub-source subject and the parsed document as data
```

## Known caveats

- UC-GH7's TeleClaude self-deployment effect is not imported or repurposed.
- Provider headers are not independently authenticated by raw-body HMAC. Header/body contradictions remain visible; body structure never substitutes a different type.
- Common nullable properties do not enable payload matching in procedure subscription routing.
- The normalizer owns mapping only; it installs no maintenance bindings, provider registry or conversation handler.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/inbound-webhook-endpoint-framework](inbound-webhook-endpoint-framework.md)
- [project/instrukt/spec/feature/runner/event-envelope](event-envelope.md)
