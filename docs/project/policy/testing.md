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
this suite; missing prerequisites fail a selected run. The pre-commit hook and the
default test commands run without a T3 release or provider.
Each feature use case is realized by exactly one named test checking its
complete situation and outcomes (`sdlc/policy/testing`, rule 8).

- This repository's pre-commit check runs what the content of the commit
  needs, without services. `.pre-commit-config.yaml` assigns each check the
  pattern of the files it applies to, and a check runs when the commit adds,
  changes, renames or deletes a file its pattern matches: the document
  validation for documents, shellcheck for shell scripts and hook files, the
  configuration validation for `instrukt.yaml`, and fmt, clippy and the
  colocated unit tests for everything a crate is built from and everything
  its unit tests read: its sources and manifests, the files compiled into the
  program, the unit tests' snapshots, and the two inputs that live outside the
  crates' folders: the shared test support under `tests/support/`, which is
  compiled into unit tests, and the workflow flavor under
  `docs/domain/sdlc/workflow/`, which the `instrukt-todos` unit tests run
  against. No commit runs the functional tests. These are project tooling, not
  quality hooks installed by instrukt in adopting repositories.
- A commit's checks take under ten seconds once what the commit changed is
  built. The budget counts running the validators, the linter and the unit
  tests; it does not count the compiler rebuilding changed crates. That
  rebuild is limited to the crates the commit touches and the crates that
  depend on them, and a commit that touches nothing a crate is built from or
  tested against builds nothing.
- GitHub CI runs that configuration over every file, and the whole test
  suite, on every pull request, on Linux and macOS, exercising filesystem case
  behavior, hooks and paths. `main` requires these checks, so a functional
  test that fails stops a pull request, not a commit.
- A version tag produces release binaries for macOS arm64 and Linux
  x86_64/arm64, and only from a revision that passed those checks.
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
boundaries. The write fence stops what instrukt itself attempts against a real
location, as the file system stands when it attempts it, so a test never fails
because another program changed that location. It is not a sandbox: the
programs instrukt starts and the test's own fixtures stay inside the temporary
home by construction. A path that another process retargets between
instrukt's check and its write, and a hard link inside the temporary home to
a file in a real location, are outside what the fence stops. A refusal reaches
the test through a record the test owns: instrukt appends to it before it does
anything else about the refusal, and removes the record when it cannot append,
so a missing record fails the test as a written one does. Two refusals leave
no trace in the record: one by a process stopped between refusing a write and
recording it, and one whose record can be neither appended to nor removed. In
both the write that was attempted did not land. Independently authored use cases supply the expected outcomes;
pure-logic units and property tests cover the Rust exception without replacing
feature-level evidence.

## Scope

All test authorship and test execution in the instrukt repository. This ruling
does not set the testing policy of repositories that adopt instrukt.

## Enforcement

Pre-commit and CI run the checks above; review checks what they cannot:
isolation, network-boundary fakes and scenario provenance.

## Exceptions

- The test harness's safety self-tests, which prove that the write fence refuses an instrukt write outside the temporary home before it lands (directly or through a symbolic link), bind no use case. They live beside the harness in `crates/instrukt-cli/tests/`. The exception covers only tests of the fence itself; any other harness behaviour is checked through the use cases that run under it.
- In pure colocated unit tests, equality between a whole returned identifier, path or node id and the expected value is a value assertion, not a `TQ-07` string-content assertion. Substring, prefix, suffix or pattern assertions on such values remain `TQ-07` findings, and rendered text and messages stay governed by `TQ-07` and the CLI snapshot rule.
- In CLI-crate integration tests, an `insta` snapshot may record the GitHub state the local GitHub fake holds after a command (issues, comments, project board fields and items), because that state is the outcome a developer sees on GitHub. It never records the requests themselves, never replaces assertions on CLI output, and never snapshots instrukt's internal state.
- In CLI-crate integration tests, an `insta` snapshot may record a file a command writes for a developer to read, such as the evaluation report, because that file is the command's output. The test makes the file deterministic (a controlled clock and fixed inputs), and the snapshot never records instrukt's internal state.
- In CLI-crate integration tests, an `insta` snapshot may record the input a conversation delivered to the runtime fake, because that input is what the company's agent reads. The test makes it deterministic (fixed message times and names, the delivery's boundary token replaced by a placeholder). The snapshot never records the requests to a provider, the messages a provider fake holds or instrukt's internal state.
