"""TCP-to-Unix socket relay for exposing IPC sockets from containers.

Runs inside the container, bridging a TCP listener to the styrened
Unix socket. Each accepted TCP connection is relayed bidirectionally
to a fresh Unix socket connection.

Usage:
    python3 tcp_relay.py <unix_socket_path> <tcp_port>
"""
from __future__ import annotations

import asyncio
import sys


async def relay(reader, writer, label):
    """Copy data from reader to writer until EOF."""
    try:
        while True:
            data = await reader.read(65536)
            if not data:
                break
            writer.write(data)
            await writer.drain()
    except (ConnectionResetError, BrokenPipeError, asyncio.CancelledError):
        pass
    finally:
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass


async def handle_tcp_client(tcp_reader, tcp_writer, unix_path):
    """Handle a single TCP client by relaying to the Unix socket."""
    try:
        unix_reader, unix_writer = await asyncio.open_unix_connection(unix_path)
    except Exception as e:
        print(f"[relay] Failed to connect to {unix_path}: {e}", flush=True)
        tcp_writer.close()
        return

    # Relay bidirectionally
    t1 = asyncio.create_task(relay(tcp_reader, unix_writer, "tcp->unix"))
    t2 = asyncio.create_task(relay(unix_reader, tcp_writer, "unix->tcp"))

    done, pending = await asyncio.wait(
        [t1, t2], return_when=asyncio.FIRST_COMPLETED
    )
    for task in pending:
        task.cancel()
        try:
            await task
        except asyncio.CancelledError:
            pass


async def main(unix_path: str, tcp_port: int):
    """Start TCP relay server."""
    server = await asyncio.start_server(
        lambda r, w: handle_tcp_client(r, w, unix_path),
        host="0.0.0.0",
        port=tcp_port,
    )
    addr = server.sockets[0].getsockname()
    print(f"[relay] Listening on {addr[0]}:{addr[1]} -> {unix_path}", flush=True)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <unix_socket_path> <tcp_port>", file=sys.stderr)
        sys.exit(1)
    asyncio.run(main(sys.argv[1], int(sys.argv[2])))
