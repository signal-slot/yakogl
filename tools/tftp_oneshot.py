#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Read-only TFTP server that serves each file ONCE (the AM62P netboot).

    tools/tftp_oneshot.py --root DIR [--bind ADDR] [--port 69] [--log FILE]
    tools/tftp_oneshot.py --self-test

Why "once": U-Boot on the Verdin AM62P fetches the next Zephyr image from this
server (driver/docs/am62p-netboot-plan.md). A file that could be fetched twice
could boot Zephyr on a reboot nobody asked for (another agent reboots the
shared board into Linux, and U-Boot finds our image still offered). So after a
COMPLETE transfer (the client acknowledged the last block) the file is renamed
to "<name>.served" and the next request for it gets "File not found", which
makes U-Boot fall back to Linux at once.

A file is also refused when it was staged (its mtime) more than --max-age
seconds ago (default 300). The runner stages an image seconds before it
reboots the board; if the runner is killed hard in between, its image must
not wait in the root for the next reboot somebody else makes.

Protocol: RFC 1350 read requests in octet mode, with the options U-Boot sends:
blksize (RFC 2348), tsize and timeout (RFC 2349) and windowsize (RFC 7440).
Write requests are refused. Paths are relative to --root; ".." and absolute
paths are refused. Plain sockets only, no third-party package.

Every event is one line on stdout (and --log):
    "<unix time> tftp: <event> key=value ..."
"done" carries bytes, seconds, the rate and the sha256 prefix; "failed" and
"refused" name the cause. Port 69 needs a privilege this host does not give
uid 1000 (net.ipv4.ip_unprivileged_port_start=1024); that is the operator's
decision, see the plan. The self-test uses a high loopback port.
"""
import argparse
import hashlib
import os
import socket
import struct
import sys
import tempfile
import threading
import time

OP_RRQ, OP_WRQ, OP_DATA, OP_ACK, OP_ERROR, OP_OACK = 1, 2, 3, 4, 5, 6
ERR_NOT_DEFINED, ERR_NOT_FOUND, ERR_ACCESS, ERR_ILLEGAL_OP, ERR_UNKNOWN_TID = 0, 1, 2, 4, 5
BLKSIZE_MIN, BLKSIZE_MAX = 8, 65464
WINDOW_MAX = 64
RETRIES = 5
SERVED_SUFFIX = ".served"


class Logger:
    def __init__(self, path=None):
        self.lock = threading.Lock()
        self.fh = open(path, "a", buffering=1) if path else None

    def __call__(self, event, **kv):
        body = " ".join("%s=%s" % (k, v) for k, v in kv.items())
        line = "%.3f tftp: %s %s" % (time.time(), event, body)
        with self.lock:
            print(line.rstrip(), flush=True)
            if self.fh:
                self.fh.write(line.rstrip() + "\n")


def error_packet(code, msg):
    return struct.pack("!HH", OP_ERROR, code) + msg.encode() + b"\0"


def parse_request(pkt):
    """Returns (opcode, filename, mode, {option: value}) or raises ValueError."""
    if len(pkt) < 4:
        raise ValueError("short request")
    op = struct.unpack("!H", pkt[:2])[0]
    fields = pkt[2:].split(b"\0")
    if len(fields) < 3 or fields[-1] != b"":
        raise ValueError("request not NUL-terminated")
    fields = [f.decode("ascii", "replace") for f in fields[:-1]]
    name, mode, rest = fields[0], fields[1].lower(), fields[2:]
    if len(rest) % 2:
        raise ValueError("odd option list")
    opts = {rest[i].lower(): rest[i + 1] for i in range(0, len(rest), 2)}
    return op, name, mode, opts


def resolve(root, name):
    """The file under root that name designates, or None when it could escape root."""
    if name.startswith("/") or "\\" in name:
        return None
    parts = [p for p in name.split("/") if p not in ("", ".")]
    if not parts or ".." in parts:
        return None
    real_root = os.path.realpath(root)
    path = os.path.realpath(os.path.join(real_root, *parts))
    if os.path.commonpath([path, real_root]) != real_root:
        return None  # a symlink out of the root
    return path


def _int(opts, key):
    try:
        return int(opts[key])
    except (KeyError, ValueError):
        return None


def negotiate(opts, size):
    """The options this server accepts, as its OACK carries them."""
    acc = {}
    b = _int(opts, "blksize")
    if b is not None and b >= BLKSIZE_MIN:
        acc["blksize"] = min(b, BLKSIZE_MAX)
    w = _int(opts, "windowsize")
    if w is not None and w >= 1:
        acc["windowsize"] = min(w, WINDOW_MAX)
    t = _int(opts, "timeout")
    if t is not None and 1 <= t <= 255:
        acc["timeout"] = t
    if "tsize" in opts:
        acc["tsize"] = size
    return acc


class Transfer:
    """One read transfer, on its own socket (its own TID, RFC 1350 section 4)."""

    def __init__(self, bind, peer, path, name, opts, oneshot, log):
        self.peer, self.path, self.name, self.opts = peer, path, name, opts
        self.oneshot, self.log = oneshot, log
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind((bind, 0))

    def who(self):
        return "%s:%d" % self.peer

    def wait_ack(self, timeout):
        """The next ACK block number from the peer, or None on timeout.
        Raises RuntimeError when the peer sends an ERROR packet."""
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return None
            self.sock.settimeout(left)
            try:
                pkt, addr = self.sock.recvfrom(65536)
            except socket.timeout:
                return None
            if addr != self.peer:
                # RFC 1350: a packet from an unknown TID gets an error; the
                # transfer goes on.
                self.sock.sendto(error_packet(ERR_UNKNOWN_TID, "Unknown transfer ID"), addr)
                continue
            if len(pkt) < 4:
                continue
            op, arg = struct.unpack("!HH", pkt[:4])
            if op == OP_ACK:
                return arg
            if op == OP_ERROR:
                raise RuntimeError("peer_error_%d_%s" % (arg, pkt[4:].rstrip(b"\0").decode("ascii", "replace")))

    def run(self):
        t0 = time.monotonic()
        try:
            with open(self.path, "rb") as fh:
                data = fh.read()
        except OSError as e:
            self.sock.sendto(error_packet(ERR_NOT_FOUND, "File not found"), self.peer)
            self.log("failed", peer=self.who(), file=self.name, cause="open_" + str(e.errno))
            self.sock.close()
            return False
        acc = negotiate(self.opts, len(data))
        blksize = acc.get("blksize", 512)
        window = acc.get("windowsize", 1)
        timeout = acc.get("timeout", 1)
        nblocks = len(data) // blksize + 1  # the last block is short, maybe empty
        self.log("start", peer=self.who(), file=self.name, bytes=len(data),
                 blksize=blksize, windowsize=window, blocks=nblocks)
        resent = 0
        try:
            if acc:
                oack = struct.pack("!H", OP_OACK) + b"".join(
                    k.encode() + b"\0" + str(v).encode() + b"\0" for k, v in acc.items())
                for _ in range(RETRIES + 1):
                    self.sock.sendto(oack, self.peer)
                    if self.wait_ack(timeout) == 0:
                        break
                else:
                    self.log("failed", peer=self.who(), file=self.name, cause="no_ack_of_oack")
                    return False
            # base: the first block (1-based, not wrapped) not yet acknowledged.
            # max_sent: the highest block sent so far. An ACK can name any block
            # in [base-1, max_sent]: when this side lags behind a queue of
            # duplicate ACKs, the peer's final ACK arrives while base is still
            # low, and it must not be discarded as stale (it was, until the
            # self-test's lost-packet case caught it: the peer had everything
            # and left, and the server timed out resending the last window).
            base, retries, max_sent, send, resumed_at = 1, 0, 0, True, None
            while base <= nblocks:
                if send:
                    last = min(base + window - 1, nblocks)
                    for n in range(base, last + 1):
                        chunk = data[(n - 1) * blksize:n * blksize]
                        self.sock.sendto(struct.pack("!HH", OP_DATA, n & 0xFFFF) + chunk, self.peer)
                    max_sent = max(max_sent, last)
                    send = False
                a = self.wait_ack(timeout)
                if a is None:
                    retries += 1
                    resent += 1
                    if retries > RETRIES:
                        self.log("failed", peer=self.who(), file=self.name, cause="timeout",
                                 acked_blocks=base - 1, of=nblocks)
                        return False
                    send = True
                    continue
                k = next((c for c in range(max_sent, base - 2, -1) if c & 0xFFFF == a), None)
                if k is None:
                    continue  # names no block that is still open: ignore, keep waiting
                if k == base - 1:
                    # RFC 7440: the peer lost a block and asks to resume at base.
                    # Honour each such request once per base, or every duplicate
                    # ACK would resend the window again (Sorcerer's Apprentice).
                    if resumed_at != base:
                        resumed_at = base
                        resent += 1
                        send = True
                    continue
                retries = 0
                base = k + 1
                send = True
        except RuntimeError as e:
            self.log("failed", peer=self.who(), file=self.name, cause=str(e).replace(" ", "_"))
            return False
        finally:
            self.sock.close()
        dt = time.monotonic() - t0
        renamed = "-"
        if self.oneshot:
            os.replace(self.path, self.path + SERVED_SUFFIX)
            renamed = os.path.basename(self.path + SERVED_SUFFIX)
        self.log("done", peer=self.who(), file=self.name, bytes=len(data),
                 sha256=hashlib.sha256(data).hexdigest()[:16], seconds="%.3f" % dt,
                 mib_per_s="%.1f" % (len(data) / 1048576 / dt if dt > 0 else 0.0),
                 windows_resent=resent, renamed_to=renamed)
        return True


class Server:
    def __init__(self, root, bind="0.0.0.0", port=69, oneshot=True, log=None, max_age=300):
        self.root, self.bind, self.oneshot, self.max_age = root, bind, oneshot, max_age
        self.log = log or Logger()
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind((bind, port))
        self.port = self.sock.getsockname()[1]
        self.stopping = False
        # One transfer per file at a time: a retransmitted RRQ must not start
        # a second transfer of the same one-shot file.
        self.busy = set()
        self.busy_lock = threading.Lock()

    def refuse(self, peer, code, msg, **kv):
        self.sock.sendto(error_packet(code, msg), peer)
        self.log("refused", peer="%s:%d" % peer, **kv)

    def handle(self, pkt, peer):
        try:
            op, name, mode, opts = parse_request(pkt)
        except ValueError as e:
            self.refuse(peer, ERR_ILLEGAL_OP, "Bad request", cause="malformed_" + str(e).replace(" ", "_"))
            return
        if op == OP_WRQ:
            self.refuse(peer, ERR_ACCESS, "Read-only server", file=name, cause="write_request")
            return
        if op != OP_RRQ:
            self.refuse(peer, ERR_ILLEGAL_OP, "Illegal operation", cause="opcode_%d" % op)
            return
        if mode != "octet":
            self.refuse(peer, ERR_NOT_DEFINED, "Only octet mode", file=name, cause="mode_" + mode)
            return
        path = resolve(self.root, name)
        if path is None:
            self.refuse(peer, ERR_ACCESS, "Access violation", file=name, cause="outside_root")
            return
        if not os.path.isfile(path):
            self.refuse(peer, ERR_NOT_FOUND, "File not found", file=name, cause="not_found")
            return
        age = time.time() - os.path.getmtime(path)
        if self.max_age and age > self.max_age:
            self.refuse(peer, ERR_NOT_FOUND, "File not found", file=name,
                        cause="stale_%ds_over_max_age_%ds" % (age, self.max_age))
            return
        with self.busy_lock:
            if path in self.busy:
                self.log("ignored", peer="%s:%d" % peer, file=name, cause="duplicate_rrq")
                return
            self.busy.add(path)
        self.log("rrq", peer="%s:%d" % peer, file=name,
                 options=",".join("%s=%s" % kv for kv in sorted(opts.items())) or "-")

        def work():
            try:
                Transfer(self.bind, peer, path, name, opts, self.oneshot, self.log).run()
            finally:
                with self.busy_lock:
                    self.busy.discard(path)

        threading.Thread(target=work, daemon=True).start()

    def serve_forever(self):
        self.log("listening", bind=self.bind, port=self.port, root=self.root,
                 oneshot="yes" if self.oneshot else "no")
        self.sock.settimeout(0.5)
        while not self.stopping:
            try:
                pkt, peer = self.sock.recvfrom(65536)
            except socket.timeout:
                continue
            except OSError:
                break
            self.handle(pkt, peer)
        self.sock.close()

    def stop(self):
        self.stopping = True


# ---------------------------------------------------------------------------
# A minimal client, for the self-test only (U-Boot is the real client).

def fetch(host, port, name, blksize=None, windowsize=None, drop_data=0, timeout=2.0):
    """Returns (bytes, None) or (None, "<error>"). drop_data discards that many
    DATA packets once, to exercise the server's retransmission."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.settimeout(timeout)
    req = struct.pack("!H", OP_RRQ) + name.encode() + b"\0octet\0"
    if blksize:
        req += b"blksize\0%d\0" % blksize
    if windowsize:
        req += b"windowsize\0%d\0" % windowsize
    if blksize or windowsize:
        req += b"tsize\x000\0"
    s.sendto(req, (host, port))
    out, expect, bs, win, since_ack, tid = bytearray(), 1, 512, 1, 0, None
    try:
        while True:
            pkt, addr = s.recvfrom(70000)
            tid = tid or addr
            op = struct.unpack("!H", pkt[:2])[0]
            if op == OP_ERROR:
                return None, pkt[4:].rstrip(b"\0").decode()
            if op == OP_OACK:
                f = pkt[2:].split(b"\0")[:-1]
                o = {f[i].decode(): int(f[i + 1]) for i in range(0, len(f), 2)}
                bs, win = o.get("blksize", 512), o.get("windowsize", 1)
                s.sendto(struct.pack("!HH", OP_ACK, 0), tid)
                continue
            if op != OP_DATA:
                return None, "unexpected opcode %d" % op
            n = struct.unpack("!H", pkt[2:4])[0]
            if drop_data > 0:
                drop_data -= 1
                continue
            if n != expect & 0xFFFF:
                # out of order: acknowledge the last good block (RFC 7440)
                s.sendto(struct.pack("!HH", OP_ACK, (expect - 1) & 0xFFFF), tid)
                since_ack = 0
                continue
            out += pkt[4:]
            expect += 1
            since_ack += 1
            last = len(pkt) - 4 < bs
            if last or since_ack >= win:
                s.sendto(struct.pack("!HH", OP_ACK, (expect - 1) & 0xFFFF), tid)
                since_ack = 0
            if last:
                return bytes(out), None
    except socket.timeout:
        return None, "client timeout"
    finally:
        s.close()


def self_test():
    ok = True

    def check(cond, what):
        nonlocal ok
        print("  %s  %s" % ("PASS" if cond else "FAIL", what))
        ok = ok and cond

    with tempfile.TemporaryDirectory() as root:
        os.makedirs(os.path.join(root, "yakogl"))
        blobs = {
            # a gl_test image is 16 107 024 bytes: 16 MiB + 1 is not a multiple of any block size
            "yakogl/big.bin": os.urandom(16 * 1048576 + 1),
            # an exact multiple of 1468: the transfer ends with an empty block
            "yakogl/exact.bin": os.urandom(1468 * 40),
            "yakogl/small.bin": b"hello",
            "yakogl/retry.bin": os.urandom(1468 * 100 + 7),
            # 70001 blocks of 512: block numbers wrap past 65535
            "yakogl/wrap.bin": os.urandom(512 * 70000 + 3),
        }
        for n, b in blobs.items():
            with open(os.path.join(root, n), "wb") as fh:
                fh.write(b)
        events = []

        class ListLog(Logger):
            def __call__(self, event, **kv):
                events.append((event, kv))

        srv = Server(root, bind="127.0.0.1", port=0, oneshot=True, log=ListLog())
        th = threading.Thread(target=srv.serve_forever, daemon=True)
        th.start()
        port = srv.port

        def served(n):
            # the rename follows the final ACK on the server's thread
            for _ in range(50):
                if os.path.exists(os.path.join(root, n + SERVED_SUFFIX)):
                    return not os.path.exists(os.path.join(root, n))
                time.sleep(0.02)
            return False

        t = time.monotonic()
        got, err = fetch("127.0.0.1", port, "yakogl/big.bin", blksize=1468, windowsize=16)
        dt = time.monotonic() - t
        check(got == blobs["yakogl/big.bin"],
              "16 MiB+1, blksize 1468, windowsize 16: bytes equal (%.2f s, err=%s)" % (dt, err))
        check(served("yakogl/big.bin"), "one-shot: renamed to .served after the final ACK")
        got, err = fetch("127.0.0.1", port, "yakogl/big.bin", blksize=1468)
        check(got is None and err == "File not found",
              "one-shot: the second request gets 'File not found' (%s)" % err)
        got, err = fetch("127.0.0.1", port, "yakogl/exact.bin", blksize=1468, windowsize=4)
        check(got == blobs["yakogl/exact.bin"], "size a multiple of blksize: bytes equal (%s)" % err)
        got, err = fetch("127.0.0.1", port, "yakogl/small.bin")
        check(got == b"hello", "no options: plain RFC 1350 (%s)" % err)
        got, err = fetch("127.0.0.1", port, "yakogl/retry.bin", blksize=1468, windowsize=8, drop_data=3)
        check(got == blobs["yakogl/retry.bin"], "three DATA packets lost: the window is resent (%s)" % err)
        got, err = fetch("127.0.0.1", port, "yakogl/wrap.bin", timeout=5.0)
        check(got == blobs["yakogl/wrap.bin"], "block numbers wrap past 65535 (%s)" % err)
        for bad in ("../etc/passwd", "/etc/passwd", "yakogl/../../x", "a\\b"):
            got, err = fetch("127.0.0.1", port, bad)
            check(got is None and err == "Access violation", "refused outside the root: %r (%s)" % (bad, err))
        stale = os.path.join(root, "yakogl/stale.bin")
        with open(stale, "wb") as fh:
            fh.write(b"old")
        os.utime(stale, (time.time() - 301, time.time() - 301))
        got, err = fetch("127.0.0.1", port, "yakogl/stale.bin")
        check(got is None and err == "File not found" and os.path.exists(stale),
              "staged 301 s ago (max age 300): refused, not served (%s)" % err)
        got, err = fetch("127.0.0.1", port, "yakogl/none.bin")
        check(got is None and err == "File not found", "missing file: 'File not found' at once")
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.settimeout(2)
        s.sendto(struct.pack("!H", OP_WRQ) + b"yakogl/w.bin\0octet\0", ("127.0.0.1", port))
        pkt, _ = s.recvfrom(1000)
        s.close()
        check(struct.unpack("!HH", pkt[:4]) == (OP_ERROR, ERR_ACCESS), "write request refused")
        srv.stop()
        th.join(2)
        # "done" is logged just after the rename, on the transfer's thread
        for _ in range(100):
            if sum(1 for e, _ in events if e == "done") >= len(blobs):
                break
            time.sleep(0.02)
        dones = [kv for e, kv in events if e == "done"]
        # population: 5 files were offered; each completed exactly once
        check(sorted(kv["file"] for kv in dones) == sorted(blobs),
              "one 'done' per offered file, each once: %d of %d %s" % (len(dones), len(blobs), sorted(set(blobs) - {kv["file"] for kv in dones})))
        check(all("mib_per_s" in kv and "sha256" in kv for kv in dones), "'done' carries rate and sha256")
        check(not any(e == "failed" for e, _ in events), "no 'failed' event")
    print("tftp_oneshot self-test: %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", help="directory served (read-only)")
    ap.add_argument("--bind", default="0.0.0.0")
    ap.add_argument("--port", type=int, default=69)
    ap.add_argument("--log", help="append every event line to this file too")
    ap.add_argument("--keep", action="store_true", help="serve files more than once (no rename)")
    ap.add_argument("--max-age", type=int, default=300,
                    help="refuse a file staged longer ago than this many seconds (0: no limit)")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.root or not os.path.isdir(a.root):
        ap.error("--root DIR is required and must exist")
    try:
        srv = Server(os.path.abspath(a.root), a.bind, a.port, not a.keep, Logger(a.log), a.max_age)
    except PermissionError:
        print("tftp: failed cause=bind_permission port=%d (a port below "
              "net.ipv4.ip_unprivileged_port_start needs a privilege; "
              "see driver/docs/am62p-netboot-plan.md)" % a.port, file=sys.stderr)
        return 3
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
