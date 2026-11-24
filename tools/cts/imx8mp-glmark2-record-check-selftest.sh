#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-record-check.py: builds synthetic records under $TMPDIR.
# Host only, no board. Usage: bash tools/cts/imx8mp-glmark2-record-check-selftest.sh
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
chk="$here/imx8mp-glmark2-record-check.py"
T=$(mktemp -d "${TMPDIR:-/var/tmp}/rc-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
export HOSTNAME=selftest-host
fail=0
D=. M= A=@  # pieces: the tracked file holds no address, path or login
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# the script's scene list must equal imx8mp-glmark2-dumps.sh scenes
want=$(bash "$here/imx8mp-glmark2-dumps.sh" scenes 2>/dev/null | sed -E 's/^gm[0-9]+-[a-z0-9]+ //; s/ -b .*$//')
got=$(python3 -I -c '
import importlib.util, sys
s = importlib.util.spec_from_file_location("rc", sys.argv[1]); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
print("\n".join(m.SCENES))' "$chk")
[ -n "$want" ] && [ "$want" = "$got" ] && ok "scene list equals imx8mp-glmark2-dumps.sh scenes" || bad "scene list differs from dumps.sh"

# mkrec DIR: a good synthetic record
mkrec() {
    python3 -I -c '
import importlib.util, os, struct, sys, zlib
d, chk = sys.argv[1], sys.argv[2]
s = importlib.util.spec_from_file_location("rc", chk); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
def chunk(t, b): return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\1\2\3")) + chunk(b"IEND", b"")
os.makedirs(d + "/photos")
open(d + "/console.log", "w").write("".join("glmark2-picture: %s: 800x600; probes c 000000ff, tl 000000ff; black no\n" % i for i in m.SCENES))
for n, i in enumerate(m.SCENES, 1):
    open("%s/photos/%02d-%s.png" % (d, n, m.slug(i)), "wb").write(png)
open(d + "/meta.txt", "w").write("commit=0123456789abcdef0123456789abcdef01234567\nimage_sha256=" + "ab" * 32 + "\n")
open(d + "/sha256", "w").write("ab" * 32 + "  image\n")' "$1" "$chk"
}
run() { python3 -I "$chk" "$@" 2>&1; }
# expect WHAT EXITCODE PATTERN DIR
expect() {
    local what=$1 rc=$2 pat=$3 out got
    shift 3
    out=$(run "$@"); got=$?
    if [ "$got" = "$rc" ] && { [ -z "$pat" ] || printf '%s\n' "$out" | grep -qE -- "$pat"; }; then ok "$what"
    else bad "$what (exit $got)"; printf '%s\n' "$out" | sed 's/^/     /'; fi
}
fresh() { rm -rf "$T/w"; mkdir -p "$T/w"; R="$T/w/20261120-1030-eye-01234567"; mkrec "$R"; }

fresh; expect "good record is complete" 0 'record-check: .*: complete \(0 problems\)$' "$R"
expect "no argument" 2 ''
expect "missing directory" 2 '' "$T/nope"
fresh; mv "$R" "$T/w/eye-run"; expect "bad name" 1 '^name: ' "$T/w/eye-run"
fresh; printf 'commit=ffffffff00000000\nimage_sha256=%s\n' "$(printf 'ab%.0s' $(seq 32))" > "$R/meta.txt"
expect "sha8 differs from commit" 1 'commit: recorded commit does not start' "$R"
fresh; sed -i '5d' "$R/console.log"; expect "console line missing" 1 'console: no glmark2-picture line for scene 05' "$R"
fresh; rm "$R"/photos/07-*.png; expect "photograph missing" 1 'photograph: missing 07-' "$R"
fresh; rm "$R"/photos/03-*.png; echo "not a png" > "$R/photos/03-x.png"; expect "invalid PNG" 1 'photograph: photos/03-x.png: not a PNG' "$R"
fresh; f=$(ls "$R"/photos/04-*); head -c 40 "$f" > "$f.t"; mv "$f.t" "$f"; expect "truncated PNG" 1 'photograph: photos/04-.*(truncated|IEND)' "$R"
fresh; f=$(ls "$R"/photos/04-*); python3 -I -c "
import sys; p=sys.argv[1]; b=bytearray(open(p,'rb').read()); b[-20]^=0xff; open(p,'wb').write(b)" "$f"
expect "corrupt PNG" 1 'photograph: photos/04-' "$R"
fresh; cp "$R"/photos/01-*.png "$R/photos/01-dup.png"; expect "duplicate photograph" 1 'scene 01 has 2 files' "$R"
fresh; rm "$R/sha256"; sed -i '/image_sha256/d' "$R/meta.txt"; expect "sha256 missing" 1 '^sha256: ' "$R"
fresh; echo "board 192${D}0${D}2${D}7 up" >> "$R/console.log"; expect "ipv4" 1 'redaction: console.log:[0-9]+: ipv4' "$R"
fresh; echo "cd /ho${M}me/x" >> "$R/console.log"; expect "/home/" 1 'home-path' "$R"
fresh; echo "/Us${M}ers/x" >> "$R/meta.txt"; expect "/Users/" 1 'users-path' "$R"
fresh; echo "open /dev/t${M}tyUSB3" >> "$R/console.log"; expect "/dev/tty" 1 'tty-path' "$R"
fresh; echo "root${A}board" >> "$R/console.log"; expect "user@host" 1 'login' "$R"
fresh; echo "uname selftest-host" >> "$R/console.log"; expect "host name" 1 'host-name' "$R"
fresh; echo "192${D}0${D}2${D}9" >> "$R/console.log"; out=$(run "$R")
if printf '%s' "$out" | grep -q '192[.]0[.]2'; then bad "hit text printed"; else ok "hit text never printed"; fi
exit $fail
