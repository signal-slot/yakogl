#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Append a leased serial console's output to a file, read through the lease's serial MCP server.

    SERIAL_MCP_URL=<the serial MCP server's URL> tools/serial_mcp_pump.py FILE
    tools/serial_mcp_pump.py --self-test

For a machine that has no device node of the console: the farm hands the
console out as MCP tools only (serial_read, serial_status, ...). An agent can
relay a boot log by hand, but not a 1.9 MB glmark2 frame dump; this does it.
FILE is the source of `tools/am62p_netboot_uart.py feed FILE PORT` ("Feed
bridge" in that file), whose port is then BOARD_UART for tools/board.sh:

    SERIAL_MCP_URL=... tools/serial_mcp_pump.py $TMPDIR/console.feed &
    tools/am62p_netboot_uart.py feed $TMPDIR/console.feed <port> &
    BOARD_UART=127.0.0.1:<port> tools/board.sh x2 run ...

It only reads: serial_status once (the offset to start from, so nothing older
than the start is relayed) and then serial_read by offset, in a loop, until
SIGTERM or SIGINT. Nothing is ever written to the console. The URL comes from
the environment (SERIAL_MCP_URL), never from a file, and is never printed.

The server shows control bytes as \\xNN in its text (a literal backslash is
not escaped); TAB, CR and LF come as they are. The bytes are restored here:
\\xNN becomes the byte when NN is a control byte other than TAB, CR and LF, or
0x7f and above; \\x41 (a printable byte, which the server never escapes) is
text and stays. Each read's `bytes` count is compared with what was restored.

Lines on stderr: "pump: start at offset N"; "pump: length differs at offset N:
A restored, B reported" (text that looked like an escape, or bytes that were
not UTF-8: seen only in the garbage of a SoC reset); "pump: TRUNCATED at
offset N" (the server's buffer dropped output before it was read: the capture
has a hole, rerun the board); "pump: read failed (...), retry"; and at the end
"pump: N bytes, M reads with a length difference, K truncated". Exit 0 on a
signal, 2 on a usage error or no URL, 3 when the server cannot be reached at
the start. Stdlib only.
"""
import json
import os
import re
import signal
import sys
import threading
import time
import urllib.request

ESC = re.compile(rb"\\x([0-9a-f]{2})")
TOOL_READ = "serial_read"
TOOL_STATUS = "serial_status"


def unescape(text):
    """The server's output text -> the console's bytes (module docstring)."""
    raw = text.encode("utf-8", "surrogateescape")

    def one(m):
        v = int(m.group(1), 16)
        if (v < 0x20 and v not in (9, 10, 13)) or v >= 0x7F:
            return bytes([v])
        return m.group(0)

    return ESC.sub(one, raw)


class Client:
    """A minimal MCP client over streamable HTTP: initialize, tools/call."""

    def __init__(self, url, timeout=60):
        self.url, self.timeout, self.sid, self.n = url, timeout, None, 0
        self.rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                "clientInfo": {"name": "serial_mcp_pump", "version": "1"}})
        self.rpc("notifications/initialized", None, notify=True)

    def rpc(self, method, params, notify=False):
        self.n += 1
        body = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            body["params"] = params
        if not notify:
            body["id"] = self.n
        head = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        if self.sid:
            head["Mcp-Session-Id"] = self.sid
        req = urllib.request.Request(self.url, json.dumps(body).encode(), head)
        with urllib.request.urlopen(req, timeout=self.timeout) as f:
            self.sid = f.headers.get("Mcp-Session-Id") or self.sid
            ctype = f.headers.get("Content-Type", "")
            text = f.read().decode("utf-8", "surrogateescape")
        if notify or not text.strip():
            return None
        if "event-stream" in ctype:
            for line in text.splitlines():
                if line.startswith("data:"):
                    d = json.loads(line[5:])
                    if d.get("id") == self.n:
                        return d
            return None
        return json.loads(text)

    def call(self, name, args):
        r = self.rpc("tools/call", {"name": name, "arguments": args})
        if not r or "result" not in r:
            raise OSError("no result")
        res = r["result"]
        if "structuredContent" in res:
            return res["structuredContent"]
        return json.loads(res["content"][0]["text"])


def pump(url, path, stop, log=sys.stderr, poll_ms=500):
    """Relay until `stop` (a threading.Event) is set. -> exit status."""

    def say(msg):
        print("pump: " + msg, file=log, flush=True)

    try:
        c = Client(url)
        off = c.call(TOOL_STATUS, {})["current_offset"]
    except (OSError, ValueError, KeyError) as e:
        say("the serial MCP server cannot be reached (%s)" % type(e).__name__)
        return 3
    say("start at offset %d" % off)
    total = bad = trunc = 0
    with open(path, "ab", buffering=0) as f:
        while not stop.is_set():
            try:
                r = c.call(TOOL_READ, {"since": off, "timeout_ms": poll_ms, "max_bytes": 262144})
            except (OSError, ValueError, KeyError) as e:
                say("read failed (%s), retry" % type(e).__name__)
                stop.wait(1)
                try:
                    c = Client(url)
                except (OSError, ValueError):
                    pass
                continue
            n = r.get("bytes", 0)
            if r.get("truncated"):
                trunc += 1
                say("TRUNCATED at offset %d (output dropped from the server's buffer before it was read)" % off)
            if n:
                b = unescape(r.get("output", ""))
                if len(b) != n:
                    bad += 1
                    say("length differs at offset %d: %d restored, %d reported" % (r.get("from_offset", off), len(b), n))
                f.write(b)
                total += len(b)
            nxt = r.get("next_offset", off)
            if nxt != off:
                off = nxt
            elif not n:
                stop.wait(0.2)
    say("%d bytes, %d reads with a length difference, %d truncated" % (total, bad, trunc))
    return 0


def self_test():
    import http.server
    import io
    import tempfile

    fails = []

    def check(cond, what):
        if not cond:
            fails.append(what)
            print("FAIL: " + what, file=sys.stderr)

    check(unescape("a\\x1b[0m") == b"a\x1b[0m", "unescape: ESC restored")
    check(unescape("\\x41 \\x7e") == b"\\x41 \\x7e", "unescape: a printable \\xNN is text")
    check(unescape("\\x00\\x7f\\xff") == b"\x00\x7f\xff", "unescape: NUL, DEL and 0xff restored")
    check(unescape("\\x09\\x0a\\x0d") == b"\\x09\\x0a\\x0d", "unescape: TAB, LF and CR are never escaped by the server, so stay text")
    check(unescape("t\tx\r\n") == b"t\tx\r\n", "unescape: TAB, CR, LF pass")
    check(unescape("A\\B") == b"A\\B", "unescape: a lone backslash stays")

    # a fake serial MCP server: 100 bytes of old output, then three chunks
    old = 100
    chunks = [
        dict(output="glmark2-fb: 0 aa8f QUJD\r\n", bytes=25),
        dict(output="\\x1b[0mline\r\n", bytes=10),
        dict(output="\\x1b text\r\n", bytes=12, truncated=True),  # a reported count the restored bytes do not have
    ]
    seen = {"calls": [], "writes": 0}

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            m = body.get("method")
            res = {}
            if m == "tools/call":
                name, args = body["params"]["name"], body["params"]["arguments"]
                seen["calls"].append((name, args.get("since")))
                if name == TOOL_STATUS:
                    res = {"structuredContent": {"current_offset": old}}
                elif name == TOOL_READ:
                    off, pos, out = args["since"], old, None
                    for ch in chunks:
                        if off == pos:
                            out = dict(ch, from_offset=pos, next_offset=pos + ch["bytes"])
                        pos += ch["bytes"]
                    if out is None:
                        out = {"bytes": 0, "output": "", "from_offset": off, "next_offset": off}
                    # the text form only, as a server without structuredContent answers
                    res = {"content": [{"type": "text", "text": json.dumps(out)}]}
                else:
                    seen["writes"] += 1
            if "id" not in body:
                self.send_response(202)
                self.end_headers()
                return
            data = json.dumps({"jsonrpc": "2.0", "id": body["id"], "result": res}).encode()
            if m == "initialize":
                ctype, data = "text/event-stream", b"event: message\ndata: " + data + b"\n\n"
            else:
                ctype = "application/json"
            self.send_response(200)
            self.send_header("Content-Type", ctype)
            self.send_header("Mcp-Session-Id", "s1")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = "http://127.0.0.1:%d/mcp" % srv.server_address[1]
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "feed")
        with open(path, "wb") as f:
            f.write(b"kept\n")
        stop, log, rc = threading.Event(), io.StringIO(), []
        t = threading.Thread(target=lambda: rc.append(pump(url, path, stop, log, poll_ms=10)))
        t.start()
        end = time.time() + 10
        want = b"kept\nglmark2-fb: 0 aa8f QUJD\r\n\x1b[0mline\r\n\x1b text\r\n"
        while time.time() < end:
            with open(path, "rb") as f:
                if f.read() == want:
                    break
            time.sleep(0.05)
        stop.set()
        t.join(10)
        with open(path, "rb") as f:
            got = f.read()
        out = log.getvalue()
        check(got == want, "pump: the file is the old content plus the three chunks restored (got %r)" % got)
        check(rc == [0], "pump: exit 0 on stop (got %s)" % rc)
        check("pump: start at offset 100" in out, "pump: starts at the server's current offset")
        check("length differs at offset 135: 8 restored, 12 reported" in out, "pump: a length difference is logged (log %r)" % out)
        check("TRUNCATED at offset 135" in out, "pump: a truncated read is logged")
        check("1 reads with a length difference, 1 truncated" in out, "pump: the end line counts them")
        check(seen["calls"][0] == (TOOL_STATUS, None) and seen["calls"][1] == (TOOL_READ, 100),
              "pump: serial_status once, then serial_read from its offset")
        check(all(n in (TOOL_STATUS, TOOL_READ) for n, _ in seen["calls"]) and seen["writes"] == 0,
              "pump: only serial_status and serial_read are called")
        check(url not in out, "pump: the URL is never printed")
    srv.shutdown()
    dead = io.StringIO()
    check(pump("http://127.0.0.1:%d/mcp" % srv.server_address[1], os.devnull, threading.Event(), dead) == 3,
          "pump: exit 3 when the server cannot be reached")
    check("127.0.0.1" not in dead.getvalue(), "pump: the unreachable message does not name the URL")
    print("serial_mcp_pump self-test: %s" % ("ok" if not fails else "%d failed" % len(fails)))
    return 1 if fails else 0


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) != 2 or argv[1].startswith("-"):
        print(__doc__, file=sys.stderr)
        return 2
    url = os.environ.get("SERIAL_MCP_URL")
    if not url:
        print("pump: set SERIAL_MCP_URL to the lease's serial MCP server", file=sys.stderr)
        return 2
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda s, f: stop.set())
    signal.signal(signal.SIGINT, lambda s, f: stop.set())
    return pump(url, argv[1], stop)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
