#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The NanoPC-T6's console from a command, given to a runner as a tty.

    T6_CONSOLE_CMD='<shell command>' t6-console-pty.py run -- <command> [args...]

Runs T6_CONSOLE_CMD with `sh -c` (its stdin /dev/null, its stdout the board's
console bytes: whatever the farm provides, e.g. a client of the console's
network endpoint written with the lease's variable names, never their
values), makes a pseudo terminal, and runs <command> with T6_UART set to the
pty's slave path (/dev/pts/N) and T6_UART_VIA=T6_CONSOLE_CMD. It exits with
the command's status. tools/cts/t6-deqp-images.sh --run uses it when
T6_CONSOLE_CMD is set, around the whole tree, so one console session serves
every row and the runner (driver/tools/kexec-zephyr/nanopc-t6.sh) sees the
local tty it already knows how to check and read.

The command line is taken from the environment, not from argv, so `ps` of
this process does not show it; no message of this tool prints it. Write it
with the lease's variable names ("$RESOURCE_..._HOST"): the shell that runs it
expands them.

What it guarantees (the same rules as driver/tools/kexec-zephyr/
t6-uart-rfc2217.py, read only):

  - Nothing is ever written to the board's console. The console command's
    stdin is /dev/null; what a process writes to the slave is read from the
    master and dropped. The T6's console is an autologin root shell.
  - The slave is raw with echo off before any byte is relayed, and this tool
    does not keep it open (fuser names no holder; TIOCEXCL works).
  - While no process has the slave open, the console's bytes are dropped,
    not queued (poll() reports POLLHUP on the master then); when the last
    opener closes the slave with bytes unread, they are flushed, so a row's
    uart.log never starts with the previous row's tail.
  - When the console command ends (a lost connection, a 'busy' adapter), a
    line says so on stderr and, when a process has the slave open, in the
    stream as "#### t6-console-pty: ..."; it is started again after
    T6_CONSOLE_RESTART_S seconds (default 2) while the command runs.

    t6-console-pty.py --self-test
        The bridge against a console command that prints canned lines; no
        board. tools/cts/t6-deqp-run-selftest.sh runs it.

Exit status: the command's; 2 for a usage error (no T6_CONSOLE_CMD, no
command) before anything is started.
"""
import errno
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time
import tty

TAG = "t6-console-pty"


def say(msg):
    sys.stderr.write("%s: %s\n" % (TAG, msg))
    sys.stderr.flush()


def raw_noecho(fd):
    tty.setraw(fd)
    a = termios.tcgetattr(fd)
    a[3] &= ~(termios.ECHO | termios.ECHONL | termios.ICANON)
    termios.tcsetattr(fd, termios.TCSANOW, a)


def slave_open(master):
    """True while some process has the slave open (no POLLHUP on the master)."""
    p = select.poll()
    p.register(master, select.POLLIN | select.POLLHUP)
    for _, ev in p.poll(0):
        if ev & select.POLLHUP:
            return False
    return True


def flush_slave(name):
    """A short open of the slave, TCIFLUSH, close: the bytes nobody read go."""
    try:
        fd = os.open(name, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError:
        return  # TIOCEXCL held by someone: nobody else can read them either
    try:
        termios.tcflush(fd, termios.TCIFLUSH)
    finally:
        os.close(fd)


class Console:
    """The console command, started again when it ends."""

    def __init__(self, cmd, restart_s):
        self.cmd = cmd
        self.restart_s = restart_s
        self.proc = None
        self.next_start = 0.0
        self.starts = 0

    def start(self):
        self.proc = subprocess.Popen(["sh", "-c", self.cmd], stdin=subprocess.DEVNULL,
                                     stdout=subprocess.PIPE, start_new_session=True)
        os.set_blocking(self.proc.stdout.fileno(), False)
        self.starts += 1

    def fd(self):
        return self.proc.stdout.fileno() if self.proc else None

    def ended(self):
        rc = self.proc.wait()
        self.proc.stdout.close()
        self.proc = None
        self.next_start = time.monotonic() + self.restart_s
        return rc

    def stop(self):
        if self.proc:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except OSError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait()
            self.proc.stdout.close()
            self.proc = None


def run(argv):
    cmd = os.environ.get("T6_CONSOLE_CMD", "")
    if not cmd.strip():
        say("T6_CONSOLE_CMD is not set: nothing started")
        return 2
    if not argv:
        say("usage: T6_CONSOLE_CMD='<command>' t6-console-pty.py run -- <command> [args...]")
        return 2
    try:
        restart_s = float(os.environ.get("T6_CONSOLE_RESTART_S", "2"))
    except ValueError:
        say("T6_CONSOLE_RESTART_S is not a number of seconds")
        return 2
    master, slave = pty.openpty()
    name = os.ttyname(slave)
    raw_noecho(slave)
    os.close(slave)  # the slave is the runner's alone
    os.set_blocking(master, False)
    con = Console(cmd, restart_s)
    con.start()
    env = dict(os.environ, T6_UART=name, T6_UART_VIA="T6_CONSOLE_CMD")
    env.pop("T6_CONSOLE_CMD", None)
    child = subprocess.Popen(argv, env=env, start_new_session=True)

    def forward(sig, _frame):
        try:
            os.killpg(child.pid, sig)
        except OSError:
            pass
    signal.signal(signal.SIGTERM, forward)
    signal.signal(signal.SIGHUP, forward)
    signal.signal(signal.SIGINT, forward)

    was_open = False
    relayed = dropped = 0
    note_q = []
    while child.poll() is None:
        if con.proc is None and time.monotonic() >= con.next_start:
            con.start()
            note_q.append("console command started again (start %d)" % con.starts)
        fds = [master] + ([con.fd()] if con.proc else [])
        try:
            r, _, _ = select.select(fds, [], [], 0.2)
        except InterruptedError:
            continue
        now_open = slave_open(master)
        if was_open and not now_open:
            flush_slave(name)
        was_open = now_open
        for n in note_q:
            say(n)
            if now_open:
                try:
                    os.write(master, ("\r\n#### %s: %s\r\n" % (TAG, n)).encode())
                except OSError:
                    pass
        note_q = []
        if master in r:
            # what the runner wrote to the slave: dropped, never sent
            try:
                os.read(master, 4096)
            except OSError as e:
                if e.errno not in (errno.EAGAIN, errno.EIO):
                    raise
        if con.proc and con.fd() in r:
            try:
                data = os.read(con.fd(), 65536)
            except BlockingIOError:
                data = None
            if data == b"":
                rc = con.ended()
                note_q.append("console command ended (rc %d); started again in %g s" % (rc, restart_s))
            elif data:
                if now_open:
                    try:
                        os.write(master, data)
                        relayed += len(data)
                    except OSError:
                        dropped += len(data)
                else:
                    dropped += len(data)
    rc = child.wait()
    con.stop()
    os.close(master)
    say("relayed %d bytes, dropped %d while no reader had the pty open; console command started %d time(s)"
        % (relayed, dropped, con.starts))
    return rc if rc >= 0 else 128 - rc


def self_test():
    """The bridge with a canned console and a reader that stands for the runner."""
    import tempfile
    fails = []
    n = 0

    def check(what, cond):
        nonlocal n
        n += 1
        print("%s %s" % ("ok  " if cond else "FAIL", what))
        if not cond:
            fails.append(what)

    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as d:
        out = os.path.join(d, "read")
        canned = os.path.join(d, "canned.sh")
        with open(canned, "w") as f:
            f.write("i=0; while :; do i=$((i+1)); printf 'line %d\\r\\n' $i; sleep 0.05; done\n")
        reader = ("import os,sys,time,termios\n"
                  "time.sleep(float(os.environ.get('READER_DELAY','0')))\n"
                  "p=os.environ['T6_UART']\n"
                  "a=termios.tcgetattr(os.open(p,os.O_RDONLY|os.O_NOCTTY))\n"
                  "open(sys.argv[1]+'.echo','w').write(str(a[3]&termios.ECHO))\n"
                  "fd=os.open(p,os.O_RDONLY|os.O_NOCTTY|os.O_NONBLOCK)\n"
                  "buf=b'';t=time.time()\n"
                  "while time.time()-t<1.0:\n"
                  "  try: buf+=os.read(fd,4096)\n"
                  "  except BlockingIOError: time.sleep(0.02)\n"
                  "open(sys.argv[1],'wb').write(buf)\n"
                  "open(sys.argv[1]+'.env','w').write(os.environ.get('T6_UART_VIA','')+'|'+os.environ.get('T6_CONSOLE_CMD','-'))\n"
                  "sys.exit(7)\n")
        env = dict(os.environ, T6_CONSOLE_CMD="sh " + canned)
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "run", "--",
                            sys.executable, "-I", "-c", reader, out], env=env,
                           stderr=subprocess.PIPE, timeout=30)
        check("the command's exit status comes back (7)", p.returncode == 7)
        data = open(out, "rb").read() if os.path.exists(out) else b""
        check("the reader got the console's lines through the pty", b"line " in data)
        check("the slave is raw with echo off", open(out + ".echo").read() == "0")
        check("T6_UART_VIA names T6_CONSOLE_CMD and the command line is not passed on",
              open(out + ".env").read() == "T6_CONSOLE_CMD|-")
        check("the summary line counts relayed bytes", b"relayed " in p.stderr)
        # bytes that came while nobody had the slave open were dropped
        env = dict(os.environ, T6_CONSOLE_CMD="sh " + canned, READER_DELAY="0.6")
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "run", "--",
                            sys.executable, "-I", "-c", reader, out], env=env,
                           stderr=subprocess.PIPE, timeout=30)
        data = open(out, "rb").read()
        check("lines from before the reader opened the pty are dropped, not queued",
              b"line " in data and not data.startswith(b"line 1\r\n") and b"dropped 0 " not in p.stderr)
        # a console command that ends: it is started again, and the stream says so
        once = os.path.join(d, "once.sh")
        with open(once, "w") as f:
            f.write("printf 'boot\\r\\n'; sleep 0.2\n")
        env = dict(os.environ, T6_CONSOLE_CMD="sh " + once, T6_CONSOLE_RESTART_S="0.1")
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "run", "--",
                            sys.executable, "-I", "-c", reader, out], env=env,
                           stderr=subprocess.PIPE, timeout=30)
        data = open(out, "rb").read()
        check("an ended console command is started again", b"started again" in p.stderr)
        check("the stream marks the gap with a '#### t6-console-pty:' line",
              b"#### t6-console-pty: console command ended" in data)
        # no command line: refused before anything starts
        env = dict(os.environ)
        env.pop("T6_CONSOLE_CMD", None)
        p = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "run", "--", "true"],
                           env=env, stderr=subprocess.PIPE, timeout=30)
        check("no T6_CONSOLE_CMD: exit 2", p.returncode == 2 and b"not set" in p.stderr)
    print("t6-console-pty self-test: %d checks, %d failed" % (n, len(fails)))
    return 1 if fails else 0


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:3] == ["run", "--"]:
        return run(argv[3:])
    say("usage: T6_CONSOLE_CMD='<command>' %s run -- <command> [args...] | --self-test"
        % os.path.basename(argv[0]))
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
