"""Stands in front of a real Redis and answers as Redis does while it loads.

Started by the `redis-server` stand-in with the server's configuration file,
whose `unixsocket` is the socket clients use, the real server's own socket and
a control directory. While `<control>/loading` exists,
every command but connection setup is answered as a loading Redis answers it:
`-LOADING`, and `INFO` reporting `loading:1`. Each such answer is counted in
`<control>/asked`. Once the file is gone, commands pass to the real server.
The stand-in ends when the real server does: its parent is the server, which
the stand-in's shell became, so a new parent means the server is gone.
"""

import os
import socket
import sys
import threading

configuration, inner_path, control = sys.argv[1], sys.argv[2], sys.argv[3]
with open(configuration) as file:
    listen_path = next(
        line.split(None, 1)[1].strip().strip('"')
        for line in file
        if line.startswith("unixsocket ")
    )
parent = os.getppid()
loading = os.path.join(control, "loading")
asked = os.path.join(control, "asked")
lock = threading.Lock()

# Connection setup a loading Redis still serves.
SETUP = {b"AUTH", b"HELLO", b"CLIENT", b"SELECT"}


def read_line(stream):
    line = stream.readline()
    if not line:
        raise EOFError
    return line


def read_reply(stream):
    """One RESP reply, as the bytes it arrived as."""
    line = read_line(stream)
    kind = line[:1]
    if kind in b"+-:_#,(":
        return line
    if kind in b"$=!":
        length = int(line[1:])
        return line if length < 0 else line + stream.read(length + 2)
    if kind in b"*~>%|":
        count = int(line[1:])
        if count < 0:
            return line
        if kind in b"%|":
            count *= 2
        return line + b"".join(read_reply(stream) for _ in range(count))
    raise ValueError(f"unknown reply {line!r}")


def read_command(stream):
    """One command: its bytes and its upper-cased name."""
    line = read_line(stream)
    if not line.startswith(b"*"):
        return line, line.split()[0].upper()
    parts = [line]
    name = None
    for _ in range(int(line[1:])):
        header = read_line(stream)
        body = stream.read(int(header[1:]) + 2)
        parts += [header, body]
        if name is None:
            name = body[:-2].upper()
    return b"".join(parts), name


def count_answer():
    with lock:
        try:
            with open(asked) as file:
                served = int(file.read() or 0)
        except FileNotFoundError:
            served = 0
        with open(asked + ".next", "w") as file:
            file.write(str(served + 1))
        os.replace(asked + ".next", asked)


def serve(client):
    with client:
        inner = socket.socket(socket.AF_UNIX)
        inner.connect(inner_path)
        with inner:
            from_client = client.makefile("rb")
            from_inner = inner.makefile("rb")
            while True:
                try:
                    command, name = read_command(from_client)
                except (EOFError, ValueError, OSError):
                    return
                if os.path.exists(loading) and name not in SETUP:
                    if name == b"INFO":
                        body = b"# Persistence\r\nloading:1\r\n"
                        answer = b"$%d\r\n%s\r\n" % (len(body), body)
                    else:
                        answer = b"-LOADING Redis is loading the dataset in memory\r\n"
                    count_answer()
                    client.sendall(answer)
                    continue
                inner.sendall(command)
                try:
                    client.sendall(read_reply(from_inner))
                except (EOFError, OSError):
                    return


def watch_parent():
    while os.getppid() == parent:
        threading.Event().wait(0.05)
    os._exit(0)


def main():
    threading.Thread(target=watch_parent, daemon=True).start()
    # The real server's socket exists once it listens.
    while not os.path.exists(inner_path):
        threading.Event().wait(0.01)
        if os.getppid() != parent:
            return
    try:
        os.unlink(listen_path)
    except FileNotFoundError:
        pass
    server = socket.socket(socket.AF_UNIX)
    server.bind(listen_path)
    os.chmod(listen_path, 0o600)
    server.listen(64)
    while True:
        client, _ = server.accept()
        threading.Thread(target=serve, args=(client,), daemon=True).start()


main()
