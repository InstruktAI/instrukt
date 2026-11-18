#!/usr/bin/env python3
"""Prints the cargo package arguments of the Rust check for the given paths.

Each path selects the workspace crate whose directory holds it, whether the
path still exists or the commit removed it. A path under a directory outside
the crates' folders that crates compile or read selects those crates, as
OUTSIDE_INPUTS names them. Any other path outside every crate,
such as the workspace manifest, the lock file or the toolchain and clippy
configuration, selects every crate. Every crate that depends on a selected
crate is selected too. Three lines are printed, each of `-p <crate>`
arguments: every selected crate, those with a library target, those with a
binary target. Nothing is printed when no path is given.

    rust-crates.py [PATH ...]
"""

import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import PurePath
from typing import TypedDict, cast


class Target(TypedDict):
    kind: list[str]


class Dependency(TypedDict):
    name: str


class Package(TypedDict):
    name: str
    manifest_path: str
    targets: list[Target]
    dependencies: list[Dependency]


class Metadata(TypedDict):
    workspace_root: str
    packages: list[Package]


# Directories outside the crates' folders that crates compile or read, with
# those crates; each entry cites its readers.
OUTSIDE_INPUTS: dict[PurePath, tuple[str, ...]] = {
    # crates/instrukt-blueprint/src/lib.rs (a #[path] module of its unit tests) and
    # crates/instrukt-cli/tests/common/mod.rs (its integration harness, which
    # clippy checks with the crate's other targets).
    PurePath("tests/support"): ("instrukt-blueprint", "instrukt-cli"),
    # crates/instrukt-todos/src/workflow/_testing.rs: the real flavor's authoring source.
    PurePath("docs/domain/sdlc/workflow"): ("instrukt-todos",),
}


@dataclass(frozen=True)
class Crate:
    name: str
    directory: PurePath
    kinds: frozenset[str]
    depends_on: frozenset[str]


def workspace_crates() -> list[Crate]:
    # Cargo is the only source of the workspace's crates and their
    # dependencies; it has no Python interface.
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        capture_output=True,
        check=True,
        text=True,
    ).stdout
    metadata = cast(Metadata, json.loads(output))
    root = PurePath(metadata["workspace_root"])
    return [
        Crate(
            name=package["name"],
            directory=PurePath(package["manifest_path"]).parent.relative_to(root),
            kinds=frozenset(kind for target in package["targets"] for kind in target["kind"]),
            depends_on=frozenset(dependency["name"] for dependency in package["dependencies"]),
        )
        for package in metadata["packages"]
    ]


def owners(path: PurePath, crates: list[Crate]) -> set[str]:
    """The crates a path belongs to; empty for a path outside every crate and outside input."""
    for directory, readers in OUTSIDE_INPUTS.items():
        if path.is_relative_to(directory):
            return set(readers)
    return {crate.name for crate in crates if crate.directory.parts and path.is_relative_to(crate.directory)}


def selected(paths: list[PurePath], crates: list[Crate]) -> set[str]:
    chosen: set[str] = set()
    for path in paths:
        found = owners(path, crates)
        if not found:
            return {crate.name for crate in crates}
        chosen |= found
    grown = True
    while grown:
        dependents = {crate.name for crate in crates if crate.name not in chosen and crate.depends_on & chosen}
        chosen |= dependents
        grown = bool(dependents)
    return chosen


def flags(names: list[str]) -> str:
    return " ".join(f"-p {name}" for name in sorted(names))


def main(argv: list[str]) -> None:
    if not argv:
        return
    crates = workspace_crates()
    chosen = selected([PurePath(path) for path in argv], crates)
    picked = [crate for crate in crates if crate.name in chosen]
    print(flags([crate.name for crate in picked]))
    print(flags([crate.name for crate in picked if "lib" in crate.kinds]))
    print(flags([crate.name for crate in picked if "bin" in crate.kinds]))


if __name__ == "__main__":
    main(sys.argv[1:])
