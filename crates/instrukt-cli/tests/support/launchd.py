"""Process-line launchd fixture: retain loaded definitions and listener custody."""
from dataclasses import dataclass
from enum import StrEnum
import json
import os
import plistlib
import select
import signal
import socket
import sys
from typing import TypedDict, cast


class Definition(TypedDict):
    Label: str
    ProgramArguments: list[str]
    EnvironmentVariables: dict[str, str]
    Sockets: dict[str, dict[str, str | int]]


class JobRow(TypedDict):
    unit: str
    pid: int | None


class Reply(TypedDict, total=False):
    ok: bool
    pid: int | None
    definition: Definition
    custody: int
    queued: bool
    jobs: list[JobRow]


class Operation(StrEnum):
    LOAD = "load"
    LIST = "list"
    KICKSTART = "kickstart"
    UNLOAD = "unload"
    INSPECT = "inspect"
    SHUTDOWN = "shutdown"
    DEBUG = "debug"
    FORCE_LEGACY = "force-legacy"
    IGNORE_DEBUG = "ignore-debug"
    STOP_PROCESS = "stop-process"


@dataclass
class Job:
    definition: Definition
    pid: int | None = None
    listener: socket.socket | None = None
    override: Definition | None = None
    custody: int = 0


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
    definition = job.override or job.definition
    job.override = None  # launchctl debug applies to the next invocation only.
    pid = os.fork()
    if pid == 0:
        listener = job.listener
        assert listener is not None
        if listener.fileno() != 3:
            os.dup2(listener.fileno(), 3)
        os.set_inheritable(3, True)
        environment = dict(os.environ, **definition["EnvironmentVariables"])
        environment.update(LISTEN_PID=str(os.getpid()), LISTEN_FDS="1", LISTEN_FDNAMES="instrukt")
        # Native cached argv/env are deliberately never re-read from disk.
        argv = definition["ProgramArguments"]
        os.execve(argv[0], argv, environment)
    job.pid = pid


def serve(address: str) -> None:
    jobs: dict[str, Job] = {}
    ignore_debug = False
    generation = 0
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
                        generation += 1
                        job = Job(definition, custody=generation)
                        jobs[definition["Label"]] = job
                        if "Sockets" in definition:
                            listener = socket.socket()
                            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                            listener.bind(("127.0.0.1", int(definition["Sockets"]["instrukt"]["SockServiceName"])))
                            listener.listen()
                            job.listener = listener
                            launch(job)
                    elif operation == Operation.DEBUG:
                        if not ignore_debug:
                            job = jobs[args[1].split("/")[-1]]
                            boundary = args.index("--")
                            environment = dict(job.definition["EnvironmentVariables"])
                            for assignment in args[args.index("--environment") + 1:boundary]:
                                key, value = assignment.split("=", 1)
                                environment[key] = value
                            override = job.definition.copy()
                            override["EnvironmentVariables"] = environment
                            override["ProgramArguments"] = args[boundary + 1:]
                            assert override["ProgramArguments"][0] == args[args.index("--program") + 1]
                            job.override = override
                    elif operation == Operation.FORCE_LEGACY:
                        job = jobs[args[-1]]
                        job.override = None
                        launch(job)
                    elif operation == Operation.IGNORE_DEBUG:
                        ignore_debug = args[-1] == "on"
                    elif operation == Operation.STOP_PROCESS:
                        stop(jobs[args[-1]])
                    elif operation == Operation.LIST:
                        if len(args) == 1:
                            result["jobs"] = [JobRow(unit=name, pid=job.pid) for name, job in jobs.items()]
                        else:
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
                        result.update(pid=job.pid, definition=job.definition, custody=job.custody,
                            queued=bool(job.listener and select.select([job.listener], [], [], 0)[0]))
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
    if sys.argv[3:] == [Operation.LIST]:
        print("PID\tStatus\tLabel")
        for row in result["jobs"]:
            print(f"{row['pid'] or '-'}\t0\t{row['unit']}")
    else:
        print(json.dumps(result))
    sys.exit(0 if result["ok"] else 1)
