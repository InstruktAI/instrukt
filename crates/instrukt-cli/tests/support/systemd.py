"""Process-line systemd user manager fixture: `systemctl --user` and
`loginctl` at the process boundary. Unit files are read from the user unit
folder on `daemon-reload`; enabled, loaded and active are kept apart; a
service's ExecStart runs with its Environment lines; `Restart=` and
`RestartSec=` decide whether and when an exited process starts again.
Enabling a unit wants it from the targets its [Install] section names, and
nothing else: a unit without `WantedBy=default.target` is not enabled. A
simulated restart of the computer ends every process and starts the units
the user manager's default target wants."""
import json
import os
import shlex
import signal
import socket
import sys
import time
from dataclasses import dataclass, field
from typing import cast


@dataclass
class Unit:
    argv: list[str] = field(default_factory=list)
    environment: dict[str, str] = field(default_factory=dict)
    restart: str = "no"
    restart_sec: float = 0.1
    pid: int | None = None
    enabled: bool = False
    due: float | None = None
    starts: int = 0
    wanted_by: list[str] = field(default_factory=list)


def unquote(value: str) -> list[str]:
    return [word.replace("%%", "%") for word in shlex.split(value)]


def parse(path: str) -> Unit:
    unit = Unit()
    with open(path) as source:
        for line in source:
            key, _, value = line.strip().partition("=")
            if key == "ExecStart":
                unit.argv = unquote(value)
            elif key == "Environment":
                name, _, setting = unquote(value)[0].partition("=")
                unit.environment[name] = setting
            elif key == "Restart":
                unit.restart = value
            elif key == "RestartSec":
                unit.restart_sec = float(value)
            elif key == "WantedBy":
                unit.wanted_by.extend(value.split())
    return unit


def stop(unit: Unit) -> None:
    unit.due = None
    if unit.pid is not None:
        try:
            os.kill(unit.pid, signal.SIGKILL)
            os.waitpid(unit.pid, 0)
        except (ProcessLookupError, ChildProcessError):
            pass
        unit.pid = None


def start(unit: Unit) -> None:
    stop(unit)
    if not unit.argv:
        return
    unit.starts += 1
    pid = os.fork()
    if pid == 0:
        os.execve(unit.argv[0], unit.argv, dict(os.environ, **unit.environment))
    unit.pid = pid


def supervise(units: dict[str, Unit]) -> None:
    now = time.monotonic()
    for unit in units.values():
        if unit.pid is not None:
            try:
                finished, _ = os.waitpid(unit.pid, os.WNOHANG)
            except ChildProcessError:
                finished = unit.pid
            if finished:
                unit.pid = None
                if unit.restart == "always":
                    unit.due = now + unit.restart_sec
        elif unit.due is not None and now >= unit.due:
            start(unit)


def reload(folder: str, units: dict[str, Unit]) -> None:
    present = set()
    for name in sorted(os.listdir(folder)) if os.path.isdir(folder) else []:
        if not name.endswith(".service"):
            continue
        present.add(name)
        fresh = parse(os.path.join(folder, name))
        if name in units:
            # A running process keeps the command it was started with.
            old = units[name]
            old.argv, old.environment = fresh.argv, fresh.environment
            old.restart, old.restart_sec = fresh.restart, fresh.restart_sec
            old.wanted_by = fresh.wanted_by
        else:
            units[name] = fresh
    for name in list(units):
        if name not in present and units[name].pid is None and not units[name].enabled:
            del units[name]


def handle(args: list[str], folder: str, units: dict[str, Unit], state: dict[str, str]) -> dict:
    if args and args[0] == "--user":
        args = args[1:]
    quiet = "--quiet" in args
    now = "--now" in args
    args = [a for a in args if a not in ("--quiet", "--now")]
    operation, rest = args[0], args[1:]
    result: dict = {"ok": True, "stdout": ""}
    unit = units.get(rest[0]) if rest else None
    if operation == "daemon-reload":
        reload(folder, units)
    elif operation == "enable":
        reload(folder, units)
        target = units[rest[0]]
        # Enabling links the unit into the targets that want it; a unit
        # without an installation target stays not enabled.
        target.enabled = "default.target" in target.wanted_by
        if now and target.pid is None:
            start(target)
    elif operation == "disable":
        if unit is not None:
            unit.enabled = False
            if now:
                stop(unit)
    elif operation in ("start", "restart"):
        start(units[rest[0]])
    elif operation == "stop":
        if unit is not None:
            stop(unit)
    elif operation == "is-active":
        result["ok"] = unit is not None and unit.pid is not None
        result["stdout"] = "" if quiet else ("active" if result["ok"] else "inactive")
    elif operation == "is-enabled":
        result["ok"] = unit is not None and unit.enabled
        result["stdout"] = "" if quiet else ("enabled" if result["ok"] else "disabled")
    elif operation == "show":
        prop = next(a for a in rest if a.startswith("--property=")).split("=", 1)[1]
        if prop == "LoadState":
            result["stdout"] = "loaded" if unit is not None else "not-found"
        elif prop == "Environment":
            result["stdout"] = " ".join(f"{key}={value}" for key, value in (unit.environment if unit is not None else {}).items())
        elif prop == "MainPID":
            result["stdout"] = str(unit.pid if unit is not None and unit.pid else 0)
        elif prop == "Linger":
            result["stdout"] = state["linger"]
    elif operation == "show-user":
        result["stdout"] = state["linger"]
    elif operation == "linger":
        state["linger"] = rest[0]
    elif operation == "reboot":
        for each in units.values():
            stop(each)
        reload(folder, units)
        for each in units.values():
            if each.enabled and "default.target" in each.wanted_by:
                start(each)
    elif operation == "kill-process":
        if unit is not None and unit.pid is not None:
            os.kill(unit.pid, signal.SIGKILL)
    elif operation == "inspect":
        result["pid"] = unit.pid if unit is not None else None
        result["enabled"] = unit is not None and unit.enabled
        result["starts"] = unit.starts if unit is not None else 0
        result["environment"] = unit.environment if unit is not None else {}
    elif operation == "list-units":
        result["stdout"] = "".join(
            f"{name} loaded {'active' if each.pid else 'inactive'} running\n"
            for name, each in units.items()
            if each.pid is not None or each.enabled
        )
    else:
        result["ok"] = False
        result["stdout"] = f"unsupported: {operation}"
    return result


def serve(address: str, folder: str) -> None:
    units: dict[str, Unit] = {}
    state = {"linger": "no"}
    with socket.socket(socket.AF_UNIX) as control:
        control.bind(address)
        control.listen()
        control.settimeout(0.1)
        try:
            while True:
                supervise(units)
                try:
                    connection, _ = control.accept()
                except TimeoutError:
                    continue
                connection.settimeout(None)
                with connection:
                    args = cast(list[str], json.loads(connection.makefile().readline()))
                    if args == ["shutdown"]:
                        try:
                            connection.sendall(b'{"ok": true}\n')
                        except BrokenPipeError:
                            pass
                        return
                    reply = handle(args, folder, units, state)
                    connection.sendall(json.dumps(reply).encode() + b"\n")
        finally:
            for unit in units.values():
                stop(unit)


def rpc(address: str, args: list[str]) -> dict:
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(address)
        connection.sendall(json.dumps(args).encode() + b"\n")
        return cast(dict, json.loads(connection.makefile().readline()))


if sys.argv[1] == "serve":
    serve(sys.argv[2], sys.argv[3])
else:
    reply = rpc(sys.argv[2], sys.argv[3:])
    if reply.get("stdout"):
        print(reply["stdout"])
    sys.exit(0 if reply["ok"] else 1)
