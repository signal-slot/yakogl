#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""RFC 2217 (Telnet COM-PORT-OPTION) console client, standard library only.

    uart_rfc2217.py probe   ADDR [--baud N]
    uart_rfc2217.py capture ADDR RAW TS [--baud N]
    uart_rfc2217.py pty     ADDR [--baud N]

ADDR is rfc2217://HOST:PORT or HOST:PORT. The probe and capture subcommands
are those of am62p_netboot_uart.py with the same files, exit codes and
SIGTERM behaviour; the difference is that the link is negotiated first
(BINARY, SGA, COM-PORT-OPTION), the baud rate is set (default 115200, 8N1)
and the data is cleaned of Telnet: IAC sequences are dropped, IAC IAC is one
0xFF byte.

  probe    Exit 0: held open (quiet or with data). 3: cannot connect. 4: the
           server answered "busy". 5: accepted and closed at once, or lost.
  capture  RAW gets the cleaned bytes, TS gets "<unix time> <line>" per line
           (CR and NUL removed). Exit 4: busy or unreachable. Otherwise runs
           until SIGTERM (exit 0, the unfinished last line is still written)
           or until the server closes.
  pty      Opens a pseudo-terminal, prints the slave path on stdout and relays
           both ways (0xFF is escaped as IAC IAC towards the board), until
           SIGTERM or until the server closes. For tools that want a tty.

Exit 42 (all subcommands): the server did not acknowledge the baud rate; the
console would be read at the wrong speed. Exit 2: bad arguments.
The host and port are never printed: messages say <BOARD_UART>.
"""
import argparse
import os
import select
import signal
import socket
import struct
import sys
import time
import tty

BAUD_UNCONFIRMED = 42
IAC, DONT, DO, WONT, WILL, SB, SE = 255, 254, 253, 252, 251, 250, 240
BINARY, SGA, COM_PORT = 0, 3, 44
SET_BAUDRATE, SET_DATASIZE, SET_PARITY, SET_STOPSIZE = 1, 2, 3, 4
SERVER = 100  # a server's answer to COM-PORT command N is N + 100
AGREED = (BINARY, SGA, COM_PORT)
HANDSHAKE_S = 5.0
WHO = "<BOARD_UART>"


def split_addr(addr):
    if addr.startswith("rfc2217://"):
        addr = addr[len("rfc2217://"):]
    host, sep, port = addr.rstrip("/").rpartition(":")
    if not sep or not host or not port.isdigit():
        raise ValueError("bad address")
    return host, int(port)


def escape(data):
    return data.replace(b"\xff", b"\xff\xff")


class Link:
    """One negotiated RFC 2217 connection. feed() turns wire bytes into data."""

    def __init__(self, sock, baud):
        self.sock, self.baud = sock, baud
        self.state, self.cmd, self.sb = "data", 0, bytearray()
        self.sent = set()      # (WILL|DO, option) we have said
        self.acked = None      # the baud rate the server answered, once
        self.baud_ok = False

    def send(self, b):
        self.sock.sendall(b)

    def subneg(self, code, value):
        self.send(bytes((IAC, SB, COM_PORT, code)) + escape(value) + bytes((IAC, SE)))

    def start(self):
        out = bytearray()
        for opt in AGREED:
            for cmd in (WILL, DO):
                self.sent.add((cmd, opt))
                out += bytes((IAC, cmd, opt))
        self.send(bytes(out))
        self.subneg(SET_BAUDRATE, struct.pack("!I", self.baud))
        self.subneg(SET_DATASIZE, b"\x08")
        self.subneg(SET_PARITY, b"\x01")
        self.subneg(SET_STOPSIZE, b"\x01")

    def option(self, cmd, opt):
        if cmd in (DO, WILL):
            mine = WILL if cmd == DO else DO
            if opt in AGREED:
                if (mine, opt) not in self.sent:
                    self.sent.add((mine, opt))
                    self.send(bytes((IAC, mine, opt)))
            else:
                self.send(bytes((IAC, WONT if cmd == DO else DONT, opt)))

    def subnegotiation(self, body):
        if len(body) >= 6 and body[0] == COM_PORT and body[1] == SET_BAUDRATE + SERVER:
            self.acked = struct.unpack("!I", bytes(body[2:6]))[0]
            self.baud_ok = self.acked == self.baud

    def feed(self, chunk):
        out = bytearray()
        for c in chunk:
            st = self.state
            if st == "data":
                if c == IAC:
                    self.state = "iac"
                else:
                    out.append(c)
            elif st == "iac":
                if c == IAC:
                    out.append(c)
                    self.state = "data"
                elif c in (WILL, WONT, DO, DONT):
                    self.cmd, self.state = c, "opt"
                elif c == SB:
                    self.sb, self.state = bytearray(), "sb"
                else:
                    self.state = "data"   # NOP, GA, ...: no argument
            elif st == "opt":
                self.option(self.cmd, c)
                self.state = "data"
            elif st == "sb":
                if c == IAC:
                    self.state = "sbiac"
                else:
                    self.sb.append(c)
            else:  # sbiac
                if c == SE:
                    self.subnegotiation(self.sb)
                    self.state = "data"
                elif c == IAC:
                    self.sb.append(IAC)
                    self.state = "sb"
                else:
                    self.state = "data"
        return bytes(out)


def open_link(cmd, args, timeout, unreachable):
    """Connect and negotiate. Returns (link, data seen during the handshake),
    or (None, exit code) with the reason already printed."""
    try:
        host, port = split_addr(args.addr)
    except ValueError:
        print("%s: bad address (want rfc2217://HOST:PORT or HOST:PORT)" % cmd, file=sys.stderr)
        return None, 2
    try:
        s = socket.create_connection((host, port), timeout=timeout)
    except (OSError, ValueError) as e:
        print("%s: cannot reach %s: %s" % (cmd, WHO, getattr(e, "strerror", None) or type(e).__name__),
              file=sys.stderr)
        return None, unreachable
    link, early, closed = Link(s, args.baud), bytearray(), False
    try:
        link.start()
        deadline = time.time() + HANDSHAKE_S
        while link.acked is None and not closed and time.time() < deadline:
            s.settimeout(max(0.05, min(0.5, deadline - time.time())))
            try:
                chunk = s.recv(4096)
            except socket.timeout:
                continue
            if not chunk:
                closed = True
                break
            early += link.feed(chunk)
    except OSError as e:
        s.close()
        print("%s: connection failed: %s" % (cmd, e.strerror or type(e).__name__), file=sys.stderr)
        return None, 5
    if b"busy" in early:
        s.close()
        print("%s: busy (another client holds the adapter %s)" % (cmd, WHO), file=sys.stderr)
        return None, 4
    if link.baud_ok:
        return link, bytes(early)
    s.close()
    if closed and link.acked is None:
        print("%s: %s accepted and closed at once" % (cmd, WHO), file=sys.stderr)
        return None, 5
    what = "did not answer the baud rate" if link.acked is None else "answered %d" % link.acked
    print("%s: %s: the server %s (asked for %d); not reading the console at a wrong speed"
          % (cmd, WHO, what, args.baud), file=sys.stderr)
    return None, BAUD_UNCONFIRMED


def clean(b):
    return b.replace(b"\r", b"").replace(b"\0", b"").decode("utf-8", "replace")


def probe(args):
    link, data = open_link("probe", args, 5, 3)
    if link is None:
        return data
    s = link.sock
    end = time.time() + 1.5
    try:
        while not data and time.time() < end:
            s.settimeout(max(0.05, end - time.time()))
            chunk = s.recv(256)
            if not chunk:
                print("probe: accepted and closed at once", file=sys.stderr)
                return 5
            data += link.feed(chunk)
    except socket.timeout:
        pass  # held open and quiet: a free console with nothing to say
    except OSError as e:
        print("probe: connection failed: %s" % (e.strerror or type(e).__name__), file=sys.stderr)
        return 5
    finally:
        s.close()
    if b"busy" in data:
        print("probe: busy (another client holds the adapter)", file=sys.stderr)
        return 4
    return 0


def on_term(signum, frame):
    raise SystemExit(0)


def capture(args):
    link, chunk = open_link("capture", args, 10, 4)
    if link is None:
        return chunk
    s = link.sock
    s.settimeout(1.0)
    signal.signal(signal.SIGTERM, on_term)
    pending = b""
    with open(args.raw, "ab", buffering=0) as raw, open(args.ts, "a", buffering=1) as st:
        try:
            while True:
                if chunk:
                    raw.write(chunk)
                    pending += chunk
                    now = time.time()
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n", 1)
                        st.write("%.3f %s\n" % (now, clean(line)))
                try:
                    wire = s.recv(4096)
                except socket.timeout:
                    chunk = b""
                    continue
                except OSError:
                    return 0
                if not wire:
                    return 0
                chunk = link.feed(wire)
        finally:
            if pending:
                st.write("%.3f %s\n" % (time.time(), clean(pending)))


def pty_relay(args):
    link, early = open_link("pty", args, 10, 3)
    if link is None:
        return early
    s = link.sock
    master, slave = os.openpty()   # the slave stays open here: no EIO when the user closes theirs
    tty.setraw(slave)
    print(os.ttyname(slave), flush=True)
    signal.signal(signal.SIGTERM, on_term)
    data = early
    try:
        while True:
            while data:
                n = os.write(master, data)
                data = data[n:]
            r, _, _ = select.select([s, master], [], [], 1.0)
            if s in r:
                try:
                    wire = s.recv(4096)
                except OSError:
                    return 0
                if not wire:
                    return 0
                data = link.feed(wire)
            if master in r:
                try:
                    out = os.read(master, 4096)
                except OSError:
                    return 0
                if out:
                    s.sendall(escape(out))
    finally:
        os.close(master)
        os.close(slave)
        s.close()


def main(argv=None):
    ap = argparse.ArgumentParser(description="RFC 2217 console client (standard library only)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def add(name, help_, *pos):
        p = sub.add_parser(name, help=help_)
        p.add_argument("addr", metavar="ADDR", help="rfc2217://HOST:PORT or HOST:PORT")
        for x in pos:
            p.add_argument(x)
        p.add_argument("--baud", type=int, default=115200, help="baud rate to set, 8N1 (default 115200)")

    add("probe", "open the console once and close it (exit 0 free, 3 unreachable, 4 busy, 5 closed)")
    add("capture", "write cleaned bytes to RAW and stamped lines to TS", "raw", "ts")
    add("pty", "relay the console to a pseudo-terminal, print its slave path")
    args = ap.parse_args(argv)
    return {"probe": probe, "capture": capture, "pty": pty_relay}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
