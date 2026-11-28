#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Relay a leased serial console's MCP output into a file, for a frame dump.

    tools/cts/imx8mp-glmark2-console-relay.py <mcp url> <file> [--from-now]
    tools/cts/imx8mp-glmark2-console-relay.py --self-test

When the farm hands a board's console out as MCP tools only (the Armadillo-X2's
UART2: the lease's serial MCP server holds the adapter's one client, so
tools/board.sh's capture gets "busy"), the console reaches board.sh through the
feed bridge of tools/am62p_netboot_uart.py ("Feed bridge"): the console output
is appended to a file and `am62p_netboot_uart.py feed <file> <port>` serves it
as BOARD_UART=127.0.0.1:<port>. A glmark2 frame dump is 1.9 MB a scene, far
more than an agent can pass through its own reads, so this script is the
relay: it calls the MCP server's `serial_read` tool over HTTP (JSON-RPC
`tools/call`, the streamable HTTP transport answering with one JSON body) once
a second with `since` = the offset it has reached, and appends the bytes to
<file>. Read only: it never writes to the console.

<mcp url> is the serial MCP server's URL of this turn (the session's MCP
configuration names it; it is the environment's: never write it in a file, a
record or a commit). The server shows control bytes as \\xNN; they are turned
back into bytes. --from-now starts at the console's current offset instead of
the oldest byte the server still holds.

One line on stderr per event, never the URL: the start offset, and
"relay: LOST <n> bytes" when the server's buffer (1 MiB) dropped output before
it was read (a dump with that hole is damaged: re-run, never patch). Runs
until SIGTERM or SIGINT, then prints "relay: <n> bytes, lost <n>" and exits 0
(1 when bytes were lost); 2 on a bad argument or when the server cannot be
reached at the start. Stdlib only.
"""
import json
import re
import signal
import sys
import threading
import time
import urllib.request

ESC = re.compile(r"\\x([0-9a-fA-F]{2})")


def unescape(text):
    """The server's text (control bytes as \\xNN) back to bytes."""
    out = bytearray()
    pos = 0
    for m in ESC.finditer(text):
        out += text[pos:m.start()].encode("utf-8", "surrogateescape")
        out.append(int(m.group(1), 16))
        pos = m.end()
    out += text[pos:].encode("utf-8", "surrogateescape")
    return bytes(out)


def call(url, name, args, rid=1, timeout=30):
    body = json.dumps({"jsonrpc": "2.0", "id": rid, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(url, data=body, headers={
        "Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        raw = r.read().decode("utf-8", "replace")
    if raw.lstrip().startswith("event:") or raw.lstrip().startswith("data:"):
        raw = "\n".join(l[5:].strip() for l in raw.splitlines() if l.startswith("data:"))
    res = json.loads(raw).get("result", {})
    if "structuredContent" in res:
        return res["structuredContent"]
    return json.loads(res["content"][0]["text"])


def relay(url, path, from_now, stop, err=sys.stderr, pause=1.0):
    """Appends the console's bytes to path until stop is set. Returns (bytes, lost)."""
    try:
        st = call(url, "serial_status", {})
    except Exception as e:  # noqa: BLE001 - any failure here is "cannot reach"
        print("relay: the MCP server cannot be reached (%s)" % type(e).__name__, file=err)
        return None
    off = int(st.get("current_offset", 0)) if from_now else int(st.get("oldest_offset", 0))
    print("relay: from offset %d" % off, file=err, flush=True)
    total = lost = 0
    with open(path, "ab", buffering=0) as f:
        while True:
            try:
                r = call(url, "serial_read", {"since": off, "timeout_ms": 1000, "max_bytes": 262144})
            except Exception as e:  # noqa: BLE001 - a failed read is retried, nothing is skipped
                print("relay: read failed (%s), retried" % type(e).__name__, file=err, flush=True)
                if stop.wait(pause):
                    break
                continue
            start = int(r.get("from_offset", off))
            if start > off:
                lost += start - off
                print("relay: LOST %d bytes (the server's buffer dropped them)" % (start - off),
                      file=err, flush=True)
            data = unescape(r.get("output", ""))
            if data:
                f.write(data)
                total += len(data)
            off = int(r.get("next_offset", off))
            if stop.is_set() and not r.get("more") and not data:
                break
            if not r.get("more") and not data and stop.wait(pause):
                continue
    print("relay: %d bytes, lost %d" % (total, lost), file=err, flush=True)
    return total, lost


def self_test():
    import http.server
    import os
    import tempfile

    stream = (b"\r\nglmark2-fb: 0 1234 QUJD\r\n" + bytes([0, 27]) + b"x\\y" + bytes(range(32, 127))) * 40
    state = {"oldest": 0, "writes": 0}

    def shown(b):
        return "".join(chr(c) if c in (10, 13) or 32 <= c < 127 else "\\x%02x" % c for c in b)

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_POST(self):
            q = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            name, a = q["params"]["name"], q["params"]["arguments"]
            if name == "serial_status":
                sc = {"current_offset": len(stream), "oldest_offset": state["oldest"]}
            elif name == "serial_read":
                s = max(int(a["since"]), state["oldest"])
                chunk = stream[s:s + 700]
                sc = {"from_offset": s, "next_offset": s + len(chunk), "output": shown(chunk),
                      "more": s + len(chunk) < len(stream)}
            else:
                state["writes"] += 1
                sc = {}
            body = json.dumps({"jsonrpc": "2.0", "id": q["id"],
                               "result": {"structuredContent": sc}}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = "http://127.0.0.1:%d/mcp" % srv.server_address[1]
    bad = []
    with tempfile.TemporaryDirectory() as d:
        def run(name, from_now):
            stop = threading.Event()
            log = open(os.path.join(d, name + ".err"), "w+")
            res = []
            t = threading.Thread(target=lambda: res.append(
                relay(url, os.path.join(d, name), from_now, stop, log, 0.05)))
            t.start()
            time.sleep(1.0)
            stop.set()
            t.join(10)
            log.seek(0)
            with open(os.path.join(d, name), "rb") as f:
                return res[0] if res else None, f.read(), log.read()

        res, got, log = run("whole", False)
        if got != stream or res != (len(stream), 0):
            bad.append("the whole stream is not relayed byte for byte")
        if "127.0.0.1" in log or "http" in log:
            bad.append("the URL is printed")
        res, got, log = run("now", True)
        if got != b"" or res != (0, 0):
            bad.append("--from-now relays old bytes")
        state["oldest"] = 1000
        res, got, log = run("lost", False)
        state["oldest"] = 0
        if got != stream[1000:] or res != (len(stream) - 1000, 0):
            bad.append("a start at the oldest held byte")
        # the buffer drops bytes between two reads: reported, never papered over
        stop = threading.Event()
        orig = call

        def dropping(u, name, args, rid=1, timeout=30):
            if name == "serial_read" and int(args["since"]) == 700:
                state["oldest"] = 2100
            return orig(u, name, args, rid, timeout)

        globals()["call"] = dropping
        res, got, log = run("hole", False)
        globals()["call"] = orig
        state["oldest"] = 0
        if res != (len(stream) - 1400, 1400) or "LOST 1400 bytes" not in log:
            bad.append("dropped bytes are not reported: %r" % (res,))
        if state["writes"]:
            bad.append("the relay wrote to the console")
        if relay("http://127.0.0.1:1/mcp", os.path.join(d, "none"), False, threading.Event(),
                 open(os.devnull, "w")) is not None:
            bad.append("an unreachable server is not refused")
    srv.shutdown()
    if unescape("a\\x00b\\x1b\r\n\\xff") != b"a\x00b\x1b\r\n\xff":
        bad.append("unescape")
    for b in bad:
        print("self-test: " + b, file=sys.stderr)
    print("console-relay self-test: %s" % ("FAILED" if bad else "ok"))
    return 1 if bad else 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    from_now = "--from-now" in argv
    args = [a for a in argv if a != "--from-now"]
    if len(args) != 2 or args[0].startswith("-"):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 2
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *a: stop.set())
    signal.signal(signal.SIGINT, lambda *a: stop.set())
    res = relay(args[0], args[1], from_now, stop)
    if res is None:
        return 2
    return 1 if res[1] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
