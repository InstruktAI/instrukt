"""Process-line 676e017 receiver contract: inherited socket, unsigned 401, no invocation lookup."""
from http.server import BaseHTTPRequestHandler, HTTPServer
import socket


class LegacyReceiver(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        self.send_response(401)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, format: str, *args: str) -> None:
        return  # The fixture models protocol behavior without writing access logs.


with HTTPServer(("127.0.0.1", 0), LegacyReceiver, bind_and_activate=False) as server:
    server.socket.close()
    server.socket = socket.socket(fileno=3)
    server.server_address = server.socket.getsockname()
    server.serve_forever()
