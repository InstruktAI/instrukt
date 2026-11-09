"""Process-line launchd fixture: retain loaded definitions and listener custody."""
from dataclasses import dataclass
from enum import StrEnum
import json
import os
import plistlib
import signal
import socket
import sys
from typing import TypedDict, cast


class Definition(TypedDict):
    Label: str
    ProgramArguments: list[str]
    EnvironmentVariables: dict[str, str]
    Sockets: dict[str, dict[str, str | int]]


class Reply(TypedDict, total=False):
    ok: bool
    pid: int | None
    definition: Definition


class Operation(StrEnum):
    LOAD = "load"
    LIST = "list"
    KICKSTART = "kickstart"
    UNLOAD = "unload"
    INSPECT = "inspect"
    SHUTDOWN = "shutdown"


@dataclass
class Job:
    definition: Definition
    pid: int | None = None
    listener: socket.socket | None = None


def rpc(address: str, args: list[str]) -> Reply:
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(address)
        connection.sendall(json.dumps(args).encode() + b"\n")
        return cast(Reply, json.loads(connection.makefile().readline()))


def stop(job: Job) -> None:
    if job.pid is not None:
        try:
            os.kill(job.pid, signal.SIGKILL)
            os.waitpid(job.pid, 0)
        except ProcessLookupError:
            job.pid = None
        job.pid = None


def launch(job: Job) -> None:
    stop(job)
    pid = os.fork()
    if pid == 0:
        listener = job.listener
        assert listener is not None
        if listener.fileno() != 3:
            os.dup2(listener.fileno(), 3)
        os.set_inheritable(3, True)
        environment = dict(os.environ, **job.definition["EnvironmentVariables"])
        environment.update(LISTEN_PID=str(os.getpid()), LISTEN_FDS="1", LISTEN_FDNAMES="instrukt")
        # Native cached argv/env are deliberately never re-read from disk.
        argv = job.definition["ProgramArguments"]
        os.execve(argv[0], argv, environment)
    job.pid = pid


def serve(address: str) -> None:
    jobs: dict[str, Job] = {}
    with socket.socket(socket.AF_UNIX) as control:
        control.bind(address)
        control.listen()
        try:
            while True:
                connection, _ = control.accept()
                with connection:
                    args = cast(list[str], json.loads(connection.makefile().readline()))
                    operation = Operation(args[0])
                    result: Reply = {"ok": True}
                    if operation == Operation.SHUTDOWN:
                        try:
                            connection.sendall(json.dumps(result).encode() + b"\n")
                        except BrokenPipeError:
                            return  # The shutdown caller already closed its connection.
                        break
                    if operation == Operation.LOAD:
                        with open(args[-1], "rb") as source:
                            definition = cast(Definition, plistlib.load(source))
                        job = Job(definition)
                        jobs[definition["Label"]] = job
                        if "Sockets" in definition:
                            listener = socket.socket()
                            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                            listener.bind(("127.0.0.1", int(definition["Sockets"]["instrukt"]["SockServiceName"])))
                            listener.listen()
                            job.listener = listener
                            launch(job)
                    elif operation == Operation.LIST:
                        result["ok"] = args[-1] in jobs
                    elif operation == Operation.KICKSTART:
                        launch(jobs[args[-1].split("/")[-1]])
                    elif operation == Operation.UNLOAD:
                        name = os.path.basename(args[-1]).removesuffix(".plist")
                        previous = jobs.pop(name, None)
                        if previous is not None:
                            stop(previous)
                            if previous.listener is not None:
                                previous.listener.close()
                    elif operation == Operation.INSPECT:
                        job = jobs[args[-1]]
                        result.update(pid=job.pid, definition=job.definition)
                    connection.sendall(json.dumps(result).encode() + b"\n")
        finally:
            for job in jobs.values():
                stop(job)
                if job.listener is not None:
                    job.listener.close()


if sys.argv[1] == "serve":
    serve(sys.argv[2])
else:
    result = rpc(sys.argv[2], sys.argv[3:])
    print(json.dumps(result))
    sys.exit(0 if result["ok"] else 1)
