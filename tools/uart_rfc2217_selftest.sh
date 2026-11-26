#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of tools/uart_rfc2217.py against a fake RFC 2217 server on
# 127.0.0.1 (a free port). No board, no network beyond loopback.
#   bash tools/uart_rfc2217_selftest.sh
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cli="$here/uart_rfc2217.py"
work=$(mktemp -d "${TMPDIR:-/tmp}/uart2217.XXXXXX") || exit 2
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$work"; }
trap cleanup EXIT

ok=0; failed=0
check() { # name, status (0 = pass)
    if [ "$2" -eq 0 ]; then ok=$((ok + 1)); echo "ok   $1"
    else failed=$((failed + 1)); echo "FAIL $1"; fi
}

cat > "$work/fake.py" <<'PY'
# Fake RFC 2217 server. argv: MODE PORTFILE RECVFILE
#   MODE ok     acknowledge the baud rate, send the payload, keep the line open
#        wrong  answer the baud rate with 9600
#        busy   send "busy" and close
#        close  accept and close
import re, socket, struct, sys, threading, time
mode, portfile, recvfile = sys.argv[1:4]
IAC, SB, SE, WILL, DO = 255, 250, 240, 251, 253
PAYLOAD = (b"hello\xff\xffworld\r\n"
           + bytes((IAC, 241))                                  # NOP
           + bytes((IAC, WILL, 1))                              # WILL ECHO inline
           + bytes((IAC, SB, 44, 107, 0, IAC, SE))              # COM-PORT notify inline
           + b"login: ")
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0)); srv.listen(8)
open(portfile, "w").write(str(srv.getsockname()[1]))
SBRE = re.compile(rb"\xff\xfa(.*?)\xff\xf0", re.S)
def serve(c):
    if mode == "close":
        c.close(); return
    if mode == "busy":
        c.sendall(b"busy\r\n"); c.settimeout(1)
        try: c.recv(4096)
        except OSError: pass
        c.close(); return
    c.sendall(bytes((IAC, DO, 44, IAC, WILL, 44, IAC, DO, 0, IAC, WILL, 0, IAC, DO, 3, IAC, WILL, 3)))
    buf, sent = b"", False
    while True:
        try: d = c.recv(4096)
        except OSError: return
        if not d: return
        buf += d
        for m in SBRE.finditer(buf):
            body = m.group(1)
            if body[:2] == bytes((44, 1)):
                baud = struct.unpack("!I", body[2:6])[0]
                if mode == "wrong": baud = 9600
                c.sendall(bytes((IAC, SB, 44, 101)) + struct.pack("!I", baud) + bytes((IAC, SE)))
                if mode == "ok" and not sent:
                    sent = True; c.sendall(PAYLOAD)
        buf = SBRE.sub(b"", buf)
        buf = re.sub(rb"\xff[\xfb-\xfe].", b"", buf)
        if buf.endswith(b"\xff"):          # a split sequence: keep it for the next read
            keep, buf = buf[-1:], buf[:-1]
        else:
            keep = b""
        if buf:
            with open(recvfile, "ab") as f: f.write(buf.replace(b"\xff\xff", b"\xff"))
        buf = keep
while True:
    c, _ = srv.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
PY

# start MODE: starts a fake server, sets $port and $recv
start() {
    rm -f "$work/port"; recv="$work/recv.$1.$RANDOM"; : > "$recv"
    python3 "$work/fake.py" "$1" "$work/port" "$recv" & pids+=($!)
    for _ in $(seq 100); do [ -s "$work/port" ] && break; sleep 0.05; done
    port=$(cat "$work/port")
}
waitfor() { # file, bytes-pattern file ... : wait until FILE contains the bytes of PATTERNFILE
    for _ in $(seq 100); do python3 - "$1" "$2" <<'PY' && return 0
import sys
import os
sys.exit(0 if os.path.exists(sys.argv[1]) and open(sys.argv[2], "rb").read() in open(sys.argv[1], "rb").read() else 1)
PY
        sleep 0.1; done; return 1
}
printf 'hello\xffworld\r\nlogin: ' > "$work/want"

# --help and addresses
help=$(python3 "$cli" --help 2>&1); [[ $help == *probe* && $help == *capture* && $help == *pty* ]]
check "help lists probe, capture, pty" $?
python3 "$cli" probe nonsense 2>/dev/null; check "bad address exits 2" $([ $? -eq 2 ] && echo 0 || echo 1)

# probe
start ok
python3 "$cli" probe "rfc2217://127.0.0.1:$port" 2>"$work/err"; check "probe: ok exits 0" $?
python3 "$cli" probe "127.0.0.1:$port" 2>/dev/null; check "probe: bare host:port exits 0" $?
! grep -q 127.0.0.1 "$work/err"; check "probe: no address in messages" $?
start busy
python3 "$cli" probe "127.0.0.1:$port" 2>"$work/err"; check "probe: busy exits 4" $([ $? -eq 4 ] && echo 0 || echo 1)
start close
python3 "$cli" probe "127.0.0.1:$port" 2>/dev/null; check "probe: closed at once exits 5" $([ $? -eq 5 ] && echo 0 || echo 1)
python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1])" > "$work/free"
python3 "$cli" probe "127.0.0.1:$(cat "$work/free")" 2>"$work/err"; rc=$?
check "probe: unreachable exits 3" $([ $rc -eq 3 ] && echo 0 || echo 1)
! grep -q "127.0.0.1\|$(cat "$work/free")" "$work/err"; check "probe: unreachable message names no address" $?
start wrong
python3 "$cli" probe "127.0.0.1:$port" 2>"$work/err"; rc=$?
check "probe: wrong baud exits 42" $([ $rc -eq 42 ] && echo 0 || echo 1)
grep -q '<BOARD_UART>' "$work/err"; check "exit 42 message says <BOARD_UART>" $?
! grep -q 127.0.0.1 "$work/err"; check "exit 42 message names no address" $?

# capture
start ok
python3 "$cli" capture "rfc2217://127.0.0.1:$port" "$work/raw" "$work/ts" 2>"$work/err" & cp=$!
waitfor "$work/raw" "$work/want"; check "capture: raw holds the payload, no Telnet bytes" $?
kill -TERM $cp; wait $cp; check "capture: SIGTERM exits 0" $?
cmp -s "$work/raw" "$work/want"; check "capture: raw is exactly the payload" $?
grep -q ' hello.*world$' "$work/ts" && grep -q '^[0-9]*\.[0-9]* login: $' "$work/ts"
check "capture: stamped lines, unfinished line written on SIGTERM" $?
start wrong
python3 "$cli" capture "127.0.0.1:$port" "$work/raw2" "$work/ts2" 2>/dev/null
check "capture: wrong baud exits 42" $([ $? -eq 42 ] && echo 0 || echo 1)
start busy
python3 "$cli" capture "127.0.0.1:$port" "$work/raw3" "$work/ts3" 2>/dev/null
check "capture: busy exits 4" $([ $? -eq 4 ] && echo 0 || echo 1)
python3 "$cli" capture "127.0.0.1:$(cat "$work/free")" "$work/raw4" "$work/ts4" 2>/dev/null
check "capture: unreachable exits 4" $([ $? -eq 4 ] && echo 0 || echo 1)

# pty, both ways (needs a pseudo-terminal: a sandbox without /dev/ptmx cannot run it)
if python3 -c 'import os; os.close(os.openpty()[0])' 2>/dev/null; then
start ok
python3 "$cli" pty "127.0.0.1:$port" > "$work/slave" 2>"$work/err" & pp=$!
for _ in $(seq 100); do [ -s "$work/slave" ] && break; sleep 0.05; done
python3 - "$(head -n1 "$work/slave")" "$work/want" <<'PY'
import os, sys, time
fd = os.open(sys.argv[1], os.O_RDWR | os.O_NOCTTY)  # already raw: setraw would flush the pending payload
want, got, end = open(sys.argv[2], "rb").read(), b"", time.time() + 10
while want not in got and time.time() < end:
    try: got += os.read(fd, 4096)
    except OSError: break
if want not in got: sys.exit(1)
os.write(fd, b"ping\xff\x01\n")
PY
check "pty: board to tty carries the payload without Telnet bytes" $?
printf 'ping\xff\x01\n' > "$work/ping"
waitfor "$recv" "$work/ping"
check "pty: tty to board arrives unescaped (0xFF sent as IAC IAC)" $?
kill -TERM $pp; wait $pp; check "pty: SIGTERM exits 0" $?
start wrong
python3 "$cli" pty "127.0.0.1:$port" 2>/dev/null
check "pty: wrong baud exits 42" $([ $? -eq 42 ] && echo 0 || echo 1)
else
    echo "skip pty: no pseudo-terminal on this host"
fi

echo "uart_rfc2217 selftest: $ok ok, $failed failed"
[ "$failed" -eq 0 ]
