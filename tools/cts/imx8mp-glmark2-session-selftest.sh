#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-session-selftest.sh
#
# Self-test of imx8mp-glmark2-session.sh. Host only: no board, no serial line,
# no camera, no network, no git ref. Everything is generated under $TMPDIR
# (exit 2 when it is unset) and nothing is written anywhere else: a synthetic
# frame dump in the sample's line format as the console.txt of a record that
# tools/board.sh would have named, a synthetic llvmpipe reference, and a
# stand-in for `imx8mp-glmark2-dumps.sh run` (IMX8MP_SESSION_DUMPS) that
# copies a prepared record instead of driving a board.
#
# Cases: (a) a clean record goes through every step in order, exit 0, and
# console.txt is unchanged; (b) a dump with a dropped row exits 3 at decode
# and nothing later runs; (c) a record with an unredacted fake address fails
# at redaction and the address is never printed; (d) a record without the
# image's sha256, or without this scene's picture line, fails at
# record-check; (e) a missing reference fails at compare; (f) --dry-run
# prints the board command and creates nothing; (g) an unknown scene and a
# record of another scene exit 2; (h) the run step takes the record the run
# created, not an older one of the same scene, fails when the run leaves
# none, and exits 3 when the new dump is damaged; (i) the two files hold no
# address and no serial device path.
#
# What a case depends on is fixed here, not taken from the host: HOSTNAME
# (imx8mp-glmark2-record-check.py refuses the host's name in a record) and
# the session's own variables are set or unset below.
set -u
here=$(cd "$(dirname "$0")" && pwd)
base=${TMPDIR:-}
if [ -z "$base" ]; then
    echo "TMPDIR is not set" >&2
    exit 2
fi
work=$(mktemp -d "$base/imx8mp-session.XXXXXX") || exit 2
trap '[ -n "${KEEP:-}" ] || rm -rf "$work"' EXIT
unset IMX8MP_IMG_DIR IMX8MP_RECORDS IMX8MP_MAIN IMX8MP_SESSION_DUMPS BOARD_UART PICTURES WDOG_S
export HOSTNAME=session-selftest-host
session=$here/imx8mp-glmark2-session.sh
fail=0

# check NAME: ok or FAIL by the status of the command before it
check() {
    local status=$?
    if [ "$status" = 0 ]; then
        echo "ok   $1"
    else
        echo "FAIL $1"
        fail=$((fail + 1))
    fi
}
# absent PATTERN FILE...: no line of the files matches
absent() {
    if grep -q -e "$1" "${@:2}"; then return 1; fi
    return 0
}

# The synthetic capture, 160x120 (the tools take the size from the dump):
# clean.log, bad.log (row 20 dropped) and the reference ref.png (the frame
# with a changed rectangle).
cat > "$work/gen.py" << 'PY'
import base64, binascii, importlib.util, os, sys
here, out = sys.argv[1], sys.argv[2]
spec = importlib.util.spec_from_file_location("fb", os.path.join(here, "glmark2-fb.py"))
fb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fb)
W, H = 160, 120
scene = "[build] use-vbo=false"


def px(x, y):
    if 60 <= x < 100 and 40 <= y < 80:
        return bytes((250, 40 + (x & 63), 30))
    return bytes(((x * 255) // W, (y * 255) // H, ((x + y) * 255) // (W + H)))


rows = [b"".join(px(x, y) for x in range(W)) for y in range(H)]
probes, c, cq, mean = fb.picture_value(W, H, rows)
pic = ("%dx%d; probes c %s00, tl %s00, tr %s00, bl %s00, br %s00; crc16 %04x, crc16q %04x; "
       "mean %d %d %d; black no" % ((W, H) + tuple(p.hex() for p in probes) + (c, cq) + tuple(mean)))
lines = ["*** Booting Zephyr OS build v4.x ***", "gc_wdog_arm: armed",
         "glmark2-fb: %s %dx%d begin" % (scene, W, H)]
for y, r in enumerate(rows):
    lines.append("glmark2-fb: %d %04x %s" % (y, fb.crc16(r), base64.b64encode(r).decode()))
lines.append("glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows)) & 0xFFFFFFFF))
lines.append("glmark2-picture: %s: %s" % (scene, pic))


def wr(name, ls):
    with open(os.path.join(out, name), "w") as f:
        f.write("".join("12:%02d:%02d %s\n" % (i // 600 % 60, i // 10 % 60, l) for i, l in enumerate(ls)))


wr("clean.log", lines)
wr("bad.log", [l for l in lines if not l.startswith("glmark2-fb: 20 ")])
chg = [bytearray(r) for r in rows]
for y in range(30, 90):
    for x in range(20, 140):
        chg[y][x * 3:x * 3 + 3] = bytes((0, 0, 255))
fb.write_png(os.path.join(out, "ref.png"), W, H, [bytes(r) for r in chg])
PY
mkdir "$work/cap" "$work/ref"
if ! python3 -I "$work/gen.py" "$here" "$work/cap"; then
    echo "FAIL generating the synthetic capture"
    exit 1
fi

label=gm01-build
slug=build-use-vbo-false
# The record's sha8 is the image's sha256 prefix (tools/board.sh). The commit
# starts with the same 8 digits, so the record passes a record-check that
# compares the name with the commit as well as one that compares it with the
# image.
commit=0123456789abcdef0123456789abcdef01234567
image=${commit}89abcdef0123456789abcdef
cp "$work/cap/ref.png" "$work/ref/$slug.png"

# mkrec DIR CAPTURE [STAMP]: a record in DIR; prints its path
mkrec() {
    local d=$1/${3:-20261125-1200}-$label-${image:0:8}
    mkdir -p "$d" || return 1
    cp "$2" "$d/console.txt"
    printf 'name=%s\ncommit=%s\nimage_sha256=%s\n' "$label" "$commit" "$image" > "$d/meta.txt"
    echo "$d"
}
# run NAME ARGS...: the session with ARGS; stdout NAME.out, stderr NAME.err, status in rc
run() {
    local name=$1
    shift
    rc=0
    bash "$session" "$@" > "$work/$name.out" 2> "$work/$name.err" || rc=$?
}
show() { sed "s/^/     $1: /" "$work/$1.err" "$work/$1.out"; }

# (a) a clean record
rec=$(mkrec "$work/a" "$work/cap/clean.log")
run a 1 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 0 ]
check "(a) clean record exits 0 (got $rc)"
[ "$rc" = 0 ] || show a
order=$(sed -n 's/^session: step \([a-z-]*\): .*/\1/p' "$work/a.out" | paste -sd' ' -)
[ "$order" = "resolve run decode redaction record-check compare judge" ]
check "(a) the steps in order (got: $order)"
grep -q "120 of 120 rows good" "$work/a.out"
check "(a) the decoder's line is shown"
[ -s "$rec/$label.png" ] && [ -s "$rec/$label.picdiff.png" ] && [ -s "$rec/$label.sidebyside.png" ]
check "(a) picture, picdiff and side-by-side are in the record"
cmp -s "$rec/console.txt" "$work/cap/clean.log"
check "(a) console.txt is unchanged"
grep -q "imx8mp-glmark2-note-fill.py --note docs/notes/imx8mp-glmark2-pictures-by-eye.md --scene $label --commit $commit --record records/imx8mp/$(basename "$rec")/ --route dump --verdict right" "$work/a.out"
check "(a) the note-fill command names scene, commit, record and route"
run a2 "$label" --dry-run --ref-dir "$work/ref" --record-dir "$rec"
[ "$rc" = 0 ] && grep -q "^session: step judge: " "$work/a2.out"
check "(a) by label, and --dry-run with a record redoes the checks (got $rc)"

# (b) a dropped row
rec=$(mkrec "$work/b" "$work/cap/bad.log")
run b 1 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 3 ]
check "(b) dropped row exits 3 (got $rc)"
grep -q "^session: step decode failed: .*re-run, never patch" "$work/b.err"
check "(b) names the decode step, re-run never patch"
grep -q "row 20 missing" "$work/b.err"
check "(b) the decoder's row line is shown"
absent "^session: step \(decode\|redaction\|record-check\|compare\|judge\): " "$work/b.out"
check "(b) nothing after decode ran"
[ ! -e "$rec/$label.png" ] && [ -s "$rec/$label.damaged.png" ] && [ ! -e "$rec/$label.sidebyside.png" ]
check "(b) the damaged picture does not keep the record's name"

# (c) an unredacted fake address (TEST-NET-1, put together here)
rec=$(mkrec "$work/c" "$work/cap/clean.log")
fake=192.0.2.$((70 + 7))
echo "board=$fake" >> "$rec/meta.txt"
run c 1 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 1 ] && grep -q "^session: step redaction failed: " "$work/c.err"
check "(c) an address fails at redaction (got $rc)"
absent "^session: step \(record-check\|compare\|judge\): " "$work/c.out"
check "(c) nothing after redaction ran"
if grep -qF "$fake" "$work/c.out" "$work/c.err"; then false; fi
check "(c) the address is never printed"

# (d) no image sha256 in the record
rec=$(mkrec "$work/d" "$work/cap/clean.log")
printf 'name=%s\ncommit=%s\n' "$label" "$commit" > "$rec/meta.txt"
run d 1 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 1 ] && grep -q "^session: step record-check failed: " "$work/d.err" && grep -q "^sha256: " "$work/d.err"
check "(d) a record without the image's sha256 fails at record-check (got $rc)"
absent "^\(photograph\|console\): " "$work/d.err"
check "(d) other scenes' and photograph lines are left out"
absent "^session: step \(compare\|judge\): " "$work/d.out"
check "(d) nothing after record-check ran"
grep -v "glmark2-picture: " "$work/cap/clean.log" > "$work/cap/nopicture.log"
rec=$(mkrec "$work/d2" "$work/cap/nopicture.log")
run d2 1 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 1 ] && grep -q "^session: step record-check failed: " "$work/d2.err" && grep -q "^console: .* scene 01 " "$work/d2.err"
check "(d) a console without this scene's picture line fails at record-check (got $rc)"
[ "$(grep -c "^console: " "$work/d2.err")" = 1 ]
check "(d) only this scene's console line is reported"

# (e) no reference picture
rec=$(mkrec "$work/e" "$work/cap/clean.log")
mkdir "$work/noref"
run e 1 --record-dir "$rec" --ref-dir "$work/noref"
[ "$rc" = 1 ] && grep -q "^session: step compare failed: " "$work/e.err"
check "(e) a missing reference fails at compare (got $rc)"
absent "^session: step judge: " "$work/e.out"
check "(e) judge did not run"

# (f) dry run: the board command, nothing created
rc=0
IMX8MP_RECORDS=$work/f-records bash "$session" 24 --dry-run > "$work/f.out" 2> "$work/f.err" || rc=$?
[ "$rc" = 0 ]
check "(f) dry run exits 0 with BOARD_UART unset (got $rc)"
[ "$rc" = 0 ] || show f
grep -q "tools/board.sh imx8mp run .*gm24-shadow/zephyr/zephyr.bin .* gm24-shadow " "$work/f.out"
check "(f) prints the board run command of gm24-shadow"
absent "^session: step \(decode\|redaction\|record-check\|compare\|judge\): " "$work/f.out"
check "(f) no later step ran"
[ ! -e "$work/f-records" ]
check "(f) no records directory was created"

# (g) bad scene, record of another scene, bad arguments
run g1 99 --dry-run
[ "$rc" = 2 ] && grep -q "^session: step resolve failed: " "$work/g1.err"
check "(g) scene 99 fails at resolve, exit 2 (got $rc)"
run g2 gm01-nosuch --dry-run
[ "$rc" = 2 ] && grep -q "^session: step resolve failed: " "$work/g2.err"
check "(g) an unknown label fails at resolve, exit 2 (got $rc)"
rec=$(mkrec "$work/g" "$work/cap/clean.log")
run g3 2 --record-dir "$rec" --ref-dir "$work/ref"
[ "$rc" = 2 ] && grep -q "^session: step run failed: .*not a record of gm02-build" "$work/g3.err"
check "(g) a record of another scene is refused at run, exit 2 (got $rc)"
run g4 1 --record-dir "$work/g/none" --ref-dir "$work/ref"
[ "$rc" = 2 ] && grep -q "^session: step run failed: " "$work/g4.err"
check "(g) a record directory that does not exist is refused, exit 2 (got $rc)"
run g5
[ "$rc" = 2 ] && grep -q "^usage: " "$work/g5.err"
check "(g) no scene: usage, exit 2 (got $rc)"
run g6 1 --no-such-option
[ "$rc" = 2 ]
check "(g) an unknown option: exit 2 (got $rc)"

# (h) the run step, with a stand-in for the dumps script's run: `scenes` is
# the real one; `run <img> <records> <label>` copies $FAKE_CAPTURE (if set) as
# a new record and exits $FAKE_RC
cat > "$work/dumps.sh" << 'SH'
#!/bin/bash
if [ "$1" != run ]; then exec bash "$REAL_DUMPS" "$@"; fi
echo "stand-in run: $2 $3 $4"
if [ -n "${FAKE_CAPTURE:-}" ]; then
    d=$3/20261126-0900-$4-$FAKE_SHA8
    mkdir -p "$d" && cp "$FAKE_CAPTURE" "$d/console.txt" && cp "$FAKE_META" "$d/meta.txt" || exit 9
fi
exit "${FAKE_RC:-0}"
SH
export REAL_DUMPS=$here/imx8mp-glmark2-dumps.sh FAKE_SHA8=${image:0:8} FAKE_META=$work/meta.txt
printf 'name=%s\ncommit=%s\nimage_sha256=%s\n' "$label" "$commit" "$image" > "$FAKE_META"
# runh NAME RECORDS: the session's run step on RECORDS through the stand-in
runh() {
    rc=0
    IMX8MP_SESSION_DUMPS=$work/dumps.sh IMX8MP_IMG_DIR=$work/img IMX8MP_RECORDS=$2 \
        bash "$session" 1 --ref-dir "$work/ref" > "$work/$1.out" 2> "$work/$1.err" || rc=$?
}
old=$(mkrec "$work/h1" "$work/cap/bad.log" 20261125-0800)
FAKE_CAPTURE=$work/cap/clean.log runh h1 "$work/h1"
[ "$rc" = 0 ]
check "(h) a run that leaves a clean record exits 0 (got $rc)"
[ "$rc" = 0 ] || show h1
grep -q "^stand-in run: $work/img $work/h1 $label\$" "$work/h1.out"
check "(h) the dumps script's run got image directory, records directory and label"
grep -q "^session: step run: record 20261126-0900-$label-${image:0:8} " "$work/h1.out"
check "(h) the record the run created is taken"
[ ! -e "$old/console-live.txt" ] && [ ! -e "$old/$label.damaged.png" ]
check "(h) the older record of the scene is not touched"
old=$(mkrec "$work/h2" "$work/cap/clean.log" 20261125-0800)
FAKE_RC=1 runh h2 "$work/h2"
[ "$rc" = 1 ] && grep -q "^session: step run failed: .*no new record" "$work/h2.err"
check "(h) a run that leaves no record fails at run, the older record is not used (got $rc)"
[ ! -e "$old/$label.png" ]
check "(h) the older record was not decoded"
FAKE_CAPTURE=$work/cap/bad.log FAKE_RC=1 runh h3 "$work/h3"
[ "$rc" = 3 ] && grep -q "^session: step decode failed: " "$work/h3.err"
check "(h) a run whose dump is damaged exits 3 at decode (got $rc)"
FAKE_CAPTURE=$work/cap/clean.log FAKE_RC=1 runh h4 "$work/h4"
[ "$rc" = 1 ] && grep -q "^session: step run failed: .*exited 1 though the dump decodes" "$work/h4.err"
check "(h) a failed run with a whole dump fails at run (got $rc)"
rc=0
IMX8MP_SESSION_DUMPS=$work/dumps.sh IMX8MP_RECORDS=$work/h5 bash "$session" 1 > "$work/h5.out" 2> "$work/h5.err" || rc=$?
[ "$rc" = 2 ] && grep -q "^session: step run failed: IMX8MP_IMG_DIR" "$work/h5.err" && absent "stand-in run" "$work/h5.out"
check "(h) without IMX8MP_IMG_DIR nothing runs, exit 2 (got $rc)"

# (i) no address, no serial device path in the two files (the patterns are put
# together here so that this file does not hold them either)
quad='([0-9]{1,3}\.){3}[0-9]{1,3}'
tty=/dev/tt
if grep -Eq -e "$quad" -e "${tty}y" "$session" "$here/imx8mp-glmark2-session-selftest.sh"; then false; fi
check "(i) no address and no serial device path in the tool and its self-test"

if [ "$fail" = 0 ]; then
    echo "imx8mp-glmark2-session self-test: all cases passed"
    exit 0
fi
echo "imx8mp-glmark2-session self-test: $fail case(s) FAILED" >&2
exit 1
