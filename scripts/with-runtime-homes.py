#!/usr/bin/env python3
"""Run a check with native application roots and a closed scheduler boundary."""
from __future__ import annotations

import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

ROOTS: dict[str, str] = {
    "INSTRUKT_HOME": "instrukt",
    "INSTRUKT_LOG_DIR": "logs",
    "T3CODE_HOME": "t3",
    "CODEX_HOME": "codex",
    "CLAUDE_CONFIG_DIR": "claude",
    "OPENCODE_CONFIG_DIR": "opencode",
    "FX_HOME": "fx",
    "KIRO_HOME": "kiro",
    "PI_CODING_AGENT_DIR": "pi",
    "SYSTEMD_UNIT_PATH": "systemd-user-units",
    "INSTRUKT_LAUNCH_AGENTS_DIR": "launch-agents",
    "INSTRUKT_CRONTAB_FILE": "crontab",
}


def isolated_environment(root: Path) -> dict[str, str]:
    """Preserve authentication/cache context while selecting exact output locations."""
    environment = os.environ.copy()
    environment.update({key: str(root / name) for key, name in ROOTS.items()})
    commands = root / "scheduler-commands"
    commands.mkdir()
    for name in ("launchctl", "systemctl", "loginctl", "crontab"):
        command = commands / name
        command.write_text(f"#!/bin/sh\necho 'check attempted scheduler command: {name}' >&2\nexit 1\n")
        command.chmod(0o755)
    environment["PATH"] = str(commands) + os.pathsep + environment.get("PATH", "")
    return environment


def interrupted(signum: int, _frame: object) -> None:
    raise SystemExit(128 + signum)


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: with-runtime-homes.py COMMAND [ARGUMENT ...]", file=sys.stderr)
        return 2
    signal.signal(signal.SIGTERM, interrupted)
    with tempfile.TemporaryDirectory(prefix="instrukt-check-") as directory:
        # The requested external command is the check boundary; it has no in-process API.
        result = subprocess.run(sys.argv[1:], env=isolated_environment(Path(directory)), check=False)
        return result.returncode if result.returncode >= 0 else 128 - result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
