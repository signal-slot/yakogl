#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The NanoPC-T6's debug UART over RFC 2217, given to a runner as a tty.

    T6_UART=rfc2217://<host>:<port> t6-uart-rfc2217.py run -- <command> [args...]
        Opens an RFC 2217 session to the serial adapter on the board's debug
        UART (UART2, 1500000 8N1), sets the line, makes a pseudo terminal
        and runs <command> with T6_UART set to the pty's slave path
        (/dev/pts/N) and T6_UART_RFC2217=1. It relays the bytes both ways
        until the command ends, and exits with the command's status. The
        three T6 runners (nanopc-t6.sh beside this, ../uboot-t6/prompt-reads.sh
        and prompt-go.sh) run themselves again through this when T6_UART is
        an rfc2217:// address and the run is --go.
    t6-uart-rfc2217.py --self-test
        The same code against a made-up adapter on 127.0.0.1 (a thread of
        this process) and commands that stand for the runners. No board.

Why it exists. The T6's debug UART is wired to a network serial adapter
that speaks RFC 2217 (telnet's COM-PORT-OPTION, option 44). The adapter
keeps 115200 as its stored speed; the T6's console runs at 1500000. A
session must set 1500000 itself, and the setting lasts for that session
only. The adapter takes one client at a time (a second one is told "busy").
The runners read and write T6_UART as a local tty (stty, fuser, cat,
TIOCEXCL), so this gives them one and leaves their checks as they are.

The address is read from the environment (T6_UART), never from the command
line, which `ps` shows. No message of this tool names it.

What it guarantees:

  - The line is set before the command starts. The session asks for
    1500000 baud, 8 data bits, no parity, 1 stop bit and no flow control,
    and the server must answer each with exactly that value in force.
    Anything else (no RFC 2217, an answer with another value, no answer in
    NEGOTIATE_S, a closed connection) is a refusal: exit NOT_STARTED, the
    command is not run. Bytes that arrive before the line is set are
    dropped, never relayed.
  - The pty's slave side is raw with echo off before any byte is relayed.
    With echo on, the slave would send every byte from the board back to
    the board, whose console is an autologin root shell (nanopc-t6.sh,
    T6_UART).
  - This tool does not keep the slave open: `fuser` on it names no holder,
    and the command can take it with TIOCEXCL.
  - While no process has the slave open, bytes from the board are dropped,
    not queued. The kernel would queue them (the self-test shows it) and
    hand an old boot's lines to the next reader. The slave's state is read
    from the master: poll() reports POLLHUP on the master while no process
    has the slave open (linux drivers/tty/pty.c, pty_close sets
    TTY_OTHER_CLOSED on the master; n_tty_poll reports it as POLLHUP).
    When the last opener closes the slave with bytes it did not read, they
    are flushed the moment the close is seen (a short open of the slave,
    TCIFLUSH, close), as a real UART's buffer goes with its last close. A
    slave left with TIOCEXCL cannot be opened for that by anyone but root,
    so it is skipped then: nobody else can read the bytes either.
  - IAC (0xff) is doubled towards the server and undoubled from it.
  - A lost connection is opened and set again; the bytes of the gap are
    lost both ways (what the command wrote during the gap is dropped, not
    sent late), and one line on stderr gives the time and the gap's length.
  - The command runs in a process group of its own. SIGTERM and SIGHUP to
    this tool are passed to the command's process (as a `kill` of the
    runner without this tool), SIGINT to its process group (as a ctrl-c on
    a terminal reaches a runner's group). The tool never kills the command
    itself: it keeps relaying until the command ends, so a runner that is
    between a jump's steps finishes them its own way (nanopc-t6.sh's EXIT
    trap stops its captures and gives the board's mode and clock back).
    When the command has ended, the processes left in its group get
    GROUP_WAIT_S to end (the runners' ssh commands are bounded by 30 s),
    then SIGTERM, then SIGKILL, each said on stderr. Then the socket and
    the pty are closed; nothing is left behind (no file is made).

The wire, as pyserial 3.5's RFC 2217 client (serial/rfc2217.py, Serial.open),
which is measured to work with the adapter, without its DO ECHO, DTR, RTS
and second purge (none is needed to set the line, and telnet's ECHO would
ask the server to send our bytes back):
  IAC WILL SGA, IAC DO SGA, IAC DO COM-PORT-OPTION, IAC WILL COM-PORT-OPTION;
  answers to the server's own requests (BINARY and SGA are agreed to, any
  other option refused); after the server's DO COM-PORT-OPTION:
  IAC SB 44 SET-BAUDRATE 00 16 e3 60 IAC SE, IAC SB 44 SET-DATASIZE 08 IAC SE,
  IAC SB 44 SET-PARITY 01 IAC SE, IAC SB 44 SET-STOPSIZE 01 IAC SE; after
  their four answers IAC SB 44 SET-CONTROL 01 IAC SE (no flow control);
  after its answer IAC SB 44 PURGE-DATA 01 IAC SE (the adapter's receive
  buffer: bytes from before this session are not relayed).

Exit: the command's status (128 + N when a signal N ended it).
NOT_STARTED (40): the command was not started, nothing was relayed (no
rfc2217://<host>:<port> in T6_UART, a session that could not be opened or
set, this tool already around the command, or a command that cannot be
run). FAILED (41): this tool failed while the command ran; the command was
sent SIGTERM and waited for. The runners use neither code.

Only the Python standard library: run it with `python3 -I`.
"""

import errno
import fcntl
import os
import re
import select
import signal
import socket
import struct
import subprocess
import sys
import termios
import threading
import time
import tty

NOT_STARTED = 40
FAILED = 41
# The environment the command gets besides T6_UART: set to 1, it tells a
# runner that T6_UART is this tool's pty (its record says so).
VIA = "T6_UART_RFC2217"
# Bounds: the connection, then the whole negotiation; a reconnect attempt
# is made at most once per RETRY_S; the command's group after it ended.
CONNECT_S = 5
NEGOTIATE_S = 5
RETRY_S = 1
GROUP_WAIT_S = 35
# How often the slave's state is read while it is closed or the session is
# down (poll() cannot wait for the slave's open).
TICK_S = 0.02

# Telnet (RFC 854, 855) and the COM-PORT-OPTION (RFC 2217).
IAC, DONT, DO, WONT, WILL, SB, SE = 255, 254, 253, 252, 251, 250, 240
BINARY, ECHO, SGA, COM_PORT = 0, 1, 3, 44
SET_BAUDRATE, SET_DATASIZE, SET_PARITY, SET_STOPSIZE, SET_CONTROL, PURGE_DATA = 1, 2, 3, 4, 5, 12
SERVER = 100  # a server's answer to command N is N + 100
BAUD = 1500000
NAMES = {SET_BAUDRATE: "SET-BAUDRATE", SET_DATASIZE: "SET-DATASIZE", SET_PARITY: "SET-PARITY",
         SET_STOPSIZE: "SET-STOPSIZE", SET_CONTROL: "SET-CONTROL", PURGE_DATA: "PURGE-DATA"}
# The line: 1500000 baud, 8 data bits, parity 1 (none), stop size 1 (one).
LINE = ((SET_BAUDRATE, struct.pack("!I", BAUD)), (SET_DATASIZE, b"\x08"),
        (SET_PARITY, b"\x01"), (SET_STOPSIZE, b"\x01"))
NO_FLOW = (SET_CONTROL, b"\x01")
PURGE_RX = (PURGE_DATA, b"\x01")
# Options this end agrees to when the server asks; any other is refused.
AGREED = (BINARY, SGA, COM_PORT)
URL = re.compile(r"rfc2217://([A-Za-z0-9._-]+|\[[0-9A-Fa-f:.]+\]):([0-9]{1,5})/?")


def say(msg):
    print("t6-uart-rfc2217: " + msg, file=sys.stderr, flush=True)


def clock(t):
    return time.strftime("%H:%M:%S", time.localtime(t)) + ".%d" % int(t % 1 * 10)


def shown(code, value):
    """A setting's value as a person reads it."""
    if code == SET_BAUDRATE and len(value) == 4:
        return str(struct.unpack("!I", value)[0])
    if len(value) == 1:
        v = value[0]
        return {SET_DATASIZE: str(v),
                SET_PARITY: {1: "none", 2: "odd", 3: "even", 4: "mark", 5: "space"}.get(v, "%d" % v),
                SET_STOPSIZE: {1: "1", 2: "2", 3: "1.5"}.get(v, "%d" % v),
                SET_CONTROL: {1: "no flow control", 2: "XON/XOFF", 3: "hardware flow control"}.get(v, "%d" % v),
                PURGE_DATA: {1: "receive buffer", 2: "transmit buffer", 3: "both buffers"}.get(v, "%d" % v),
                }.get(code, "%d" % v)
    return value.hex() or "nothing"


def parse_url(s):
    """(host, port) of rfc2217://<host>:<port>; ValueError otherwise."""
    m = URL.fullmatch(s)
    if not m or not 0 < int(m.group(2)) < 65536:
        raise ValueError("T6_UART is not rfc2217://<host>:<port>")
    return m.group(1).strip("[]"), int(m.group(2))


def escape(data):
    return data.replace(b"\xff", b"\xff\xff")


class Telnet:
    """The telnet stream from the other end, cut into its parts, in order:
    ("data", bytes), ("opt", DO|DONT|WILL|WONT, option) and ("sb", payload,
    IAC undoubled). Other commands (NOP, GA, ...) carry nothing here and
    are dropped."""

    def __init__(self):
        self.state, self.cmd, self.sb = "data", 0, bytearray()

    def feed(self, chunk):
        out, data = [], bytearray()
        if self.state == "data" and IAC not in chunk:
            return [("data", bytes(chunk))] if chunk else []
        for b in chunk:
            if self.state == "data":
                if b == IAC:
                    self.state = "iac"
                else:
                    data.append(b)
            elif self.state == "iac":
                self.state = "data"
                if b == IAC:
                    data.append(IAC)
                elif b in (DO, DONT, WILL, WONT):
                    self.cmd, self.state = b, "opt"
                elif b == SB:
                    self.sb, self.state = bytearray(), "sb"
            elif self.state == "opt":
                if data:
                    out.append(("data", bytes(data)))
                    data = bytearray()
                out.append(("opt", self.cmd, b))
                self.state = "data"
            elif self.state == "sb":
                if b == IAC:
                    self.state = "sb-iac"
                else:
                    self.sb.append(b)
            else:  # "sb-iac"
                if b == IAC:
                    self.sb.append(IAC)
                    self.state = "sb"
                elif b == SE:
                    if data:
                        out.append(("data", bytes(data)))
                        data = bytearray()
                    out.append(("sb", bytes(self.sb)))
                    self.state = "data"
                else:  # no SE: the block ends here, as malformed
                    self.state = "data"
        if data:
            out.append(("data", bytes(data)))
        return out


class Refused(Exception):
    pass


class Session:
    """One RFC 2217 session with the line set (open() does both), then the
    data both ways. Bytes from the server before the line is set are
    dropped."""

    def __init__(self, sock, line=LINE):
        self.sock, self.line = sock, line
        self.telnet = Telnet()
        self.we, self.they = {}, {}           # option -> True (on) / False (refused)
        self.asked = {(WILL, SGA), (DO, SGA), (DO, COM_PORT), (WILL, COM_PORT)}
        self.answers = {}                     # server's answer code -> value
        self.early = bytearray()              # what came before the line was set
        self.ready = False

    @classmethod
    def open(cls, host, port, line=LINE):
        try:
            sock = socket.create_connection((host, port), timeout=CONNECT_S)
        except OSError as e:
            raise Refused("cannot connect to T6_UART's RFC 2217 server: %s" % (e.strerror or e.__class__.__name__))
        try:
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            s = cls(sock, line)
            s.negotiate(time.monotonic() + NEGOTIATE_S)
            return s
        except BaseException:
            sock.close()
            raise

    def send(self, b):
        self.sock.sendall(bytes(b))

    def sb(self, code, value):
        self.send(bytes((IAC, SB, COM_PORT, code)) + escape(value) + bytes((IAC, SE)))

    def option(self, cmd, opt):
        """The server's DO, DONT, WILL or WONT: agreed to for AGREED, refused
        for any other; answered only when the state changes, so no two
        ends answer each other forever (RFC 854, "loop")."""
        mine = cmd in (DO, DONT)              # DO/DONT are about what this end does
        table = self.we if mine else self.they
        yes, no = (WILL, WONT) if mine else (DO, DONT)
        if cmd in (DO, WILL):
            want = opt in AGREED
            if table.get(opt) != want:
                table[opt] = want
                if (yes, opt) not in self.asked or not want:
                    self.send((IAC, yes if want else no, opt))
        else:
            if table.get(opt) is not False:
                if table.get(opt):
                    self.send((IAC, no, opt))
                table[opt] = False
        self.asked.discard((yes, opt))

    def take(self, chunk):
        """Handles what the server sent; returns its data bytes, once the line is set."""
        data = bytearray()
        for part in self.telnet.feed(chunk):
            if part[0] == "data":
                (data if self.ready else self.early).extend(part[1])
            elif part[0] == "opt":
                self.option(part[1], part[2])
            elif len(part[1]) >= 2 and part[1][0] == COM_PORT:
                self.answers[part[1][1]] = part[1][2:]
        return bytes(data)

    def wait(self, deadline, what, done):
        while not done():
            left = deadline - time.monotonic()
            if left <= 0:
                raise Refused("the server did not answer %s within %d s" % (what, NEGOTIATE_S))
            r, _, _ = select.select([self.sock], [], [], left)
            if r:
                try:
                    chunk = self.sock.recv(4096)
                except OSError:
                    chunk = b""
                if not chunk:
                    said = re.sub(rb"[^ -~]", b".", bytes(self.early[:80])).decode()
                    raise Refused("the server closed the connection while %s was awaited%s" % (
                        what, " (it sent %r)" % said if said else ""))
                self.take(chunk)

    def setting(self, deadline, wanted):
        """Sends the settings, waits for their answers and requires each to
        be the value sent."""
        for code, value in wanted:
            self.answers.pop(code + SERVER, None)
            self.sb(code, value)
        names = "/".join(NAMES[c] for c, _ in wanted)
        self.wait(deadline, names, lambda: all(c + SERVER in self.answers for c, _ in wanted))
        for code, value in wanted:
            got = self.answers[code + SERVER]
            if got != value:
                raise Refused("the server answers %s with %s, not %s: the line is not set" % (
                    NAMES[code], shown(code, got), shown(code, value)))

    def negotiate(self, deadline):
        self.send((IAC, WILL, SGA, IAC, DO, SGA, IAC, DO, COM_PORT, IAC, WILL, COM_PORT))
        self.wait(deadline, "COM-PORT-OPTION (RFC 2217)", lambda: COM_PORT in self.we)
        if not self.we[COM_PORT]:
            raise Refused("the server refuses COM-PORT-OPTION: it does not speak RFC 2217")
        self.setting(deadline, self.line)
        self.setting(deadline, (NO_FLOW,))
        self.setting(deadline, (PURGE_RX,))
        self.ready = True
        self.sock.settimeout(CONNECT_S)


# --------------------------------------------------------------------------
# The pty and the relay.

def make_pty():
    """A pty whose slave is raw with echo off (and 1500000 8N1, which a pty
    only stores); the slave is closed here. Returns (master, slave path)."""
    master, slave = os.openpty()
    try:
        tty.setraw(slave)
        a = termios.tcgetattr(slave)
        a[3] &= ~(termios.ECHO | termios.ECHONL | termios.ICANON | termios.ISIG | termios.IEXTEN)
        a[2] = (a[2] & ~(termios.CSIZE | termios.PARENB | termios.CSTOPB | termios.CRTSCTS)) | termios.CS8 | termios.CLOCAL
        a[4] = a[5] = termios.B1500000
        termios.tcsetattr(slave, termios.TCSANOW, a)
        if termios.tcgetattr(slave)[3] & termios.ECHO:
            raise OSError(errno.EIO, "the pty's echo does not read back off")
        name = os.ttyname(slave)
    except BaseException:
        os.close(master)
        raise
    finally:
        os.close(slave)
    os.set_blocking(master, False)
    return master, name


def slave_open(master):
    """True while some process has the slave open (no POLLHUP on the master)."""
    p = select.poll()
    p.register(master, select.POLLIN)
    return not any(ev & select.POLLHUP for _, ev in p.poll(0))


def flush_slave(name):
    """Drops what the last reader of the slave left unread. A slave its last
    opener left with TIOCEXCL opens for root only (EBUSY): nobody can read
    the bytes then, so there is nothing to do. Any other failure is said
    and the run goes on: the relay itself still works."""
    try:
        fd = os.open(name, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as e:
        if e.errno != errno.EBUSY:
            say("cannot open %s to drop what its last reader left unread: %s" % (name, e.strerror))
        return
    try:
        termios.tcflush(fd, termios.TCIFLUSH)
    except OSError as e:
        say("cannot drop what the last reader of %s left unread: %s" % (name, e.strerror))
    finally:
        os.close(fd)


def read_master(master):
    """What the command wrote to the slave; b"" when nothing is there (EIO is
    the master's answer while no process has the slave open)."""
    try:
        return os.read(master, 65536)
    except BlockingIOError:
        return b""
    except OSError as e:
        if e.errno == errno.EIO:
            return b""
        raise


def group_alive(pgid):
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False


class Relay:
    def __init__(self, host, port, session, master, name, child, signals):
        self.host, self.port, self.session = host, port, session
        self.master, self.name, self.child = master, name, child
        self.is_open = False      # the slave, as last seen
        self.wrote = False        # bytes went into the slave since it was seen closed
        self.lost_at = None       # time.time() the connection was lost
        self.next_try = 0.0
        self.signals = signals    # filled by run()'s handlers

    def lose(self):
        self.session.sock.close()
        self.session = None
        self.lost_at = time.time()
        self.next_try = time.monotonic()

    def reconnect(self):
        try:
            self.session = Session.open(self.host, self.port)
        except Refused:
            self.next_try = time.monotonic() + RETRY_S
            return
        # What the command wrote during the gap is not sent late: it was
        # typed at a board that was not listening.
        termios.tcflush(self.master, termios.TCIFLUSH)
        if not slave_open(self.master) and self.wrote:
            flush_slave(self.name)
            self.wrote = False
        now = time.time()
        say("%s: the connection was lost at %s and the line set again at %s: a gap of %.1f s (its bytes are lost both ways)"
            % (time.strftime("%Y-%m-%d %H:%M:%S"), clock(self.lost_at), clock(now), now - self.lost_at))
        self.lost_at = None

    def to_board(self, data):
        if not data or self.session is None:
            return
        try:
            self.session.send(escape(data))
        except OSError:
            self.lose()

    def to_slave(self, data):
        # Read again now, not at the last poll: a reader that just closed
        # must not leave these in the queue for the next one.
        if not data or not slave_open(self.master):
            return
        try:
            os.write(self.master, data)  # a full slave drops the rest, as a UART's overrun
            self.wrote = True
        except (BlockingIOError, OSError):
            pass

    def watch_slave(self):
        now = slave_open(self.master)
        if self.is_open and not now:
            self.to_board(read_master(self.master))  # the closer's last bytes
            if self.wrote:
                flush_slave(self.name)
            self.wrote = False
        self.is_open = now

    def forward(self, sig, ended):
        if sig == signal.SIGINT or ended:
            try:
                os.killpg(self.child.pid, sig)
            except ProcessLookupError:
                pass
        else:
            self.child.send_signal(sig)

    def loop(self, wake):
        rc = None
        group_deadline = None
        stage = 0                 # after the command ended: 0 wait, 1 TERM sent, 2 KILL sent
        while True:
            while self.signals:
                self.forward(self.signals.pop(0), rc is not None)
            if rc is None and self.child.poll() is not None:
                rc = self.child.returncode
                group_deadline = time.monotonic() + GROUP_WAIT_S
            if rc is not None:
                if not group_alive(self.child.pid):
                    break
                if time.monotonic() > group_deadline:
                    if stage == 2:
                        break
                    sig = (signal.SIGTERM, signal.SIGKILL)[stage]
                    say("processes of the command's group were left %d s after it ended: %s sent to the group"
                        % (GROUP_WAIT_S if stage == 0 else 5, signal.Signals(sig).name))
                    os.killpg(self.child.pid, sig)
                    stage += 1
                    group_deadline = time.monotonic() + 5
            self.watch_slave()
            if self.session is None and time.monotonic() >= self.next_try:
                self.reconnect()
            fds = [wake] + ([self.master] if self.is_open else []) + ([self.session.sock] if self.session else [])
            tick = None if self.is_open and self.session and rc is None else TICK_S
            r, _, _ = select.select(fds, [], [], tick)
            if wake in r:
                try:
                    os.read(wake, 512)
                except BlockingIOError:
                    pass
            if self.master in r:
                self.to_board(read_master(self.master))
            if self.session and self.session.sock in r:
                try:
                    chunk = self.session.sock.recv(65536)
                except OSError:
                    chunk = b""
                if chunk:
                    self.to_slave(self.session.take(chunk))
                else:
                    self.lose()
        if self.lost_at is not None:
            say("the connection lost at %s was not set again before the command ended" % clock(self.lost_at))
        return 128 - rc if rc < 0 else rc


def run(cmd):
    if not cmd:
        say("usage: T6_UART=rfc2217://<host>:<port> t6-uart-rfc2217.py run -- <command> [args...]")
        return NOT_STARTED
    if os.environ.get(VIA):
        say("REFUSED: %s is set: this tool already runs around this command" % VIA)
        return NOT_STARTED
    try:
        host, port = parse_url(os.environ.get("T6_UART", ""))
        session = Session.open(host, port)
    except (ValueError, Refused) as e:
        say("REFUSED: %s; the command was not started" % e)
        return NOT_STARTED
    except KeyboardInterrupt:
        say("interrupted while the session was opened; the command was not started")
        return NOT_STARTED
    master, name = make_pty()
    say("the debug UART over RFC 2217: %d %s%s%s, %s, as the server answered; the command gets it as %s (raw, no echo)" % (
        BAUD, shown(SET_DATASIZE, b"\x08"), "N", shown(SET_STOPSIZE, b"\x01"), shown(*NO_FLOW), name))
    wake_r, wake_w = os.pipe()
    os.set_blocking(wake_r, False)
    os.set_blocking(wake_w, False)
    relay = None
    old = {}
    pending = []              # signals to pass on, in the order they came
    try:
        signal.set_wakeup_fd(wake_w)
        for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            old[s] = signal.signal(s, lambda n, _f: pending.append(n))
        old[signal.SIGCHLD] = signal.signal(signal.SIGCHLD, lambda n, _f: None)
        env = dict(os.environ, T6_UART=name, **{VIA: "1"})
        try:
            child = subprocess.Popen(cmd, env=env, process_group=0)
        except OSError as e:
            say("REFUSED: cannot run %s: %s; nothing was relayed" % (cmd[0], e.strerror))
            return NOT_STARTED
        relay = Relay(host, port, session, master, name, child, pending)
        try:
            return relay.loop(wake_r)
        except Exception as e:
            say("FAILED while the command ran: %r; SIGTERM to the command, waiting for it" % e)
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
                child.wait()
            return FAILED
    finally:
        signal.set_wakeup_fd(-1)
        for s, h in old.items():
            signal.signal(s, h)
        if relay and relay.session:
            relay.session.sock.close()
        elif session and not relay:
            session.sock.close()
        os.close(master)
        os.close(wake_r)
        os.close(wake_w)


# --------------------------------------------------------------------------
# Self-test: a made-up adapter on 127.0.0.1, commands that stand for the
# runners.

class FakeAdapter(threading.Thread):
    """An RFC 2217 server on 127.0.0.1, one client at a time (a second is
    told "busy"). Its line starts at 115200 8N1, as the adapter's
    stored setting, and the board's bytes pass only while the line is
    1500000 8N1: board() drops them otherwise, and what the client
    sends at another line is kept apart (garbled). wrong = (code,
    value): the first such setting is answered with that value and not
    taken. drop() closes the client's connection."""

    def __init__(self, wrong=None):
        super().__init__(daemon=True)
        self.lsock = socket.socket()
        self.lsock.bind(("127.0.0.1", 0))
        self.lsock.listen(4)
        self.port = self.lsock.getsockname()[1]
        self.wrong = wrong
        self.lock = threading.Lock()
        self.conn, self.conns, self.wire, self.cmds = None, 0, [], []
        self.got, self.garbled, self.dropped = bytearray(), bytearray(), 0
        self.line = {}
        self.stop = False

    def line_ok(self):
        return [self.line.get(c) for c, _ in LINE] == [v for _, v in LINE]

    def board(self, data):
        with self.lock:
            if self.conn is None or not self.line_ok():
                self.dropped += len(data)
                return
            self.conn.sendall(escape(data))

    def drop(self):
        with self.lock:
            if self.conn:
                self.conn.shutdown(socket.SHUT_RDWR)

    def send(self, conn, data):
        with self.lock:
            conn.sendall(data)

    def answer(self, conn, opts, cmd, opt):
        mine = cmd in (DO, DONT)
        on = opt in AGREED and cmd in (DO, WILL)
        if opts.get((mine, opt)) != on:
            opts[(mine, opt)] = on
            self.send(conn, bytes((IAC, (WILL if on else WONT) if mine else (DO if on else DONT), opt)))

    def run(self):
        while not self.stop:
            try:
                conn, _ = self.lsock.accept()
            except OSError:
                return
            with self.lock:
                if self.conn is not None:
                    conn.sendall(b"busy\r\n")
                    conn.shutdown(socket.SHUT_WR)
                    threading.Thread(target=self.drain, args=(conn,), daemon=True).start()
                    continue
                self.conn = conn
                self.conns += 1
                self.line = {SET_BAUDRATE: struct.pack("!I", 115200), SET_DATASIZE: b"\x08",
                             SET_PARITY: b"\x01", SET_STOPSIZE: b"\x01"}
                self.wire.append(bytearray())
                self.cmds.append([])
            threading.Thread(target=self.serve, args=(conn, self.wire[-1], self.cmds[-1]), daemon=True).start()

    def serve(self, conn, wire, cmds):
        opts = {}
        tn = Telnet()
        try:
            # as servers do: it offers BINARY both ways itself
            opts[(True, BINARY)] = opts[(False, BINARY)] = True
            self.send(conn, bytes((IAC, DO, BINARY, IAC, WILL, BINARY)))
            while True:
                chunk = conn.recv(4096)
                if not chunk:
                    break
                wire.extend(chunk)
                for part in tn.feed(chunk):
                    if part[0] == "data":
                        with self.lock:
                            (self.got if self.line_ok() else self.garbled).extend(part[1])
                    elif part[0] == "opt":
                        cmds.append(part)
                        self.answer(conn, opts, part[1], part[2])
                    elif part[1][:1] == bytes((COM_PORT,)) and len(part[1]) >= 2:
                        code, value = part[1][1], part[1][2:]
                        cmds.append(("sb", code, value))
                        with self.lock:
                            if self.wrong and self.wrong[0] == code:
                                value, self.wrong = self.wrong[1], None
                            elif code in self.line:
                                self.line[code] = value
                        self.send(conn, bytes((IAC, SB, COM_PORT, code + SERVER)) + escape(value)
                                  + bytes((IAC, SE)))
        except OSError:
            pass
        with self.lock:
            self.conn = None
        conn.close()

    def drain(self, conn):
        """Reads a refused client to its end, so the close sends no reset
        that could overtake "busy"."""
        conn.settimeout(5)
        try:
            while conn.recv(4096):
                pass
        except OSError:
            pass
        conn.close()

    def close(self):
        self.stop = True
        self.lsock.shutdown(socket.SHUT_RDWR)   # wakes accept(); a close alone does not
        self.lsock.close()


def self_test():
    import shutil
    import tempfile
    check = Checks(self_test)
    here = os.path.abspath(__file__)

    # The command: a small program of steps, in argv, that stands for a
    # runner. Files in $T6_TEST_DIR carry the hand-over with the test.
    child_src = r'''
import fcntl, os, select, signal, subprocess, sys, termios, time
d, uart, fd, args = os.environ["T6_TEST_DIR"], os.environ["T6_UART"], None, sys.argv[1:]
def put(name, data=b""):
    with open(os.path.join(d, name), "wb") as f:
        f.write(data)
def wait(name):
    end = time.monotonic() + 15
    while not os.path.exists(os.path.join(d, name)):
        if time.monotonic() > end:
            sys.exit(99)
        time.sleep(0.01)
put("env", ("%s %s" % (uart, os.environ.get("T6_UART_RFC2217", ""))).encode())
while args:
    op, args = args[0], args[1:]
    if op == "open":
        fd = os.open(uart, os.O_RDWR | os.O_NOCTTY)
    elif op == "close":
        os.close(fd)
    elif op == "lflag":
        put(args.pop(0), b"%d" % termios.tcgetattr(fd)[3])
    elif op == "excl":
        fcntl.ioctl(fd, termios.TIOCEXCL)
        put("excl-ok")
    elif op == "echo":
        a = termios.tcgetattr(fd)
        a[3] |= termios.ECHO
        termios.tcsetattr(fd, termios.TCSANOW, a)
    elif op == "holders":
        mine, pids = os.getpid(), []
        real = os.path.realpath(uart)
        for p in os.listdir("/proc"):
            if not p.isdigit() or int(p) == mine:
                continue
            try:
                fds = os.listdir("/proc/%s/fd" % p)
            except OSError:
                continue
            for f in fds:
                try:
                    if os.readlink("/proc/%s/fd/%s" % (p, f)) == real:
                        pids.append(p)
                        break
                except OSError:
                    pass
        fu = subprocess.run(["fuser", uart], capture_output=True) if os.path.exists("/usr/bin/fuser") or os.path.exists("/bin/fuser") else None
        put(args.pop(0), (" ".join(pids) + "|" + ("%d %s" % (fu.returncode, fu.stdout.decode().strip()) if fu else "no fuser")).encode())
    elif op == "mark":
        put(args.pop(0))
    elif op == "wait":
        wait(args.pop(0))
    elif op == "read":
        until, name, got = args.pop(0).encode("latin-1"), args.pop(0), b""
        end = time.monotonic() + 15
        while until not in got and time.monotonic() < end:
            r, _, _ = select.select([fd], [], [], 0.1)
            if r:
                got += os.read(fd, 4096)
        put(name, got)
    elif op == "write":
        os.write(fd, bytes.fromhex(args.pop(0)))
    elif op == "onterm":
        def term(n, f, code=int(args.pop(0))):
            put("got-signal-%d" % n)
            subprocess.Popen(["sleep", "1"])    # left in the group after this exits
            os._exit(code)
        signal.signal(signal.SIGTERM, term)
        signal.signal(signal.SIGINT, term)
    elif op == "sleep":
        time.sleep(float(args.pop(0)))
    elif op == "exit":
        sys.exit(int(args.pop(0)))
'''
    tmp = tempfile.mkdtemp(prefix="t6-uart-rfc2217-")
    child_py = os.path.join(tmp, "child.py")
    with open(child_py, "w") as f:
        f.write(child_src)

    def start(server, steps, url=None, extra=None):
        d = tempfile.mkdtemp(dir=tmp)
        env = dict(os.environ, T6_UART=url or "rfc2217://127.0.0.1:%d" % server.port, T6_TEST_DIR=d)
        env.pop(VIA, None)
        env.update(extra or {})
        p = subprocess.Popen([sys.executable, "-I", here, "run", "--", sys.executable, "-I", child_py] + steps,
                             env=env, stderr=subprocess.PIPE)
        return p, d

    def finish(p):
        try:
            _, err = p.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            p.kill()
            _, err = p.communicate()
        return p.returncode, err.decode(errors="replace")

    def wait_file(d, name, t=15):
        end = time.monotonic() + t
        while not os.path.exists(os.path.join(d, name)):
            if time.monotonic() > end:
                return False
            time.sleep(0.01)
        return True

    def touch(d, name):
        open(os.path.join(d, name), "wb").close()

    def read(d, name):
        try:
            with open(os.path.join(d, name), "rb") as f:
                return f.read()
        except OSError:
            return None

    def quiet(err, server):
        """No message names the address."""
        return "127.0.0.1" not in err and ":%d" % server.port not in err

    def tool_pids(marker):
        """Processes whose environment carries this run's T6_TEST_DIR."""
        out = []
        for p in os.listdir("/proc"):
            if p.isdigit():
                try:
                    with open("/proc/%s/environ" % p, "rb") as f:
                        if marker.encode() in f.read().split(b"\0"):
                            out.append(p)
                except OSError:
                    pass
        return out

    try:
        # 0. The kernel queues what reaches a pty while its slave is closed,
        # and keeps what a reader left unread for the next one: what the
        # tool drops and flushes below would otherwise reach the next reader.
        m, s = os.openpty()
        name = os.ttyname(s)
        tty.setraw(s)
        os.close(s)
        os.write(m, b"old boot")
        fd = os.open(name, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        time.sleep(0.05)
        try:
            queued = os.read(fd, 100)
        except BlockingIOError:
            queued = b""
        check(queued == b"old boot",
              "a bare pty hands bytes written while its slave was closed to the next reader (%r)" % queued)
        os.close(fd)
        os.close(m)

        # 1. The whole relay: the line set as asked, the slave raw with echo
        # off at the command's first open, no holder before it and TIOCEXCL
        # taken, 0xff both ways, nothing sent back, the command's status.
        down = b"\xff\x00board \xff\xff\xffline\r\n\xffEND"
        up = b"\xffhost \x00\xff\xff typed\r\xff"
        srv = FakeAdapter()
        srv.start()
        p, d = start(srv, ["holders", "holders", "open", "lflag", "lflag", "excl", "mark", "opened",
                           "read", "END", "down", "write", up.hex(), "wait", "sent", "close", "exit", "7"])
        opened = wait_file(d, "opened")
        srv.board(down)
        ok_down = wait_file(d, "down")
        end = time.monotonic() + 5
        while bytes(srv.got) != up and time.monotonic() < end:
            time.sleep(0.01)
        time.sleep(0.2)
        touch(d, "sent")
        rc, err = finish(p)
        env = (read(d, "env") or b"").decode().split()
        check(rc == 7, "the tool exits with the command's status 7 (got %d: %s)" % (rc, err.strip()[-300:]))
        check(opened and ok_down and read(d, "down") == down,
              "0xff from the board, doubled on the wire, reaches the command once each (%r)" % read(d, "down"))
        check(bytes(srv.got) == up and not srv.garbled,
              "0xff from the command reaches the board once each, at 1500000 8N1 (%r)" % bytes(srv.got))
        check(bytes(srv.got) == up,
              "echo is off: no byte from the board came back to it (the board got only the command's bytes)")
        lflag = int(read(d, "lflag") or b"-1")
        check(lflag >= 0 and not lflag & (termios.ECHO | termios.ICANON),
              "the slave is raw with echo off at the command's first open (lflag %#x)" % lflag)
        holders = (read(d, "holders") or b"?").decode()
        check(holders.split("|")[0] == "" and holders.split("|")[1] in ("1 ", "no fuser", "1"),
              "no process holds the slave before the command opens it, fuser names none ('%s')" % holders)
        check(read(d, "excl-ok") is not None, "the command takes the slave with TIOCEXCL")
        check(len(env) == 2 and env[0].startswith("/dev/pts/") and env[1] == "1",
              "the command gets T6_UART as the pty's slave and %s=1 (%r)" % (VIA, env))
        sbs = [c for c in srv.cmds[0] if c[0] == "sb"]
        want = [("sb", c, v) for c, v in LINE + (NO_FLOW, PURGE_RX)]
        check(sbs == want, "the client sets the line, no flow control and the purge, nothing else (%r)" % sbs)
        check(bytes(srv.wire[0][:12]) == bytes((IAC, WILL, SGA, IAC, DO, SGA, IAC, DO, COM_PORT, IAC, WILL, COM_PORT)),
              "the session opens with pyserial's requests, without DO ECHO (%r)" % bytes(srv.wire[0][:12]))
        check(not any(c[0] == "opt" and c[2] == ECHO for c in srv.cmds[0]), "the client never asks for telnet ECHO")
        check(quiet(err, srv) and "1500000 8N1, no flow control" in err and env and env[0] in err,
              "stderr says the line and the pty, never the address (%r)" % err.strip())
        check(srv.conns == 1 and not tool_pids(d), "one connection; no process of the run is left")
        srv.close()

        # 2. The fake itself: a client that leaves SET-BAUDRATE out gets no
        # byte of the board, so a tool without it fails the checks above.
        srv = FakeAdapter()
        srv.start()
        s = Session.open("127.0.0.1", srv.port, line=LINE[1:])
        srv.board(b"at 115200")
        s.sock.settimeout(0.5)
        try:
            heard = s.take(s.sock.recv(100))
        except socket.timeout:
            heard = b""
        check(heard == b"" and srv.dropped == 9 and not srv.line_ok(),
              "the made-up adapter passes nothing at its stored 115200 (heard %r)" % heard)
        s.sock.close()
        srv.close()

        # 3. A wrong answer: refused, the command never started.
        srv = FakeAdapter(wrong=(SET_BAUDRATE, struct.pack("!I", 115200)))
        srv.start()
        p, d = start(srv, ["mark", "started", "exit", "0"])
        rc, err = finish(p)
        check(rc == NOT_STARTED and read(d, "started") is None and read(d, "env") is None,
              "an answer of 115200 to SET-BAUDRATE: exit %d, the command not started (rc %d)" % (NOT_STARTED, rc))
        check("REFUSED: the server answers SET-BAUDRATE with 115200, not 1500000" in err and quiet(err, srv),
              "the refusal names the setting and both values, not the address (%r)" % err.strip())
        srv.close()

        # 4. The connection lost once: opened and set again, the gap said,
        # the bytes after it relayed.
        srv = FakeAdapter()
        srv.start()
        p, d = start(srv, ["open", "mark", "opened", "read", "AFTER", "out", "exit", "0"])
        wait_file(d, "opened")
        srv.board(b"BEFORE ")
        time.sleep(0.2)
        srv.drop()
        end = time.monotonic() + 10
        while not (srv.conns == 2 and srv.conn and ("sb", PURGE_DATA, b"\x01") in srv.cmds[1]) \
                and time.monotonic() < end:
            time.sleep(0.01)
        time.sleep(0.2)
        srv.board(b"AFTER")
        rc, err = finish(p)
        out = read(d, "out") or b""
        gap = re.search(r"t6-uart-rfc2217: \d{4}-\d\d-\d\d \d\d:\d\d:\d\d: the connection was lost at "
                        r"\d\d:\d\d:\d\d\.\d and the line set again at \d\d:\d\d:\d\d\.\d: a gap of \d+\.\d s", err)
        check(rc == 0 and out == b"BEFORE AFTER" and srv.conns == 2,
              "a dropped connection is opened again and the bytes after it reach the command (rc %d, %r)" % (rc, out))
        check(gap is not None and err.count("connection was lost") == 1 and quiet(err, srv),
              "one stderr line gives the gap's time and length, no address (%r)" % err.strip())
        check([c for c in srv.cmds[1] if c[0] == "sb"] == want, "the second session sets the line again")
        srv.close()

        # 5. The slave closed: bytes from the board are dropped, and what a
        # reader left unread is not handed to the next one.
        srv = FakeAdapter()
        srv.start()
        p, d = start(srv, ["mark", "closed1", "wait", "sent1", "open", "mark", "open1", "read", "FRESH1", "a",
                           "wait", "sent2", "close", "mark", "closed2", "wait", "go3",
                           "open", "mark", "open2", "read", "FRESH2", "b", "exit", "0"])
        wait_file(d, "closed1")
        srv.board(b"STALE1")
        time.sleep(0.5)
        touch(d, "sent1")
        wait_file(d, "open1")
        srv.board(b"FRESH1")
        wait_file(d, "a")
        srv.board(b"UNREAD")
        time.sleep(0.5)
        touch(d, "sent2")
        wait_file(d, "closed2")
        time.sleep(0.3)
        touch(d, "go3")
        wait_file(d, "open2")
        srv.board(b"FRESH2")
        rc, err = finish(p)
        check(rc == 0 and read(d, "a") == b"FRESH1",
              "bytes that came while no process had the slave open are not delivered (%r)" % read(d, "a"))
        check(read(d, "b") == b"FRESH2",
              "bytes the last reader left unread are not handed to the next one (%r)" % read(d, "b"))
        srv.close()

        # 6. The check of 1 sees echo: a command that turns echo on gets the
        # board's byte sent back to the board.
        srv = FakeAdapter()
        srv.start()
        p, d = start(srv, ["open", "echo", "mark", "opened", "read", "E", "out", "sleep", "0.3", "exit", "0"])
        wait_file(d, "opened")
        srv.board(b"E")
        rc, err = finish(p)
        check(rc == 0 and bytes(srv.got) == b"E", "with echo turned on, the board's byte comes back (%r)" % bytes(srv.got))
        srv.close()

        # 7. SIGTERM and SIGINT to the tool: passed on, the command ends its
        # own way and its status is the tool's; what it left in its group
        # is waited for; nothing of the run is left.
        for sig, code in ((signal.SIGTERM, 5), (signal.SIGINT, 6)):
            srv = FakeAdapter()
            srv.start()
            p, d = start(srv, ["onterm", str(code), "open", "mark", "opened", "sleep", "30"])
            wait_file(d, "opened")
            p.send_signal(sig)
            rc, err = finish(p)
            check(rc == code and read(d, "got-signal-%d" % sig) is not None and not tool_pids(d),
                  "%s: passed to the command, its status %d is the tool's (rc %d), no process left"
                  % (signal.Signals(sig).name, code, rc))
            srv.close()

        # 8. Not started: no address, a closed port, the tool around itself.
        srv = FakeAdapter()
        srv.start()
        p, d = start(srv, ["mark", "started"], url="/dev/ttyUSB0")
        rc, err = finish(p)
        check(rc == NOT_STARTED and "T6_UART is not rfc2217://<host>:<port>" in err and read(d, "started") is None,
              "T6_UART that is no rfc2217:// address: refused (rc %d)" % rc)
        p, d = start(srv, ["mark", "started"], extra={VIA: "1"})
        rc, err = finish(p)
        check(rc == NOT_STARTED and read(d, "started") is None and srv.conns == 0,
              "the tool inside a run of itself: refused before it connects (rc %d)" % rc)
        port = srv.port
        srv.close()
        p, d = start(srv, ["mark", "started"], url="rfc2217://127.0.0.1:%d" % port)
        rc, err = finish(p)
        check(rc == NOT_STARTED and "cannot connect" in err and quiet(err, srv) and read(d, "started") is None,
              "a server that is not there: refused, no address in the message (%r)" % err.strip())
        busy = FakeAdapter()
        busy.start()
        hold = socket.create_connection(("127.0.0.1", busy.port))
        while busy.conns == 0:
            time.sleep(0.01)
        p, d = start(busy, ["mark", "started"])
        rc, err = finish(p)
        check(rc == NOT_STARTED and "'busy..'" in err and quiet(err, busy),
              "an adapter that holds another client: refused with what it said (%r)" % err.strip())
        hold.close()
        busy.close()

        # 9. The parts.
        tn = Telnet()
        parts = tn.feed(b"a\xff\xffb\xff") + tn.feed(b"\xfd\x2c\xff\xfa\x2c\x65\xff\xff\x00\xff\xf0c")
        check(parts == [("data", b"a\xffb"), ("opt", DO, COM_PORT), ("sb", b"\x2c\x65\xff\x00"), ("data", b"c")],
              "the telnet reader across chunks: data, IAC IAC, an option, a subnegotiation (%r)" % parts)
        check(escape(b"\xff\x01\xff") == b"\xff\xff\x01\xff\xff", "escape doubles each 0xff")
        for good in ("rfc2217://adapter.example:4001", "rfc2217://[2001:db8::1]:23", "rfc2217://a-b_c.d:65535/"):
            check(parse_url(good)[1] > 0, "parse_url takes %s" % good)
        for bad in ("adapter:4001", "rfc2217://adapter", "rfc2217://adapter:0", "rfc2217://adapter:70000",
                    "rfc2217://adapter:4001?logging=debug", "rfc2217://a b:1", "/dev/pts/3"):
            try:
                parse_url(bad)
                taken = True
            except ValueError:
                taken = False
            check(not taken, "parse_url refuses %r" % bad)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return check.summary("t6-uart-rfc2217")


class Checks:
    """A self-test's checks, as prompt-reads.py's: every line of the test
    function that calls check() must have run by the end, else the summary
    names the lines."""

    def __init__(self, fn):
        self.fn, self.ran, self.failed, self.n = fn, set(), [], 0

    def __call__(self, cond, what):
        self.n += 1
        self.ran.add(sys._getframe(1).f_lineno)
        if not cond:
            self.failed.append(what)
            print("FAIL: " + what)

    def lines(self):
        code = self.fn.__code__
        with open(code.co_filename) as f:
            src = f.read().splitlines()
        out = []
        for i in range(code.co_firstlineno, len(src)):
            if src[i][:1] not in ("", " ", "#"):
                break
            if re.match(r"\s*check\(", src[i]):
                out.append(i + 1)
        return out

    def summary(self, name):
        not_run = [n for n in self.lines() if n not in self.ran]
        print("%s self-test: %d checks run, %d failed%s" % (
            name, self.n, len(self.failed),
            ", check lines not run: " + " ".join(map(str, not_run)) if not_run else ""))
        return 0 if not self.failed and self.n > 0 and not not_run else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:3] == ["run", "--"]:
        return run(argv[3:])
    say("usage: T6_UART=rfc2217://<host>:<port> %s run -- <command> [args...] | --self-test" % os.path.basename(argv[0]))
    return NOT_STARTED


if __name__ == "__main__":
    sys.exit(main(sys.argv))
