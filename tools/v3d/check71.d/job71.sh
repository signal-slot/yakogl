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
#  (e) G1: a D24S8 depth buffer stored and loaded as Z only, as stencil only
#      and as both (v3d_job71_clear's zs_store_aspects / zs_load_aspects),
#      the LOAD and STORE packets read back from the decode (buffer, the
#      image's D24S8 format, UIF, padded height); aspects the buffer does not
#      hold (a D16's stencil, `zs`'s stencil beside a separate `s`, unknown
#      bits) refused.
#  (f) gl: every job of Mesa's 7.1 references for the gl_test shapes,
#      tools/v3d/pi5/gl_{scissor,scisbox,writemask,viewport,fbods,colormask,
#      mrt4,mrt8}.clif (29 jobs: cases 5, 58, 59, 66, 67, 102 and the 4- and
#      8-target clears), built by job71_check from each shape's GL state and
#      compared job by job by gl_cmp.py (below) with every address left out:
#      the binning list as the state each draw sees (Mesa writes only what
#      changed since the draw before), the render list, the tile list, the
#      shader and attribute records and the tile allocation size packet by
#      packet. A remaining difference must match a line of gl.allow (below),
#      which names it and gives its reason, or it fails; a gl.allow line that
#      matches nothing fails too. Each reference also compares equal to
#      itself, and a Z/S store put in place of writemask's Z-only one is found.
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

# ---- (e) G1: a D24S8 loaded or stored as Z only or as stencil only ----
run g1
g1=$(sed -n 's/^job71_check: g1 \([0-9]* of [0-9]*\) .*, \([0-9]* of [0-9]*\) refused: ok$/\1 loads and stores, \2 refused/p' "$dir/g1.out")

# ---- (f) gl: the jobs of Mesa's 7.1 references for the gl_test shapes ----
# gl_cmp.py: job k of ours against job k of the reference (see its text);
# gl.allow: the differences v3d_job71 still has, each named with its reason.
cat >"$dir/gl_cmp.py" <<'EOF'
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""gl_cmp.py OURS REF SHAPE ALLOW: compare the jobs of two multi-job CLIF
dumps (V3D_DEBUG=clif text, one job per @add_bin/@add_render pair), job k with
job k, every field whose value is a buffer address left out (as
clif_cmp.py --ignore-addresses; nothing else is left out).

Per job:
  bcl     the binning list as the state each draw sees: the packets up to
          START_TILE_BINNING one by one; for every draw, the last value of each
          state packet before it (Mesa writes only the state that changed since
          the draw before, v3d_job71 all of it on every draw) and the draw
          packet; the packets after the last draw one by one;
  rcl     the render list packet by packet;
  tiles   the generic tile list packet by packet;
  rec     the shader records and attribute records in order, field by field;
  submit  the @add_bin / @add_render arguments that are not addresses (the
          tile allocation size).
A difference is a line "<shape> job <k> <part> ...: <ours> != <ref>". ALLOW
holds "<shape pattern> | <difference pattern> | <id> | <reason>" lines
(fnmatch patterns; a line applies to the shapes its first pattern matches): a
difference that matches is a named difference, printed as "named <id>: ..."
and not a failure; an applying line that no difference matches is stale (a
failure). Every other difference is printed as it is. The last line is
"gl_cmp: <shape> J jobs, N packets, D differences, A named (<id> n, ...), S
stale; R state packets of ours repeat the state in effect" (N: the
reference's packets and records). Exit 0 when D and S are 0.
"""
import difflib
import fnmatch
import re
import sys

FIELD = re.compile(r"^\s+/\*\s*(.*?):\s*\*/\s*(.*)$")
PACKET = re.compile(r"^[A-Z][A-Z0-9_]*$")
DRAWS = ("VERTEX_ARRAY_PRIMS", "VERTEX_ARRAY_INSTANCED_PRIMS", "INDEXED_PRIM_LIST",
         "INDEXED_INSTANCED_PRIM_LIST")
PROLOGUE_END = "START_TILE_BINNING"


def jobs(path):
    out, cur = [], None
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            if raw.startswith("MESA: error"):
                continue
            line = re.sub(r"^MESA: info: ?", "", raw.rstrip("\n"))
            if line.startswith("@createbuf") and cur is None:
                cur = []
            if cur is not None:
                cur.append(line)
            if line.startswith("@wait_render"):
                out.append(cur)
                cur = None
    return out


class Pk:
    def __init__(self, name):
        self.name = name
        self.fields = []

    def vals(self):
        return [(k, v) for k, v in self.fields if not v.startswith("[")]


def parse(job):
    """(BCL packets, RCL packets, tile list packets, records, submit args)"""
    lists, recs, sub = {}, [], {}
    buf, kind, pk, cur = None, "", None, None
    for line in job:
        m = re.match(r"^@buffer (\S+)", line)
        if m:
            buf, kind, pk, cur = m.group(1), "", None, None
            continue
        m = re.match(r"^@format (\S+)", line)
        if m:
            kind, pk = m.group(1), None
            cur = lists.setdefault(buf, []) if kind == "ctrllist" else None
            continue
        m = re.match(r"^@(add_bin|add_render)", line)
        if m:
            kind, pk, cur = m.group(1), None, None
            sub[kind] = []
            continue
        if kind in ("add_bin", "add_render") and line.strip() and not line.startswith("@"):
            sub[kind].append(line.strip().split(" /*")[0])
            continue
        if line.startswith("@"):
            kind, pk, cur = "", None, None
            continue
        m = FIELD.match(line)
        if kind.startswith("shadrec"):
            if m:
                if pk is None:
                    pk = Pk(kind.upper())
                    recs.append(pk)
                pk.fields.append((m.group(1).strip(), m.group(2).split(" /*")[0].strip()))
        elif cur is not None:
            if PACKET.match(line):
                pk = Pk(line)
                cur.append(pk)
            elif pk is not None and m:
                pk.fields.append((m.group(1).strip(), m.group(2).split(" /*")[0].strip()))

    def bo(a):
        return re.match(r"\[([^+\]]+)", a).group(1)

    bcl, rcl = bo(sub["add_bin"][0]), bo(sub["add_render"][0])
    tiles = [p for b, ps in lists.items() if b not in (bcl, rcl) for p in ps]
    args = [a for a in sub["add_bin"] + sub["add_render"] if not a.startswith("[")]
    return lists.get(bcl, []), lists.get(rcl, []), tiles, recs, args


def key(p):
    """a state packet's slot: STENCIL_CFG per face, BLEND_CFG per RT mask"""
    f = dict(p.fields)
    if p.name == "STENCIL_CFG":
        return "STENCIL_CFG[front %s back %s]" % (f.get("Front Config"), f.get("Back Config"))
    if p.name == "BLEND_CFG":
        return "BLEND_CFG[%s]" % f.get("Render Target Mask")
    if p.name.endswith("_FLAGS") and not p.name.startswith("ZERO_ALL"):
        return "%s[%s]" % (p.name, p.fields[0][1] if p.fields else "")
    return p.name


def fld_cmp(where, p, q, diffs):
    fa, fb = dict(p.vals()), dict(q.vals())
    for k in [k for k, _ in p.vals()] + [k for k, _ in q.vals() if k not in fa]:
        if fa.get(k) != fb.get(k):
            diffs.append("%s.%s: %s != %s" % (where, k, fa.get(k, "<none>"), fb.get(k, "<none>")))


def seq_cmp(where, a, b, diffs):
    sm = difflib.SequenceMatcher(None, [p.name for p in a], [p.name for p in b], autojunk=False)
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        if op == "equal":
            for k in range(i2 - i1):
                fld_cmp("%s #%d %s" % (where, i1 + k, a[i1 + k].name), a[i1 + k], b[j1 + k], diffs)
            continue
        for i in range(i1, i2):
            diffs.append("%s #%d %s: present != absent" % (where, i, a[i].name))
        for j in range(j1, j2):
            diffs.append("%s #%d %s: absent != present" % (where, j, b[j].name))


def bcl_split(ps):
    """(prologue, [(state, draw packet)], epilogue, state packets repeated)"""
    pro, draws, state, tail, rep = [], [], {}, [], 0
    i = 0
    while i < len(ps):
        pro.append(ps[i])
        i += 1
        if ps[i - 1].name == PROLOGUE_END:
            break
    for p in ps[i:]:
        if p.name in DRAWS:
            draws.append((dict(state), p))
            tail = []
            continue
        tail.append(p)
        k = key(p)
        if k in state and state[k].vals() == p.vals():
            rep += 1
        state[k] = p
    return pro, draws, tail, rep


def bcl_cmp(where, a, b, diffs):
    pa, da, ta, rep = bcl_split(a)
    pb, db, tb, _ = bcl_split(b)
    seq_cmp(where + " prologue", pa, pb, diffs)
    if len(da) != len(db):
        diffs.append("%s draws: %d != %d" % (where, len(da), len(db)))
    for n, ((sa, xa), (sb, xb)) in enumerate(zip(da, db)):
        w = "%s draw %d" % (where, n)
        if xa.name != xb.name:
            diffs.append("%s: %s != %s" % (w, xa.name, xb.name))
        else:
            fld_cmp("%s %s" % (w, xa.name), xa, xb, diffs)
        for k in list(sa) + [k for k in sb if k not in sa]:
            if k not in sb:
                diffs.append("%s %s: present != absent" % (w, k))
            elif k not in sa:
                diffs.append("%s %s: absent != present" % (w, k))
            else:
                fld_cmp("%s %s" % (w, k), sa[k], sb[k], diffs)
    seq_cmp(where + " epilogue", ta, tb, diffs)
    return rep


def main():
    ours, ref, shape, allow_path = sys.argv[1:5]
    allow = []
    with open(allow_path, encoding="utf-8") as f:
        for line in f:
            if line.strip() and not line.startswith("#"):
                shapes, pat, ident, reason = line.rstrip("\n").split(" | ", 3)
                if fnmatch.fnmatchcase(shape, shapes):
                    allow.append([pat, ident, reason, 0])
    ja, jb = jobs(ours), jobs(ref)
    diffs = []
    npk = rep = 0
    if len(ja) != len(jb):
        diffs.append("%s jobs: %d != %d" % (shape, len(ja), len(jb)))
    for k, (a, b) in enumerate(zip(ja, jb)):
        ba, ra, ta, reca, arga = parse(a)
        bb, rb, tb, recb, argb = parse(b)
        npk += len(bb) + len(rb) + len(tb) + len(recb)
        w = "%s job %d" % (shape, k)
        rep += bcl_cmp(w + " bcl", ba, bb, diffs)
        seq_cmp(w + " rcl", ra, rb, diffs)
        seq_cmp(w + " tiles", ta, tb, diffs)
        seq_cmp(w + " rec", reca, recb, diffs)
        if arga != argb:
            diffs.append("%s submit: %s != %s" % (w, " ".join(arga), " ".join(argb)))
    bad, named = [], {}
    for d in diffs:
        for e in allow:
            if fnmatch.fnmatchcase(d, e[0]):
                e[3] += 1
                named.setdefault(e[1], []).append(d)
                break
        else:
            bad.append(d)
    for d in bad:
        print(d)
    stale = [e for e in allow if e[3] == 0]
    for e in stale:
        print("stale named difference %s: %s" % (e[1], e[0]))
    for ident, ds in named.items():
        for d in ds:
            print("named %s: %s" % (ident, d))
    ids = ", ".join("%s %d" % (i, len(ds)) for i, ds in named.items())
    print("gl_cmp: %s %d jobs, %d packets, %d differences, %d named%s, %d stale; %d state "
          "packets of ours repeat the state in effect" % (
              shape, len(jb), npk, len(bad), len(diffs) - len(bad), " (%s)" % ids if ids else "",
              len(stale), rep))
    return 1 if bad or stale else 0


if __name__ == "__main__":
    sys.exit(main())
EOF

# The differences v3d_job71 keeps against Mesa's jobs, each with its reason
# (none is G1's; each is the builder's choice or Mesa's blitter, and the GPU
# draws the same pixels): shape pattern | difference pattern | id | reason.
cat >"$dir/gl.allow" <<'EOF'
[!m]* | * job * rcl #0 TILE_RENDERING_MODE_CFG_COMMON.Early-Z disable: 1 != 0 | EZ | v3d_job71 never enables early Z (v3d_job71.h, refused list: "early Z (off: legal, only slower)"; the rendering mode's Early-Z disable is always set), where Mesa turns it on for a job with a Z/S buffer whose draws allow it (v3dx_rcl.c emit_rcl, job->decided_global_ez_enable)
[!m]* | * job * bcl draw * POINT_SIZE.Point Size: 1.000000 != 0.125000 | PSIZE | the draw is Mesa's clear blitter, whose rasterizer state has point size 0, written as 0.125 (v3dx_state.c, MAX2(point_size, .125f)); v3d_job71 writes gallium's ES point size 1.0 for every draw (v3d_job71.c emit_state); a TRIANGLE_FAN does not read it
[fw]* | * job * tiles #8 CLEAR_RT: present != absent | CLRT | the job loads every buffer and clears none (Mesa's job->clear_tlb 0), so Mesa writes no CLEAR_RENDER_TARGETS after the tile's stores (v3dx_rcl.c v3d_rcl_emit_stores); v3d_job71 writes it after every tile's stores (v3d_job71.c emit_tile_list), and the next tile loads over the cleared buffer
fbods | fbods job 0 rec #[4-7] SHADREC_GL_*: present != absent | REC | the job's second draw keeps the program and the vertex buffer: Mesa writes no new shader state and the draw reuses the first record (v3dx_draw.c, the shader state is emitted when dirty); v3d_job71 writes a record for every draw (its GL_SHADER packet is still equal field by field)
EOF

gl_line=""
rm -rf "$dir/gl" && mkdir -p "$dir/gl" || exit 1
out=$(TMPDIR=$dir/tmp "$dir/job71_check" gl "$dir/gl" 2>&1)
rc=$?
printf '%s\n' "$out" >"$dir/gl.out"
if [ "$rc" -ne 0 ] || ! printf '%s\n' "$out" | tail -n 1 | grep -q ": ok$"; then
  fail "(gl) job71_check gl: exit $rc"
  printf '%s\n' "$out" | sed 's/^/  /'
else
  jobs=0 packets=0 diffs=0 named=0
  for s in scissor scisbox writemask viewport fbods colormask mrt4 mrt8; do
    refclif=tools/v3d/pi5/gl_$s.clif
    # the comparison reads the reference as equal to itself
    python3 -I "$dir/gl_cmp.py" "$refclif" "$refclif" "$s" /dev/null >"$dir/gl_$s.self" 2>&1 || {
      fail "(gl) $refclif is not equal to itself:"
      sed 's/^/  /' "$dir/gl_$s.self"
    }
    python3 -I "$dir/gl_cmp.py" "$dir/gl/gl_$s.clif" "$refclif" "$s" "$dir/gl.allow" \
      >"$dir/gl_$s.cmp" 2>&1 || fail "(gl) gl_$s differs from $refclif:"
    last=$(tail -n 1 "$dir/gl_$s.cmp")
    echo "(gl) $refclif: ${last#gl_cmp: $s }"
    grep -v '^gl_cmp: ' "$dir/gl_$s.cmp" | sed 's/^/  /'
    nums=$(printf '%s\n' "$last" | sed -n 's/^gl_cmp: [a-z0-9]* \([0-9]*\) jobs, \([0-9]*\) packets, \([0-9]*\) differences, \([0-9]*\) named.*/\1 \2 \3 \4/p')
    if [ -n "$nums" ]; then
      read -r nj np nd nn <<<"$nums"
      jobs=$((jobs + nj)) packets=$((packets + np)) diffs=$((diffs + nd)) named=$((named + nn))
    else
      fail "(gl) gl_$s: no gl_cmp summary"
    fi
  done
  # a changed store is found: our writemask with its Z-only store (G1)
  # turned into a Z/S store
  sed 's|Buffer to Store: \*/ 9 /\* Z \*/|Buffer to Store: */ 11 /* Z/S */|' "$dir/gl/gl_writemask.clif" \
    >"$dir/mutated_writemask.clif"
  if python3 -I "$dir/gl_cmp.py" "$dir/mutated_writemask.clif" tools/v3d/pi5/gl_writemask.clif writemask \
    "$dir/gl.allow" >"$dir/mutated_writemask.cmp" 2>&1 ||
    ! grep -q '^writemask job 1 tiles #[0-9]* STORE.Buffer to Store: 11 != 9$' "$dir/mutated_writemask.cmp"; then
    fail "(gl) a Z/S store in place of writemask job 1's Z-only store was not found"
  fi
  for id in $(sed -n 's/^[^|]* | [^|]* | \([A-Z]*\) | .*/\1/p' "$dir/gl.allow"); do
    n=$(cat "$dir"/gl_*.cmp | grep -c "^named $id: ")
    echo "(gl) named $id ($n): $(sed -n "s/^[^|]* | [^|]* | $id | //p" "$dir/gl.allow")"
  done
  gl_line="8 references, $jobs jobs, $packets packets, $diffs differences, $named named"
fi

summary="(a) clear: ${clear_line:-not run}; (b) triangle: ${triangle_line:-not run}; (c) multi; (d) ${refused:-?} refused; (e) G1: ${g1:-not run}; (f) gl: ${gl_line:-not run}"
if [ "$fails" -ne 0 ]; then
  echo "$name: $summary, $fails failed"
  exit 1
fi
echo "$name: $summary, 0 failed"
exit 0
