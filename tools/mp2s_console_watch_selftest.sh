#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_console_watch_selftest.sh - gate-visible selftest of mp2s_console_watch.py.
#
#   tools/mp2s_console_watch_selftest.sh
#
# Runs the tool's built-in --self-test, then black-box cases of its real
# interface (`watch DEVICE RAW STAMPED [--boots N] [--timeout S] [--linger S]`;
# it has no --uart/--until, and exit 4 is "console unreachable") against a
# pseudo-terminal that a python3 -I helper creates and feeds. No serial
# device, no BOARD_UART, no board. Prints "N checks, M failed".
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mp2s_console_watch.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_cw.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
n=0; bad=0
check() { n=$((n+1)); if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; bad=$((bad+1)); fi; }

# helper MODE TOOL DIR [args...]: pty, run `watch <slave> raw st args`, feed
# MODE's bytes, print "rc=<n> secs=<n> sent=<bytes the tool wrote back>".
cat > "$tmp/helper.py" <<'PY'
import os, pty, select, subprocess, sys, time, tty
mode, tool, d = sys.argv[1:4]
extra = sys.argv[4:]
def cyc(left, image, linux):
    t = ["NOTICE:  BL2: v2.10", "U-Boot 2023.10"]
    if left is not None: t.append("mp2z: boots left %d" % left)
    if image: t += ["*** Booting Zephyr OS ***", "mp2: done, PSCI SYSTEM_RESET at 1234 ms"]
    if linux: t.append("Starting kernel ...")
    return t
full = "".join(x + "\r\n" for x in cyc(1,1,0) + cyc(0,1,0) + cyc(None,0,1) + ["login: "]).encode()
m, s = pty.openpty(); tty.setraw(s)
p = subprocess.Popen([sys.executable, "-I", tool, "watch", os.ttyname(s),
                      d + "/raw", d + "/st"] + extra,
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
t0 = time.time(); time.sleep(1)
if mode == "split":
    for i in range(0, len(full), 7):          # splits lines mid-way
        os.write(m, full[i:i+7]); time.sleep(0.01)
elif mode == "boots":
    os.write(m, full)
back = b""
while p.poll() is None and time.time() < t0 + 25:
    r, _, _ = select.select([m], [], [], 0.2)
    if r: back += os.read(m, 4096)
if p.poll() is None: p.kill()
p.communicate()
open(d + "/full", "wb").write(full)
print("rc=%d secs=%d sent=%d" % (p.returncode, time.time() - t0, len(back)))
PY

"$tool" --self-test >"$tmp/builtin.out" 2>&1; check "built-in --self-test" $?

mkdir "$tmp/a"
r=$(python3 -I "$tmp/helper.py" boots "$tool" "$tmp/a" --boots 2 --linger 1 --timeout 20)
case "$r" in "rc=0 "*) check "three boots + login: exit 0" 0;; *) check "three boots + login: exit 0 ($r)" 1;; esac
cmp -s "$tmp/a/raw" "$tmp/a/full"; check "three boots: raw log holds every byte" $?
grep -q "boots left 0" "$tmp/a/st"; check "three boots: stamped log has the last count" $?
case "$r" in *sent=0) check "three boots: nothing sent to the console" 0;; *) check "nothing sent ($r)" 1;; esac

mkdir "$tmp/b"
r=$(python3 -I "$tmp/helper.py" split "$tool" "$tmp/b" --boots 2 --linger 1 --timeout 20)
case "$r" in "rc=0 "*) check "split mid-line: exit 0" 0;; *) check "split mid-line: exit 0 ($r)" 1;; esac
cmp -s "$tmp/b/raw" "$tmp/b/full"; check "split mid-line: raw log holds every byte" $?
c=$(grep -c "Starting kernel \.\.\." "$tmp/b/st"); [ "$c" = 1 ]; check "split mid-line: lines reassembled" $?

mkdir "$tmp/c"
r=$(python3 -I "$tmp/helper.py" none "$tool" "$tmp/c" --boots 2 --timeout 3)
s=${r#*secs=}; s=${s%% *}
case "$r" in
  "rc=1 "*) [ "$s" -le 8 ]; check "no boots until timeout: exit 1 within timeout + few s ($r)" $?;;
  *) check "no boots until timeout: exit 1 ($r)" 1;;
esac
r=$(python3 -I "$tmp/helper.py" none "$tool" "$tmp/c" --timeout 3)
case "$r" in "rc=0 "*) check "no --boots: ends at timeout with exit 0" 0;; *) check "no --boots timeout ($r)" 1;; esac

"$tool" watch "$tmp/nonexistent" "$tmp/r" "$tmp/s" >/dev/null 2>"$tmp/e.out"; rc=$?
[ "$rc" = 4 ]; check "missing device: exit 4" $?
grep -q "cannot reach the console" "$tmp/e.out"; check "missing device: message" $?
"$tool" watch only-one >/dev/null 2>&1; [ $? = 2 ]; check "bad usage: exit 2" $?

echo "$n checks, $bad failed"
[ "$bad" = 0 ]
