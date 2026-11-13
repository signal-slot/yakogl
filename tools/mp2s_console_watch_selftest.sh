#!/usr/bin/env bash
# Selftest of tools/mp2s_console_watch.py: no board. A pty pair carries a
# scripted console of 3 boots; checks the line count, the boot count, both
# stop conditions, the exit 2 case and that nothing reached the master side.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_watch.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
exec python3 -I -c "$(cat <<'PY'
import os, pty, re, select, subprocess, sys, time
watch, tmp = sys.argv[1:3]
fails = []

def check(name, ok, info=""):
    print(("ok   " if ok else "FAIL ") + name + ("" if ok else " " + str(info)))
    if not ok:
        fails.append(name)

def script():
    out = []
    for i, why in enumerate(["Power-on reset (por_rstn)", "System reset (SYSRST)",
                             "System reset (SYSRST)"]):
        out += ["NOTICE:  BL2: v2.10-stm32mp-r3.1(release):v2.10",
                "NOTICE:  Reset reason: %s (0x1)" % why, "U-Boot 2023.10",
                "Hit any key to stop autoboot:  0", "mp2s: boots left %d" % (2 - i),
                "mp2: image done"]
    return out + ["stm32mp2 login:"]          # 3 * 6 + 1 = 19 lines

def run(args, feed):
    m, s = pty.openpty()
    path = os.ttyname(s)
    log = os.path.join(tmp, "log%d" % len(os.listdir(tmp)))
    p = subprocess.Popen([sys.executable, "-I", watch, "-o", log] + args,
                         env=dict(os.environ, BOARD_UART=path),
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    time.sleep(0.5)
    for l in feed:
        os.write(m, (l + "\r\n").encode())
        time.sleep(0.01)
    got = b""                                  # whatever the watcher wrote to the console
    while p.poll() is None:
        if select.select([m], [], [], 0.2)[0]:
            got += os.read(m, 4096)
    out, err = p.communicate()
    if select.select([m], [], [], 0.1)[0]:
        got += os.read(m, 4096)
    os.close(m)
    os.close(s)
    return p.returncode, open(log).read(), got, path, out + err

rc, txt, got, path, o = run(["--boots", "3", "--until", "login:", "--timeout", "20"], script())
body = [l for l in txt.splitlines() if l.startswith("[+")]
check("until: exit 0", rc == 0, rc)
check("until: 19 lines", len(body) == 19, len(body))
check("until: 3 boots", "boots=3 " in txt.splitlines()[-1], txt.splitlines()[-1])
check("until: timestamps", all(re.match(r"\[\+\d{3,}\.\d{3}\] ", l) for l in body))
check("zero bytes written to the console", got == b"", got)
check("UART path not in log or output", path not in txt and path.encode() not in o, path)
check("header names $BOARD_UART", "uart=$BOARD_UART" in txt.splitlines()[0])

rc, txt, got, path, o = run(["--boots", "3", "--until", "login:", "--timeout", "3"],
                            script()[:12] + ["stm32mp2 login:"])
check("until before K boots: exit 4", rc == 4, rc)
check("until before K boots: 2 boots", "boots=2 " in txt.splitlines()[-1])
check("timeout: nothing written", got == b"", got)

rc, txt, got, path, o = run(["--boots", "1", "--until", "NEVER", "--timeout", "2"], script())
check("no match: exit 4", rc == 4, rc)
check("no match: 3 boots counted", "boots=3 " in txt.splitlines()[-1])
check("no match: nothing written", got == b"", got)

p = subprocess.run([sys.executable, "-I", watch, "-o", os.path.join(tmp, "x"),
                    "--uart", os.path.join(tmp, "nonexistent")], capture_output=True)
check("unopenable: exit 2", p.returncode == 2, p.returncode)

src = open(watch).read()
check("source: O_RDONLY|O_NOCTTY, no write call",
      "os.O_RDONLY | os.O_NOCTTY" in src and "os.write" not in src and ".send(" not in src)
print("%d failed" % len(fails) if fails else "all ok")
sys.exit(1 if fails else 0)
PY
)" "$here/mp2s_console_watch.py" "$tmp"
