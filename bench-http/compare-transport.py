#!/usr/bin/env python3
"""Paired before/after adapter smoke benchmark; writes raw JSON lines.

Build and copy the old server binary before modifying the adapter. Pass the
old and new binaries as positional arguments. This is a local regression
probe, not a claim about production performance or other HTTP engines.
"""
import argparse
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time


def measure(binary, client, size, conns, seconds, post):
    env = dict(os.environ, BENCH_WORKERS="1", BENCH_BODY_SIZE=str(size))
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    env["BENCH_ADDR"] = f"127.0.0.1:{port}"
    with tempfile.TemporaryFile() as log:
        server = subprocess.Popen([binary], env=env, stdout=log, stderr=log)
        try:
            for _ in range(100):
                if server.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(log.read().decode())
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=.1):
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError("server failed to listen")
            # Check complete contents on repeated requests, not only length.
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            try:
                expected = (b"hello from the benchmark " * (size // 25 + 2))[:size]
                for _ in range(3):
                    conn.request("POST" if post else "GET", "/bench", b"x" * post)
                    response = conn.getresponse()
                    assert response.status == 200
                    assert response.read() == expected
            finally:
                conn.close()
            def drive(duration):
                output = subprocess.check_output(
                    [client, env["BENCH_ADDR"], str(conns), str(duration), str(post)],
                    env=env, text=True, timeout=duration + 15,
                )
                fields = dict(line.split(None, 1) for line in output.splitlines())
                assert int(fields["errors"]) == int(fields["mismatches"]) == 0, output
                return fields
            drive(1)
            return drive(seconds)
        finally:
            if server.poll() is None:
                server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("--client", default=str(Path(__file__).parent / "target/release/client"))
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=2)
    args = parser.parse_args()
    binaries = {name: str(Path(getattr(args, name)).resolve()) for name in ("before", "after")}
    # Turn interruption into an exception so the active server is reaped.
    def interrupted(_sig, _frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupted)
    for size, conns, post in [(24, 4, 0), (16384, 4, 0), (16384, 64, 0), (16384, 4, 4096)]:
        for rnd in range(args.rounds):
            order = ("before", "after") if rnd % 2 == 0 else ("after", "before")
            for name in order:
                result = measure(binaries[name], args.client, size, conns, args.seconds, post)
                print(json.dumps(dict(result, round=rnd + 1, variant=name, size=size,
                                      conns=conns, post=post)), flush=True)


if __name__ == "__main__":
    main()
