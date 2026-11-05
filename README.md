# instrukt

Context engineering for coding agents: documentation as the way agents are
instructed — snippets, progressive disclosure, docs before code, and
publication to the agents a machine runs — with optional modules for a work
record, a workflow engine and event-driven standing agents.

The repository is documentation-first. Start with
`docs/project/spec/domain/context-map.md` and
`docs/project/design/architecture/overview.md`.

## Doctrine comes from a docs repository

instrukt ships no doctrine of its own. The principles, policies, procedures and
workflow definitions agents follow come from a docs repository: an ordinary
project that declares the domains it `publishes` and is adopted with
`instrukt init`. Its sync publishes those domains into `~/.instrukt/docs/` on
the computer, and every project there reads them. Our docs repository publishes
`general` and `sdlc`; fork it, change it or bring your own
(`docs/project/spec/feature/publication/docs-repositories.md`). Until it is split
out, this repository is that docs repository: its `docs/global/` is the
doctrine, and its `instrukt.yaml` declares `publishes`.

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
