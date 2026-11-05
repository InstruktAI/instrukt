---
description: "instrukt's release and its installer — every version tag publishes one archive per supported platform, a checksum file and `install.sh`; the installer picks the computer's platform, verifies the download, installs the binary into the user's bin directory without touching anything else, and upgrades or pins by running it again."
delivered_by: [instrukt-first-build]
---
# Release Installer — Spec

## What it is

A developer, or an agent acting for one, gets the `instrukt` binary onto a
computer with one command, and gets a newer one the same way. The installer
is a POSIX shell script published with every release. It installs the release
built for the computer it runs on, refuses a download that does not match the
release's checksums, and writes only the binary. It is the one way instrukt is installed, by a developer and by the instrukt
skill (`project/instrukt/spec/feature/distribution/agent-skill`).

## Canonical fields

### The release

A push of a tag `v<version>` to `InstruktAI/instrukt` builds and publishes the
GitHub release of that tag (`.github/workflows/release.yml`). Its assets:

| Asset | Content |
| --- | --- |
| `instrukt-v<version>-<target>.tar.gz` | per supported target, a folder `instrukt-v<version>-<target>/` holding the `instrukt` binary and `README.md` (`scripts/package-release.sh`) |
| `SHA256SUMS` | one line per archive, `<sha256 hex>  <archive name>`, in the format `sha256sum -c` reads |
| `install.sh` | the installer, from `scripts/install.sh` at the tagged commit |

- Supported targets: `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`.
- The release is refused, and nothing is published, unless `<version>`
  equals the workspace version in `Cargo.toml` and the `metadata.version` of
  the instrukt skill (`skills/instrukt/SKILL.md`) at the tagged commit.
- A published release is never changed; a fix is a new version. There is one
  release channel: every release is the latest when it is published.

### Where it gets binaries

- Without a version, the installer resolves the latest release from
  `https://github.com/InstruktAI/instrukt/releases/latest` and downloads its
  assets from `https://github.com/InstruktAI/instrukt/releases/download/<tag>/`.
- `--version <tag>` (or `INSTRUKT_VERSION=<tag>`) pins that release; a tag
  without the leading `v` is accepted and the `v` added.
- It downloads over HTTPS with `curl`, or `wget` when `curl` is absent. It
  needs `tar` and one of `sha256sum` or `shasum -a 256`; a missing tool is
  named and nothing is downloaded.

### Platforms

| `uname -s` / `uname -m` | Target |
| --- | --- |
| `Darwin` / `arm64` | `aarch64-apple-darwin` |
| `Linux` / `x86_64` | `x86_64-unknown-linux-gnu` |
| `Linux` / `aarch64` or `arm64` | `aarch64-unknown-linux-gnu` |

Any other computer is refused, naming what it reported and the supported
targets. Nothing is downloaded or written.

### Verification

1. Download the target's archive and `SHA256SUMS` of the same release into a
   new private temporary directory.
2. The archive's digest must equal its line in `SHA256SUMS`; a missing line or
   a different digest refuses the install.
3. Unpack the archive in the temporary directory; it must hold exactly the
   expected folder with an `instrukt` file.
4. Run the unpacked `instrukt --version`; it must run and report the
   release's version. A binary that cannot run on this computer (a C library
   it does not have, for example) refuses the install.

Every refusal names the step and the asset, exits non-zero, removes the
temporary directory and leaves any installed binary as it was. The checksum
file and the archives come from the same release, so verification proves an
intact download of what that release published, not who published it.

### Where it installs

- The install directory is `$INSTRUKT_INSTALL_DIR` when set, otherwise
  `$XDG_BIN_HOME` when set, otherwise `~/.local/bin` (the XDG Base Directory
  specification's place for user executables). It is created when absent.
- The binary is written as `<dir>/instrukt`, by copying into a temporary name
  in that directory and renaming it over the old one, so a command running
  the old binary meanwhile never sees a partial file.
- The installer writes nothing else: no shell profile, no `~/.instrukt`, no
  agent configuration, no scheduler registration. It never asks for elevated
  rights.
- When the directory is not on the caller's `PATH`, the installer says so and
  prints the line that would add it; it changes no file to do that.
- It ends by naming the installed version and path, and the next steps:
  `instrukt -h`, and the command that installs the instrukt skill of the same
  release into the computer's agent CLIs
  (`npx skills add InstruktAI/instrukt@v<version> --skill instrukt -g -y`).

### Upgrade

Upgrading is running the installer again. With no version it installs the
latest release; with `--version` it installs exactly that one, older or
newer. When the binary in the install directory already reports the target
version, the installer says so and changes nothing. A failed upgrade leaves
the previous binary in place.

### Use cases

#### UC-RI1: A developer installs instrukt with one command

```gherkin
Given a developer on a supported computer without instrukt
When they run instrukt's release installer
Then the latest release built for their computer is downloaded and checked against the release's checksums
And `instrukt` is installed in their user bin directory and reports that release's version
And no other file on the computer has changed
And they are told the path, how to put it on their PATH when it is not, and that `instrukt -h` lists the commands
```

#### UC-RI2: A developer upgrades instrukt or pins a version

```gherkin
Given a developer has an older instrukt installed by the installer
When they run the installer again
Then the latest release replaces it and reports its version
When they run it again
Then they are told they already have that version and nothing changes
When they run it naming an earlier release
Then that release replaces it
```

#### UC-RI3: A download that does not match the release is refused

```gherkin
Given a developer has instrukt installed
And the archive they download does not match its line in the release's checksums
When they run the installer
Then it refuses, naming the archive whose checksum failed
And their installed instrukt is unchanged
```

#### UC-RI4: A computer the release is not built for is refused

```gherkin
Given a developer on a computer none of the release's targets supports
When they run the installer
Then it refuses, naming what their computer reported and the supported platforms
And nothing is downloaded or written
```

## Known caveats

- Intel macOS, Windows and musl-only Linux have no release target; the
  installer refuses them.
- Checksums prove integrity against the release, not authorship: trust rests
  on the GitHub release served over HTTPS.
- The installer does not remove instrukt; deleting the binary does.

## See Also

- project/instrukt/spec/domain/distribution
- project/instrukt/spec/feature/distribution/agent-skill
- project/instrukt/design/architecture/overview
