# instrukt

Context engineering for coding agents: documentation as the way agents are
instructed — snippets, progressive disclosure, docs before code, and
publication to the agents a machine runs — with optional modules for a work
record, a workflow engine and event-driven standing agents.

The repository is documentation-first. Start with
`docs/local/spec/domain/context-map.md` and
`docs/local/design/architecture/overview.md`.

## Development

One Cargo workspace, one binary `instrukt` (`crates/instrukt-cli`), with
`instrukt-core`, `instrukt-blueprint`, `instrukt-runner` and `instrukt-todos`
beside it.

```sh
make hooks   # once per clone: pre-commit runs fmt, clippy and test
make check   # the same checks by hand
```

Command-line tests run the built binary against a temporary `HOME` and
temporary git repositories (`crates/instrukt-cli/tests/common`); they never
touch the live home or agent settings.
