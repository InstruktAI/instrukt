---
description: Instrukt opts into test authorship with isolated real-binary CLI tests and spec-bound behavior.
---
# Testing — Policy

## Rules

### Project opt-in and scope

This repository authors new tests under the opt-in doctrine in
`sdlc/policy/testing`. It adapts the generic Rust discipline in
`sdlc/policy/rust/testing` to instrukt's CLI and repository boundaries.

- Pure logic has colocated unit tests: frontmatter, interpolation, runtime
  selection, index patching and configuration-layer validation. These are the
  Rust unit-test exception and do not require use-case bindings.
- Feature-spec behavior has integration tests that run the real `instrukt`
  binary, following the command-line rules of `sdlc/policy/rust/testing`.
  They use the CLI crate's single `crates/instrukt-cli/tests/integration.rs`
  target, with feature cases in `crates/instrukt-cli/tests/cases/<feature>.rs`;
  only that package's tests can locate the built binary. Live configuration,
  agent entrypoints and settings are never test destinations.
- CLI behavior uses no in-process mocks. For GitHub, the only legitimate fake
  is a local HTTP server standing in for GitHub's API at the network line;
  the client remains real (`sdlc/concept/functional-test-boundary`). The GitHub
  API address is configurable (`github.api_url`, global layer) so tests and
  self-hosted GitHub can point it elsewhere.
- Tests bind feature-spec use cases authored before the build by someone other
  than the builder. Rust bindings use `// spec: <spec_id> <UC-id>`.
  `instrukt specs coverage --check` checks them in CI once the command exists;
  until then, review checks them by hand.
- `insta` is limited to CLI text output, the contract agents read. `proptest`
  exercises invariants such as patch idempotence. There is no coverage gate.

### Checks and release

The T3 release-contract suite is a bounded external-system check under
`project/instrukt/spec/feature/runner/t3-contract`. Its dedicated CI jobs run
the real headless release and verify the existing T3 fake on the exercised
surfaces, with provider-dependent exclusions reported explicitly. CLI-facing
checks use the real binaries; adapter-only checks use existing public interfaces
without implementation visibility changes. The `t3-contract` feature selects
this suite; missing prerequisites fail a selected run. Ordinary hook/test commands
keep their current scope and gain no release download or provider dependency.
Each newly authored feature use case has one named test checking its complete
situation and outcomes. This ruling adds no other test-level, unit-test or
repository-wide fake architecture decision.

- This repository's pre-commit checks run fmt, clippy and test, in seconds
  without services. These are project tooling, not quality hooks installed by
  instrukt in adopting repositories.
- GitHub CI runs fmt, clippy and test on Linux and macOS, exercising filesystem
  case behavior, hooks and paths.
- A version tag produces release binaries for macOS arm64 and Linux
  x86_64/arm64.
- Parity against the corpus instrukt was extracted from is a one-off
  acceptance run, not a CI dependency.
- Code is synchronous by default; a crate that needs concurrent network I/O
  keeps it behind its own interface. Clippy runs with `-D warnings`, denying
  `dbg_macro` and `print_stdout` outside the command output writer.

Subfunctions (pending-doc folding, graph status, tick status and eval
fingerprint computation) are exercised by the stakeholder use cases that
reach them; they introduce no separate use cases. Session reuse, availability
selection, direct-main integration and conflict refresh are exercised within
the dispatch, integration and record-recovery goals.

## Rationale

The live T3 boundary supplies independent evidence for the adapter and its fake;
a fake agreeing with itself cannot detect upstream drift. Isolating the release
suite keeps ordinary checks independent of release availability and credentials.
One test per use-case situation keeps its acceptance evidence predictable and
reviewable without splitting the required outcomes among several bindings.

The real CLI, filesystem and git boundaries are the behavior developers adopt.
Isolating them in temporary homes protects the machine while preserving those
boundaries. Independently authored use cases supply the expected outcomes;
pure-logic units and property tests cover the Rust exception without replacing
feature-level evidence.

## Scope

All test authorship and test execution in the instrukt repository. This ruling
does not set the testing policy of repositories that adopt instrukt.

## Enforcement

Pre-commit and CI run the checks above; review checks what they cannot:
isolation, network-boundary fakes and scenario provenance.

## Exceptions

- The test harness's safety self-tests, which prove that the temporary-home guard detects writes outside the temporary home (directly or through a symbolic link), bind no use case. They live beside the harness in `crates/instrukt-cli/tests/`. The exception covers only tests of the guard itself; any other harness behaviour is checked through the use cases that run under it.
- In pure colocated unit tests, equality between a whole returned identifier, path or node id and the expected value is a value assertion, not a `TQ-07` string-content assertion. Substring, prefix, suffix or pattern assertions on such values remain `TQ-07` findings, and rendered text and messages stay governed by `TQ-07` and the CLI snapshot rule.
- In CLI-crate integration tests, an `insta` snapshot may record the GitHub state the local GitHub fake holds after a command (issues, comments, project board fields and items), because that state is the outcome a developer sees on GitHub. It never records the requests themselves, never replaces assertions on CLI output, and never snapshots instrukt's internal state.
- In CLI-crate integration tests, an `insta` snapshot may record a file a command writes for a developer to read, such as the evaluation report, because that file is the command's output. The test makes the file deterministic (a controlled clock and fixed inputs), and the snapshot never records instrukt's internal state.
