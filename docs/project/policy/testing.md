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
- Logging is isolated through the library's production `INSTRUKT_LOG_DIR`
  variable, set to a per-test directory and shared by subprocesses, hooks,
  background runners and log readers. The logger creates it when writing;
  inspection leaves it absent. Log isolation changes neither `HOME` nor XDG
  bases (`sdlc/policy/testing`).
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

<!-- planned-change:ratchet-file-size-and-suppressions -->

- This repository's pre-commit check runs what the content of the commit
  needs, without services. `.pre-commit-config.yaml` assigns each check the
  pattern of the files it applies to, and a check runs when the commit adds,
  changes, renames or deletes a file its pattern matches: the document
  validation for documents, shellcheck for shell scripts and hook files, the
  lint and the type check for Python scripts, the configuration validation
  for `instrukt.yaml`, and fmt, clippy and the colocated unit tests for
  everything a crate is built from and everything its unit tests read: its
  sources and manifests, the files compiled into the program, the unit tests'
  snapshots, and the two inputs that live outside the crates' folders: the
  shared test support under `tests/support/`, which is compiled into unit
  tests, and the workflow flavor under `docs/domain/sdlc/workflow/`, which the
  `instrukt-todos` unit tests run against. No commit runs the functional
  tests. These are project tooling, not quality hooks installed by instrukt in
  adopting repositories.
- A commit's checks take under ten seconds once what the commit changed is
  built. The budget counts running the validators, the linter and the unit
  tests; it does not count the compiler rebuilding changed crates. That
  rebuild is limited to the crates the commit touches and the crates that
  depend on them, and a commit that touches nothing a crate is built from or
  tested against builds nothing.
- A check's tool that is not installed yet is installed when it is first
  needed: by `make hooks`, which installs the tools `.pre-commit-config.yaml`
  pins when it enables the hook, or by the first commit or every-file run
  that needs it. That installation and the tool's first run after it are not
  counted in the ten seconds. No commit is refused because a tool is missing.
- GitHub CI runs every check in `.pre-commit-config.yaml` over every file
  without running colocated unit tests more than once per selected platform
  or repeating OS-independent checks across platforms:
  - On every pull request, Linux runs the OS-independent checks once,
    doctests and the full workspace test suite (built once and sharded across
    parallel runners); macOS runs workspace clippy across all targets and the
    macOS-specific test suite (launchd, module installation, Darwin
    process-group cleanup, kqueue notifications, hooks and write-fence paths).
    `main` requires these pull-request checks: a failing Linux test or scoped
    macOS test blocks a pull request, not a commit.
  - On every `main` push and tagged release, macOS runs workspace clippy
    across all targets, the full workspace test suite and doctests; a failure
    in a non-platform-specific test that manifests only on macOS is detected
    by the full macOS job on `main` pushes and tagged releases and blocks
    tagged release publication, not pull requests.
  - A pull request runs, of the checks above, only those its changed files
    reach, as `scripts/ci-changes.sh` classifies them; a `main` push or tagged
    release runs every check. A shipped document or configuration file a
    workspace test reads is a Rust input, and a change to it runs the Rust
    checks alongside the document checks. A change confined to documents and
    configuration that no Rust check or test reads, root `instrukt.yaml`
    among them, reaches no Rust input:
    Linux builds the CLI binary and runs the document, configuration,
    use-case coverage and instruction-file checks with it,
    without the workspace test suite, doctests, macOS, the dependency audit,
    the pinned contracts, or the commit checks whose inputs did not change.
    Among the commit checks, fmt, clippy and the unit tests run when a Rust
    input changes, shellcheck when a script or a hook file changes, and
    the Python lint and type check when a Python file or their configuration
    changes. A contract check runs when a crate on its path, the workspace
    manifest or lock, or a pinned tool changes. A moved file counts as both
    the path it left and the path it reached. A path no rule names, a change
    whose files cannot be read, a classification that does not succeed, and
    a change to the pipeline, `scripts/ci-changes.sh`, or `scripts/test-tools.env`
    run every check. Every check `main` requires reports on every pull request,
    skipping its steps rather than itself. CI runs for a branch only through
    its pull request.
- A version tag produces release binaries for macOS arm64 and Linux
  x86_64/arm64, and only from a revision that passed those checks.
- Code is synchronous by default; a crate that needs concurrent network I/O
  keeps it behind its own interface. Clippy runs with `-D warnings`, denying
  `dbg_macro` and `print_stdout` outside the command output writer.

<!-- change:ratchet-file-size-and-suppressions -->

- This repository's pre-commit check runs what the content of the commit
  needs, without services. `.pre-commit-config.yaml` assigns each check the
  pattern of the files it applies to, and a check runs when the commit adds,
  changes, renames or deletes a file its pattern matches: the document
  validation for documents, shellcheck for shell scripts and hook files, the
  lint and the type check for Python scripts, the configuration validation
  for `instrukt.yaml`, the size and suppression ratchet for Rust, Python and
  shell files and its baseline
  (`project/instrukt/policy/size-and-suppression-ratchet`), and fmt, clippy
  and the colocated unit tests for
  everything a crate is built from and everything its unit tests read: its
  sources and manifests, the files compiled into the program, the unit tests'
  snapshots, and the two inputs that live outside the crates' folders: the
  shared test support under `tests/support/`, which is compiled into unit
  tests, and the workflow flavor under `docs/domain/sdlc/workflow/`, which the
  `instrukt-todos` unit tests run against. No commit runs the functional
  tests. These are project tooling, not quality hooks installed by instrukt in
  adopting repositories.
- A commit's checks take under ten seconds once what the commit changed is
  built. The budget counts running the validators, the linter and the unit
  tests; it does not count the compiler rebuilding changed crates. That
  rebuild is limited to the crates the commit touches and the crates that
  depend on them, and a commit that touches nothing a crate is built from or
  tested against builds nothing.
- A check's tool that is not installed yet is installed when it is first
  needed: by `make hooks`, which installs the tools `.pre-commit-config.yaml`
  pins when it enables the hook, or by the first commit or every-file run
  that needs it. That installation and the tool's first run after it are not
  counted in the ten seconds. No commit is refused because a tool is missing.
- GitHub CI runs every check in `.pre-commit-config.yaml` over every file
  without running colocated unit tests more than once per selected platform
  or repeating OS-independent checks across platforms:
  - On every pull request, Linux runs the OS-independent checks once,
    doctests and the full workspace test suite (built once and sharded across
    parallel runners); macOS runs workspace clippy across all targets and the
    macOS-specific test suite (launchd, module installation, Darwin
    process-group cleanup, kqueue notifications, hooks and write-fence paths).
    `main` requires these pull-request checks: a failing Linux test or scoped
    macOS test blocks a pull request, not a commit.
  - On every `main` push and tagged release, macOS runs workspace clippy
    across all targets, the full workspace test suite and doctests; a failure
    in a non-platform-specific test that manifests only on macOS is detected
    by the full macOS job on `main` pushes and tagged releases and blocks
    tagged release publication, not pull requests.
  - A pull request runs, of the checks above, only those its changed files
    reach, as `scripts/ci-changes.sh` classifies them; a `main` push or tagged
    release runs every check. A shipped document or configuration file a
    workspace test reads is a Rust input, and a change to it runs the Rust
    checks alongside the document checks. A change confined to documents and
    configuration that no Rust check or test reads, root `instrukt.yaml`
    among them, reaches no Rust input:
    Linux builds the CLI binary and runs the document, configuration,
    use-case coverage and instruction-file checks with it,
    without the workspace test suite, doctests, macOS, the dependency audit,
    the pinned contracts, or the commit checks whose inputs did not change.
    Among the commit checks, fmt, clippy and the unit tests run when a Rust
    input changes, shellcheck when a script or a hook file changes, the
    size and suppression ratchet when a Rust, Python or shell file, a hook
    file or the ratchet baseline changes, and the Python lint and type check
    when a Python file or their configuration changes. A change the ratchet
    reads starts the Linux build job, which runs it. A contract check runs when a crate on its path, the workspace
    manifest or lock, or a pinned tool changes. A moved file counts as both
    the path it left and the path it reached. A path no rule names, a change
    whose files cannot be read, a classification that does not succeed, and
    a change to the pipeline, `scripts/ci-changes.sh`, or `scripts/test-tools.env`
    run every check. Every check `main` requires reports on every pull request,
    skipping its steps rather than itself. CI runs for a branch only through
    its pull request.
- A version tag produces release binaries for macOS arm64 and Linux
  x86_64/arm64, and only from a revision that passed those checks.
- Code is synchronous by default; a crate that needs concurrent network I/O
  keeps it behind its own interface. Clippy runs with `-D warnings`, denying
  `dbg_macro` and `print_stdout` outside the command output writer.

<!-- /planned-change:ratchet-file-size-and-suppressions -->

Subfunctions (pending-doc folding, graph status, tick status and eval
fingerprint computation) are exercised by the stakeholder use cases that
reach them; they introduce no separate use cases. Session reuse, availability
selection and direct-main integration are exercised within the dispatch and
integration goals.

### Inherited home and isolated application roots

Checks inherit `HOME`, broad XDG bases and `TMPDIR` unchanged for signing,
SSH and caches. `INSTRUKT_LOG_DIR` selects the isolated app log directory
under `project/instrukt/spec/feature/configuration/logging`. They isolate
Instrukt, T3 and the six native runtime roots named by
`project/instrukt/spec/feature/publication/agent-entrypoints`. The shared check
launcher owns temporary roots and scheduler-command rejection stubs; CLI
fixtures retain their existing recording scheduler stubs and write fence.
Systemd uses an explicit `SYSTEMD_UNIT_PATH`, launchd uses
`INSTRUKT_LAUNCH_AGENTS_DIR`, and file-only cron uses `INSTRUKT_CRONTAB_FILE`.
Native-account cron checks unset the latter only behind a recording stub.
Cursor/Grok/Antigravity default-only outputs are not filesystem-delivery
fixtures against a real home. Existing pure/source evidence covers their
planning/format; relocated Claude/fx file checks do not prove context receipt.

`.pre-commit-config.yaml`'s document validation check runs `instrukt docs
validate --no-todo-lookup` (`project/instrukt/spec/feature/docs/snippet-validation`),
checking pending-slug form, marker structure, workflow roles and all other
structural rules without services.

## Rationale

The live T3 boundary supplies independent evidence for the adapter and its fake;
a fake agreeing with itself cannot detect upstream drift. Isolating the release
suite keeps ordinary checks independent of release availability and credentials.
One test per use-case situation keeps its acceptance evidence predictable and
reviewable without splitting the required outcomes among several bindings.
Running the document validation hook with `--no-todo-lookup` keeps commit checks
service-free: the isolated Instrukt root holds neither a work database nor
GitHub App configuration, while live todo-slug existence lookup stays on
configured interactive runs and `instrukt sync`.

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

A missing tool is installed, not asked for: installing it is idempotent, and
a refusal would only make a developer run the command the check could have
run itself.

Running OS-independent checks once on Linux, compiling the Linux test archive
once for parallel shards, and scoping macOS pull-request runs to
platform-specific surfaces keeps pull-request feedback fast while preserving
full cross-platform verification on `main` and every tagged release.
Running, of those checks, only the ones a pull request's files reach spares a
change to documents no test reads the workspace test suite, macOS and every
check its files do not reach. A document a test reads is that test's input:
sparing its change the suite would let a pull request pass that then turns
`main` red. An unnamed, unreadable or unclassified change runs everything, so a
gap in the classification costs time, never a missed check.

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
- In CLI-crate integration tests, an `insta` snapshot may record the messages the Slack fake received in a conversation, because those posts are what a person sees. The test makes them deterministic; the snapshot never records the requests themselves or instrukt's internal state.
