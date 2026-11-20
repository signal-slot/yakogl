#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# triangle71: the driver's V3D 7.1 clear-then-triangle frame
# (driver/src/v3d/v3d_frame71_tri.c over v3d_frame.c, -DV3D_CL_VER=71; the
# design: docs/notes/v3d71-triangle-frame.md) with Mesa's own 7.1 shaders
# (tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h), against the job
# Mesa 26.2.2 submitted for the same triangle on the Raspberry Pi 5
# (tools/v3d/pi5/triangle.clif). tools/v3d/frame_check.c built at 7.1 with
# -DFRAME_CHECK_TRIANGLE (--triangle71) builds the job with every buffer at
# the dump's address and under its name, and decodes it with Mesa's CLE
# decoder for a version 71 device into CLIF text:
#  (a) the decoder reads every list in full: the census of the board's lists
#      (no other packet, no "Unknown", no decoder error), four control-list
#      sections, one shader record and two attribute records, no list byte
#      left over as binary data; the lists are the board's 126 / 96 bytes and
#      the indirect buffer the board's layout (uniforms +0x00, record +0x60,
#      attribute records +0x80, tile list +0xa0..+0xbb).
#  (b) against tools/v3d/pi5/triangle.clif: every packet of every list and
#      every submit argument through tools/v3d/clif_cmp.py with only
#      IGNORED below left out (no address field, not the tile allocation
#      size: it is the board's 16384); the shader
#      record and the attribute records field by field (clif_cmp.py reads no
#      record section); the QPU words of the three programs, the uniform
#      streams and the vertex data word by word. The record is the BCM2712
#      D0 layout ("GL Shader State Record Draw Index"), which Mesa's decoder
#      prints with the plain 7.1 layout on both sides
#      (tools/v3d/pi5/README.md, "Differences" item 4): the two plain decodes
#      must be equal as they stand, and frame_check also unpacks our record
#      at the D0 layout and checks every field, and the 24 flag bits, against
#      the board's record as the header gives it.
#  (c) against tools/v3d/shim71/triangle.clif (drm-shim, V3D 7.1.7, the plain
#      7.1 record), the job built at the shim's addresses: the lists and
#      submit as in (b); the plain decodes of the two records differ in
#      exactly SHIM_DIFFERS (the D0 layout's flags from bit 5 up sit 2 to 4
#      bits higher), and our record read at the D0 layout equals the shim's
#      record read at its own, plain, layout in every field the plain layout
#      has.
#  (d) refused inputs are refused: frame_check's 20 changed frames (each must
#      make v3d_frame_build_triangle return false), and an unknown placement.
# Also: v3d_frame71_tri.c built at 4.2 defines nothing.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=triangle71
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

pi5=tools/v3d/pi5/triangle.clif
shim=tools/v3d/shim71/triangle.clif

# IGNORED: the fields clif_cmp.py leaves out, each with its reason.
#  STORE.Memory Format, STORE.Height in UB or Stride: our frame stores the
#    target as a raster buffer with a byte stride (256), Mesa's GL target is a
#    UIF texture with a height in UIF blocks (8); frame_check.sh's
#    FORMAT_FIELDS, docs/notes/v3d71-clear-frame.md 1.3. The two dummy stores
#    of the render list have 0 in both either way.
# Nothing else. No address field is ignored: the job is built with every
# buffer at the dump's address. Nor is the tile allocation size (ADD_BIN.arg3),
# which the clear's comparison (frame_check.sh (d)) must leave to a rounding
# rule: for one draw v3d_frame_alloc_sizes gives 16384 at 4 KiB pages, the
# board's (16 KiB buffers) and the shim's alike; it is also checked to be ours
# rounded up to 16 KiB (tools/v3d/pi5/README.md "Differences" item 2).
IGNORED=(--ignore 'STORE.Memory Format' --ignore 'STORE.Height in UB or Stride')
# SHIM_DIFFERS: the record fields whose plain-layout decode differs between
# the D0 record (ours, the board's) and the shim's plain 7.1.7 record, with
# the D0 bit the plain decode reads for each (README "Differences" item 4).
SHIM_DIFFERS=('Fragment shader uses real pixel centre W in addition to centroid W2'
  'Insert Primitive ID as first varying to fragment shader'
  'Disable implicit point/line varyings')

fails=0
fail() {
  echo "FAIL: $*"
  fails=$((fails + 1))
}

# ---- build: the builder at 7.1 with our warnings, frame_check against Mesa ----
{
  cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" -c driver/src/v3d/v3d_frame.c \
    -o "$dir/v3d_frame71.o" &&
    cc -O1 -std=c11 -Wall -Wextra -Werror -DV3D_CL_VER=71 "${inc[@]}" \
      -c driver/src/v3d/v3d_frame71_tri.c -o "$dir/v3d_frame71_tri.o" &&
    cc -O1 -std=c11 -Wall -Wextra -Werror "${inc[@]}" -c driver/src/v3d/v3d_frame71_tri.c \
      -o "$dir/v3d_frame71_tri_42.o" &&
    cc -O1 -std=c11 -Wall -Werror -DV3D_CL_VER=71 -DFRAME_CHECK_TRIANGLE "${inc[@]}" \
      -Itools/v3d/tri-shaders71 tools/v3d/frame_check.c "$dir/v3d_frame71.o" \
      "$dir/v3d_frame71_tri.o" "${libs[@]}" -o "$dir/frame_check_tri71"
} >"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}
syms=$(nm "$dir/v3d_frame71_tri_42.o" | grep -c ' [TDBR] ')
if [ "$syms" -ne 0 ]; then
  fail "v3d_frame71_tri.c built at 4.2 defines $syms symbol(s), expected none"
fi

# The record, attribute-record and binary comparisons (clif_cmp.py reads no
# record section and no binary data).
cat >"$dir/rec_cmp.py" <<'EOF'
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""rec_cmp.py recs A B [label ...]: the shadrec sections of two CLIF dumps,
field by field; exit 0 when exactly the labels given differ (in the main
record). rec_cmp.py d0 OUT B: frame_check's "d0 <label>: <v>" lines against
B's main record, for every label B has and every value that is not an
address. rec_cmp.py bin A B BUF:START:END ...: the binary words of buffer BUF
in [START, END) equal in A and B."""
import re
import sys


def lines(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return [re.sub(r"^MESA: info: ?", "", ln.rstrip("\n")) for ln in f]


FIELD = re.compile(r"^\s+/\*\s*(.*?):\s*\*/\s*(.*)$")


def recs(path):
    out, cur = [], None
    for ln in lines(path):
        if ln.startswith("@"):
            m = re.match(r"@format (shadrec_\w+)", ln)
            cur = None
            if m:
                cur = (m.group(1), [])
                out.append(cur)
            continue
        if cur is not None:
            m = FIELD.match(ln)
            if m:
                cur[1].append((m.group(1), m.group(2)))
    return out


def bins(path):
    words, buf, off, on = {}, None, 0, False
    for ln in lines(path):
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
                    words[(buf, off)] = int(w, 16)
                    off += 4
    return words


def main():
    mode, a, b = sys.argv[1], sys.argv[2], sys.argv[3]
    if mode == "recs":
        want = set(sys.argv[4:])
        ra, rb = recs(a), recs(b)
        if [k for k, _ in ra] != [k for k, _ in rb]:
            print("rec_cmp: sections %s != %s" % ([k for k, _ in ra], [k for k, _ in rb]))
            return 1
        n, diff = 0, set()
        bad = False
        for (k, fa), (_, fb) in zip(ra, rb):
            if [x for x, _ in fa] != [x for x, _ in fb]:
                print("rec_cmp: %s: the field lists differ" % k)
                bad = True
                continue
            for (x, va), (_, vb) in zip(fa, fb):
                n += 1
                if va != vb:
                    print("rec_cmp: %s %s: %s != %s" % (k, x, va, vb))
                    if k == "shadrec_gl_main":
                        diff.add(x)
                    else:
                        bad = True
        if diff != want:
            for x in sorted(want - diff):
                print("rec_cmp: %s: expected to differ, equal" % x)
            bad = True
        print("rec_cmp: %d sections, %d fields, %d differ (%d expected)" % (len(ra), n, len(diff), len(want)))
        return 1 if bad else 0
    if mode == "d0":
        ours = {}
        for ln in lines(a):
            m = re.match(r"^d0 (.*?): (\S+)$", ln)
            if m:
                ours[m.group(1)] = int(m.group(2), 0)
        rb = [f for k, f in recs(b) if k == "shadrec_gl_main"]
        if len(rb) != 1:
            print("rec_cmp: %s: %d main records" % (b, len(rb)))
            return 1
        n, bad = 0, 0
        for x, v in rb[0]:
            if v.startswith("["):
                continue
            n += 1
            tv = int(v.split()[0], 0)
            if x not in ours:
                print("rec_cmp: %s: not in the D0 record" % x)
                bad += 1
            elif ours[x] != tv:
                print("rec_cmp: %s: ours at D0 %d, %s's %d" % (x, ours[x], b, tv))
                bad += 1
        print("rec_cmp: d0 %d fields, %d differ" % (n, bad))
        return 1 if bad else 0
    if mode == "bin":
        wa, wb = bins(a), bins(b)
        n, bad = 0, 0
        for spec in sys.argv[4:]:
            buf, s, e = spec.split(":")
            for off in range(int(s, 0), int(e, 0), 4):
                n += 1
                va, vb = wa.get((buf, off)), wb.get((buf, off))
                if va is None or va != vb:
                    bad += 1
                    if bad <= 10:
                        print("rec_cmp: %s+0x%x: %s != %s" % (buf, off, va, vb))
        print("rec_cmp: %d words, %d differ" % (n, bad))
        return 1 if bad else 0
    return 2


sys.exit(main())
EOF

# ---- one placement: build, (a), the lists, the records and the words ----
# $1 pi5|shim, $2 the reference dump, $3.. the record fields expected to
# differ in the plain decode. Sets "<where>_line" for the summary.
check_against() {
  local where=$1 refclif=$2
  shift 2
  local out rc last
  out=$(TMPDIR=$dir/tmp FRAME_CHECK_CLIF=$dir/$where.clif "$dir/frame_check_tri71" --triangle71 \
    "$where" 2>&1)
  rc=$?
  printf '%s\n' "$out" >"$dir/$where.out"
  last=$(printf '%s\n' "$out" | tail -n 1)
  if [ "$rc" -ne 0 ] || ! printf '%s\n' "$last" | grep -q ": ok$"; then
    fail "($where) frame_check --triangle71 $where: exit $rc"
    printf '%s\n' "$out" | grep -v '^d0 ' | sed 's/^/  /'
    return
  fi
  case $last in
  *"BCL 126 RCL 96 indirect 187 bytes (record +0x60, tile list +0xa0..+0xbb)"*) ;;
  *) fail "($where) not the board's list sizes and layout: $last" ;;
  esac
  python3 -I tools/v3d/clif_cmp.py "$dir/$where.clif" "$refclif" "${IGNORED[@]}" \
    >"$dir/$where.cmp" 2>&1 || {
    fail "($where) differs from $refclif:"
    sed 's/^/  /' "$dir/$where.cmp"
  }
  qms() { sed 's/^MESA: info: \{0,1\}//' "$1" | awk '/^@add_bin/ { n = 4; next } n && --n == 0 { print $1; exit }'; }
  local ours board
  ours=$(qms "$dir/$where.clif") board=$(qms "$refclif")
  if ! [[ $ours =~ ^[0-9]+$ && $board =~ ^[0-9]+$ ]] || [ "$board" -ne $(((ours + 16383) / 16384 * 16384)) ]; then
    fail "($where) tile allocation size: ours '$ours', the dump's '$board' (want ours rounded up to 16 KiB)"
  fi
  python3 -I "$dir/rec_cmp.py" recs "$dir/$where.clif" "$refclif" "$@" >"$dir/$where.recs" 2>&1 || {
    fail "($where) the shader or attribute records differ from $refclif:"
    sed 's/^/  /' "$dir/$where.recs"
  }
  python3 -I "$dir/rec_cmp.py" d0 "$dir/$where.out" "$refclif" >"$dir/$where.d0" 2>&1
  local d0rc=$?
  # the code buffer's three programs (FS 0x60, VS 0xa8, CS 0x160 .. 0x1d8),
  # the uniform streams in front of the record, the vertex buffer's 24 words
  local code ind vtx
  case $where in
  pi5) code=resource_0x1000 ind=CL_0x119000 vtx=resource_0x15000 ;;
  shim) code=resource_0x1000 ind=CL_0x10a000 vtx=resource_0x9000 ;;
  esac
  python3 -I "$dir/rec_cmp.py" bin "$dir/$where.clif" "$refclif" "$code:0x60:0x1d8" "$ind:0:0x60" \
    "$vtx:0:0x60" >"$dir/$where.bin" 2>&1 || {
    fail "($where) the shader words, uniforms or vertex data differ from $refclif:"
    sed 's/^/  /' "$dir/$where.bin"
  }
  local line
  line="$(tail -n 1 "$dir/$where.cmp" | sed 's/^clif_cmp: //'), record fields $(tail -n 1 "$dir/$where.recs" |
    sed 's/^rec_cmp: [0-9]* sections, //'), words $(tail -n 1 "$dir/$where.bin" | sed 's/^rec_cmp: //'), tile allocation $ours"
  if [ "$where" = shim ]; then
    # (c): our record at the D0 layout is the shim's at the plain layout
    if [ "$d0rc" -ne 0 ]; then
      fail "(shim) our record read at the D0 layout differs from the shim's plain record:"
      sed 's/^/  /' "$dir/$where.d0"
    fi
    line+=", D0 against plain: $(tail -n 1 "$dir/$where.d0" | sed 's/^rec_cmp: d0 //')"
  fi
  printf -v "${where}_line" '%s' "$line"
  echo "($where) $refclif: $line"
}

pi5_line="" shim_line=""
# (a), (b)
check_against pi5 "$pi5"
# (c): the plain decodes differ in SHIM_DIFFERS, by the D0 layout
check_against shim "$shim" "${SHIM_DIFFERS[@]}"

# (d) an unknown placement is refused too (the 20 changed frames ran above)
if TMPDIR=$dir/tmp "$dir/frame_check_tri71" --triangle71 nowhere >"$dir/nowhere.out" 2>&1; then
  fail "(d) --triangle71 nowhere: built"
fi
refused=$(sed -n 's/.*, \([0-9]* of [0-9]*\) refused: ok$/\1/p' "$dir/pi5.out" | tail -n 1)

summary="(a)+(b) pi5: ${pi5_line:-not run}; (c) shim: ${shim_line:-not run}; (d) ${refused:-?} refused"
if [ "$fails" -ne 0 ]; then
  echo "$name: $summary, $fails failed"
  exit 1
fi
echo "$name: $summary, 0 failed"
exit 0
