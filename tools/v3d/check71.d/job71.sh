#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# job71: the V3D 7.1 job builder (driver/src/v3d/v3d_job71.c, -DV3D_CL_VER=71
# over v3d_frame.c at 7.1), which the 7.1 GL table drives, against the jobs
# Mesa 26.2.2 submitted on the Raspberry Pi 5. tools/v3d/job71_check.c builds
# each job with every buffer at the board dump's address and under its name
# and decodes it with Mesa's CLE decoder for a version 71 device:
#  (a) clear: the 64x64 clear to 0xffff0000 equals tools/v3d/pi5/clear.clif
#      through tools/v3d/clif_cmp.py with nothing ignored (no address field,
#      no store format: the target is the board's UIF texture; the tile
#      allocation size at the Pi 5 kernel's 16 KiB granule); the census and
#      no list byte left undecoded; 19 / 96 / 27 bytes.
#  (b) triangle: the same clear then Mesa's triangle with Mesa's 7.1 shaders
#      (tools/v3d/tri-shaders71) equals tools/v3d/pi5/triangle.clif the same
#      way, the D0 shader record and the attribute records field by field
#      (clif_cmp.py's SHADREC_* pseudo-packets), the uniform streams, the QPU
#      words and the vertex data word by word; the board's layout (126 / 96
#      bytes, record +0x60, tile list +0xa0..+0xbb) and D0 flag word; a draw
#      refused after writing its state leaves nothing behind.
#  (c) multi: 3 draws (arrays, indexed, instanced) into 8 render targets of
#      mixed formats and tilings (one GL_NONE) with a D24S8 buffer loaded and
#      stored: every list decodes in full with the job's packet census, and
#      the RENDER_TARGET_PART2/PART3 clear colours (40 and 56 bits, which
#      Mesa's decoder and unpacker read wrong) are checked byte by byte.
#  (d) refusals: job71_check's refused inputs are refused.
# Also: v3d_job71.c built at 7.1 with -Wall -Wextra -Werror, and at 4.2
# defines nothing.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
# Run as `sh job71.sh` too: a POSIX shell hands it to bash before any bash syntax.
if [ -z "${BASH_VERSION:-}" ]; then exec bash "$0" "$@"; fi
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=job71
dir=build/v3d71/$name
mkdir -p "$dir/tmp" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
  libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
  libs=("$mb/src/broadcom/libbroadcom_v3d.a" "$mb/src/broadcom/cle/libbroadcom_cle.a"
    "$mb/src/broadcom/libv3d_neon.a" "$mb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi
inc+=(-Idriver/src/v3d -Idriver/src/v3d/generated)

fails=0
fail() {
  echo "FAIL: $*"
  fails=$((fails + 1))
}

# ---- build: the builder at 7.1 and 4.2 with our warnings, the check against Mesa ----
{
  cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" -c driver/src/v3d/v3d_frame.c \
    -o "$dir/v3d_frame71.o" &&
    cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" -c driver/src/v3d/v3d_job71.c \
      -o "$dir/v3d_job71.o" &&
    cc -O1 -std=c11 -Wall -Wextra -Werror "${inc[@]}" -c driver/src/v3d/v3d_job71.c \
      -o "$dir/v3d_job71_42.o" &&
    cc -O1 -std=c11 -Wall -Werror -DV3D_CL_VER=71 "${inc[@]}" -Itools/v3d/tri-shaders71 \
      tools/v3d/job71_check.c "$dir/v3d_frame71.o" "$dir/v3d_job71.o" "${libs[@]}" \
      -o "$dir/job71_check"
} >"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}
syms=$(nm "$dir/v3d_job71_42.o" | grep -c ' [TDBR] ')
if [ "$syms" -ne 0 ]; then
  fail "v3d_job71.c built at 4.2 defines $syms symbol(s), expected none"
fi

# The binary words of a buffer range in two CLIF dumps (clif_cmp.py reads no
# binary data): bin.py A B BUF:START:END ...
cat >"$dir/bin.py" <<'EOF'
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""bin.py A B BUF:START:END ...: the words of buffer BUF in [START, END) are
present and equal in the binary sections of the CLIF dumps A and B."""
import re
import sys


def words(path):
    out, buf, off, on = {}, None, 0, False
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            ln = re.sub(r"^MESA: info: ?", "", raw.rstrip("\n"))
            m = re.match(r"@buffer (\S+)", ln)
            if m:
                buf, on = m.group(1), False
                continue
            m = re.match(r"@format binary /\* \[(\S+)\+0x([0-9a-f]+)\] \*/", ln)
            if m:
                buf, off, on = m.group(1), int(m.group(2), 16), True
                continue
            if ln.startswith("@"):
                on = False
                continue
            if on:
                for w in ln.split():
                    if re.fullmatch(r"0x[0-9a-f]{8}", w):
                        out[(buf, off)] = int(w, 16)
                        off += 4
    return out


wa, wb = words(sys.argv[1]), words(sys.argv[2])
n = bad = 0
for spec in sys.argv[3:]:
    buf, s, e = spec.split(":")
    for off in range(int(s, 0), int(e, 0), 4):
        n += 1
        va, vb = wa.get((buf, off)), wb.get((buf, off))
        if va is None or va != vb:
            bad += 1
            if bad <= 10:
                print("bin: %s+0x%x: %s != %s" % (buf, off, va, vb))
print("bin: %d words, %d differ" % (n, bad))
sys.exit(1 if bad else 0)
EOF

# ---- (a), (b): the board's jobs ----
# $1 clear|triangle, $2 the board's dump; sets "<mode>_line"
board() {
  local mode=$1 refclif=$2 out rc last
  out=$(TMPDIR=$dir/tmp JOB71_CLIF=$dir/$mode.clif "$dir/job71_check" "$mode" 2>&1)
  rc=$?
  printf '%s\n' "$out" >"$dir/$mode.out"
  last=$(printf '%s\n' "$out" | tail -n 1)
  if [ "$rc" -ne 0 ] || ! printf '%s\n' "$last" | grep -q ": ok$"; then
    fail "($mode) job71_check $mode: exit $rc"
    printf '%s\n' "$out" | sed 's/^/  /'
    return
  fi
  python3 -I tools/v3d/clif_cmp.py "$dir/$mode.clif" "$refclif" >"$dir/$mode.cmp" 2>&1 || {
    fail "($mode) differs from $refclif:"
    sed 's/^/  /' "$dir/$mode.cmp"
  }
  local line
  line=$(tail -n 1 "$dir/$mode.cmp" | sed 's/^clif_cmp: //')
  if [ "$mode" = triangle ]; then
    # the uniform streams in front of the record, the three programs (FS
    # 0x60, VS 0xa8, CS 0x160 .. 0x1d8) and the vertex buffer's 24 words
    python3 -I "$dir/bin.py" "$dir/$mode.clif" "$refclif" CL_0x119000:0:0x60 \
      resource_0x1000:0x60:0x1d8 resource_0x15000:0:0x60 >"$dir/$mode.bin" 2>&1 || {
      fail "($mode) the uniforms, shader words or vertex data differ from $refclif:"
      sed 's/^/  /' "$dir/$mode.bin"
    }
    line+=", $(tail -n 1 "$dir/$mode.bin" | sed 's/^bin: //')"
  fi
  printf -v "${mode}_line" '%s' "$line"
  echo "($mode) $refclif: $line"
}

clear_line="" triangle_line=""
board clear tools/v3d/pi5/clear.clif
board triangle tools/v3d/pi5/triangle.clif

# ---- (c), (d) ----
run() {
  local mode=$1 out rc last
  out=$(TMPDIR=$dir/tmp JOB71_CLIF=$dir/$mode.clif "$dir/job71_check" "$mode" 2>&1)
  rc=$?
  printf '%s\n' "$out" >"$dir/$mode.out"
  last=$(printf '%s\n' "$out" | tail -n 1)
  if [ "$rc" -ne 0 ] || ! printf '%s\n' "$last" | grep -q ": ok$"; then
    fail "($mode) job71_check $mode: exit $rc"
    printf '%s\n' "$out" | sed 's/^/  /'
    return
  fi
  echo "($mode) ${last#job71_check: }"
}
run multi
run refusals
refused=$(sed -n 's/.*refusals \([0-9]* of [0-9]*\) refused: ok$/\1/p' "$dir/refusals.out")

summary="(a) clear: ${clear_line:-not run}; (b) triangle: ${triangle_line:-not run}; (c) multi; (d) ${refused:-?} refused"
if [ "$fails" -ne 0 ]; then
  echo "$name: $summary, $fails failed"
  exit 1
fi
echo "$name: $summary, 0 failed"
exit 0
