# instrukt

```sh
curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh
npx skills add InstruktAI/instrukt@v0.1.0 --skill instrukt -g -y
```

The installer prints the skill command pinned to the release it installed.
Run `instrukt -h` for commands.

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
project whose `docs/general/` and `docs/domain/<x>/` folders are what it
publishes, adopted with `instrukt init`. Its sync publishes them into
`~/.instrukt/docs/` on the computer, and every project there reads them. Our docs repository publishes
`general` and `sdlc`; fork it, change it or bring your own
(`docs/project/spec/feature/publication/docs-repositories.md`). Until it is split
out, this repository is that docs repository: its `docs/general/` and
`docs/domain/sdlc/` are the doctrine it publishes.

## Development

One Cargo workspace, one binary `instrukt` (`crates/instrukt-cli`), with
`instrukt-core`, `instrukt-blueprint`, `instrukt-runner` and `instrukt-todos`
beside it.

```sh
make hooks          # once per clone: pre-commit runs the checks the staged files need
make commit-checks  # every commit check over every file, as CI runs it
make check          # lint, the whole test suite, coverage and the audit by hand
```

The commit check uses the pre-commit framework at the version pinned in
`scripts/test-tools.env` (`uv tool install pre-commit==<version>`).
`.pre-commit-config.yaml` assigns each check the files it applies to; the
functional suite runs in CI on every pull request. The Python lint and type
check are the framework's own hooks for ruff and mypy at pinned commits.
`make hooks` installs them into the framework's cache (`~/.cache/pre-commit`),
which needs network access. A commit made before that, or after a pinned tool
changes, installs what it needs itself and takes longer that one time;
`make commit-checks` and CI do the same.

Command-line tests run the built binary against a temporary `HOME` and
temporary git repositories (`crates/instrukt-cli/tests/common`); they never
touch the live home or agent settings.
