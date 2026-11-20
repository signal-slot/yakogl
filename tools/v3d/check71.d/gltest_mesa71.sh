#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gltest_mesa71: tools/v3d/gltest-mesa71/ (Mesa's offline V3D 7.1 programs for
# gl_test cases 5, 58, 59, 66, 67 and 102 and the back end's clear quad) is,
# byte for byte, what a fresh run gives: tools/v3d/gltest-mesa71/gen.py
# compiles each program with Mesa's v3d compiler (v3d_shc at 7.1.10.2) and
# writes <name>.json, .disasm and .h, the record values coming from
# record71.c (Mesa's v3d_compute_vpm_config and 7.1 draw-index packer, linked
# from the same Mesa build). The control program (passthrough + interp_color)
# must give the words, uniform streams, attributes and record that
# tools/v3d/tri-shaders71 read off the Pi 5's own Mesa dump. Every committed
# header compiles as C11 with -Wall -Wextra -Werror.
#
# Mesa's compiler is not in the vendored reference (third_party/mesa-26.2.1 has
# no src/compiler), so this needs a Mesa 26.2.1 tree with v3d_shc built
# (tools/v3d/README.md, "Set up once"): V3D_SHC naming the binary (its build
# directory is the one two levels up, the tree one more), else MESA_V3D_TREE
# with build-v3d/src/broadcom/v3d_shc. Without either it is a gap naming them.
#
#   tools/v3d/check71.d/gltest_mesa71.sh            check
#   tools/v3d/check71.d/gltest_mesa71.sh --write    regenerate the committed files
# Convention: tools/v3d/check71.d/README.md.
# Run as `sh <this>` too: a POSIX shell hands it to bash.
[ -n "${BASH_VERSION-}" ] || exec bash "$0" "$@"
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=gltest_mesa71
dir=build/v3d71/$name
committed=tools/v3d/gltest-mesa71
write=0
[ "${1-}" = --write ] && write=1
rm -rf "$dir" || exit 1
mkdir -p "$dir/out" "$dir/work" || exit 1
fail=0

cc=${CC:-cc}
cxx=${CXX:-c++}

# The committed headers compile, whatever else is available.
for h in "$committed"/*.h; do
  [ -e "$h" ] || continue
  printf '#include "%s"\nint main(void) { return 0; }\n' "$(basename "$h")" >"$dir/hdr.c"
  if "$cc" -std=c11 -Wall -Wextra -Werror -I"$committed" -c -o "$dir/hdr.o" "$dir/hdr.c" 2>"$dir/cc.log"; then
    echo "$name: $h compiles as C11, -Wall -Wextra -Werror"
  else
    cat "$dir/cc.log"
    echo "$name: $h does not compile"
    fail=$((fail + 1))
  fi
done

# Mesa's compiler: V3D_SHC, else MESA_V3D_TREE's build-v3d.
shc=${V3D_SHC:-}
tree=${MESA_V3D_TREE:-}
if [ -z "$shc" ] && [ -n "$tree" ]; then
  if [ ! -d "$tree/build-v3d" ]; then
    echo "MESA_V3D_TREE='$tree' has no build-v3d (tools/v3d/README.md, 'Set up once')"
    echo "gap: MESA_V3D_TREE"
    exit 77
  fi
  shc=$tree/build-v3d/src/broadcom/v3d_shc
fi
if [ -z "$shc" ]; then
  echo "$name: Mesa's v3d compiler is not in the vendored reference build; set V3D_SHC or MESA_V3D_TREE (tools/v3d/README.md, 'Set up once')"
  echo "gap: V3D_SHC MESA_V3D_TREE"
  exit 77
fi
if [ ! -x "$shc" ]; then
  echo "$name: '$shc' is not an executable v3d_shc (ninja -C <build> src/broadcom/v3d_shc)"
  echo "gap: V3D_SHC"
  exit 77
fi
mb=$(cd "$(dirname "$shc")/../.." && pwd) || exit 1
ms=$(cd "$mb/.." && pwd) || exit 1
for need in "$ms/src/broadcom/compiler/v3d_compiler.h" "$mb/src/broadcom/cle/v3d_packet_v71_pack.h" \
  "$mb/src/broadcom/libbroadcom_v3d.a" "$mb/compile_commands.json"; do
  [ -e "$need" ] || {
    echo "$name: $need is missing: '$shc' is not inside a Mesa tree's build directory"
    echo "gap: V3D_SHC MESA_V3D_TREE"
    exit 77
  }
done
for t in python3 glslangValidator "$cxx"; do
  command -v "$t" >/dev/null 2>&1 || {
    echo "$name: $t is not installed"
    exit 1
  }
done

# record71, linked like v3d_shc (its meson link line, the libraries in a group).
libs=()
for l in src/broadcom/libbroadcom_v3d.a src/broadcom/libv3d_neon.a src/util/libmesa_util.a \
  src/util/libmesa_util_clflush.a src/util/libmesa_util_clflushopt.a src/util/libmesa_util_simd.a \
  src/util/blake3/libblake3.a src/c11/impl/libmesa_util_c11.a src/compiler/nir/libnir.a \
  src/compiler/libcompiler.a; do
  [ -e "$mb/$l" ] && libs+=("$mb/$l")
done
# record71.c is compiled with v3d_shc.c's own command (the build's
# compile_commands.json): Mesa's include paths and defines, whatever the tree.
if ! python3 -I - "$mb/compile_commands.json" "$PWD/$committed/record71.c" "$PWD/$dir/record71.o" \
  >"$dir/build.log" 2>&1 <<'PY' ||
import json, shlex, subprocess, sys
db, src, obj = sys.argv[1:]
cmd = [e for e in json.load(open(db)) if e["file"].endswith("src/broadcom/v3d_shc.c")]
if len(cmd) != 1:
    sys.exit("no single compile command for v3d_shc.c in " + db)
a = shlex.split(cmd[0]["command"])
if a[0].endswith("ccache"):
    a = a[1:]
out, skip = [], 0
for x in a:
    if skip:
        skip -= 1
    elif x in ("-MQ", "-MF", "-o", "-c"):
        skip = 1
    elif x != "-MD":
        out.append(x)
sys.exit(subprocess.run(out + ["-c", src, "-o", obj], cwd=cmd[0]["directory"]).returncode)
PY
  ! "$cxx" -o "$dir/record71" "$dir/record71.o" -Wl,--start-group "${libs[@]}" -Wl,--end-group \
    -lz -lzstd -lm -pthread >>"$dir/build.log" 2>&1; then
  tail -20 "$dir/build.log"
  echo "$name: record71 did not build against $mb"
  exit 1
fi

out=$dir/out
[ "$write" -eq 1 ] && out=$committed
if ! python3 -I "$committed/gen.py" --shc "$shc" --record71 "$dir/record71" --mesa-src "$ms" \
  --out-dir "$out" --work "$dir/work" >"$dir/gen.log" 2>&1; then
  cat "$dir/gen.log"
  echo "$name: gen.py failed"
  exit 1
fi
sed 's/^/  /' "$dir/gen.log"
if [ "$write" -eq 1 ]; then
  echo "$name: wrote $committed; run again without --write to check"
  exit 0
fi

# Byte for byte, both ways (no committed program the run does not make).
programs=0
for f in "$dir"/out/*; do
  b=$(basename "$f")
  if cmp -s "$f" "$committed/$b"; then
    echo "$name: $committed/$b equals a fresh run"
  else
    [ -e "$committed/$b" ] && diff -u "$committed/$b" "$f" | head -20
    echo "$name: $committed/$b differs from a fresh run (tools/v3d/check71.d/$name.sh --write)"
    fail=$((fail + 1))
  fi
  case $b in *.json) programs=$((programs + 1)) ;; esac
done
for f in "$committed"/*.json "$committed"/*.disasm "$committed"/*.h; do
  [ -e "$f" ] || continue
  [ -e "$dir/out/$(basename "$f")" ] || {
    echo "$name: $f is committed but no program makes it"
    fail=$((fail + 1))
  }
done

# The control against the board's Mesa (tri-shaders71, read off the Pi 5's dump).
if python3 -I - "$dir/work/passthrough_interp_color.json" "$dir/work/passthrough_interp_color.record" \
  tools/v3d/tri-shaders71/mesa_passthrough_interp_color.json \
  tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h >"$dir/control.log" 2>&1 <<'PY'; then
import json, re, sys
ours = json.load(open(sys.argv[1]))
rec = dict((k, int(v)) for k, v in (l.split() for l in open(sys.argv[2])))
board = json.load(open(sys.argv[3]))
hdr = dict(re.findall(r"#define MESA71_TRI_(\w+) (0x[0-9a-f]+|\d+)u?", open(sys.argv[4]).read()))
bad = []
for st in ("fs", "vs", "cs"):
    for k, v in board[st].items():
        if ours[st].get(k) != v:
            bad.append("%s.%s: %r, the board's %r" % (st, k, ours[st].get(k), v))
if ours["attributes"] != board["attributes"]:
    bad.append("attributes differ")
# tri-shaders71's record names -> record71's fields
names = {
    "REC_POINT_SIZE": "point_size_in_shaded_vertex_data", "REC_ENABLE_CLIPPING": "enable_clipping",
    "REC_FS_Z_WRITES": "fragment_shader_does_z_writes", "REC_EARLY_Z_OFF": "turn_off_early_z_test",
    "REC_FS_USES_REAL_CENTRE_W": "fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2",
    "REC_SAMPLE_RATE_SHADING": "enable_sample_rate_shading",
    "REC_INSERT_PRIM_ID": "insert_primitive_id_as_first_varying_to_fragment_shader",
    "REC_SCOREBOARD_OFF": "turn_off_scoreboard",
    "REC_SCOREBOARD_WAIT_FIRST_THRSW": "do_scoreboard_wait_on_first_thread_switch",
    "REC_DISABLE_IMPLICIT_PL_VARYINGS": "disable_implicit_point_line_varyings",
    "REC_NO_PRIM_PACK": "no_prim_pack", "REC_VARYINGS": "number_of_varyings_in_fragment_shader",
    "REC_CS_OUT_VPM": "coordinate_shader_output_vpm_segment_size",
    "REC_CS_MIN_OUT": "min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size",
    "REC_CS_IN_VPM": "coordinate_shader_input_vpm_segment_size",
    "REC_CS_MIN_IN": "min_coord_shader_input_segments_required_in_play",
    "REC_VS_OUT_VPM": "vertex_shader_output_vpm_segment_size",
    "REC_VS_MIN_OUT": "min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size",
    "REC_VS_IN_VPM": "vertex_shader_input_vpm_segment_size",
    "REC_VS_MIN_IN": "min_vertex_shader_input_segments_required_in_play",
    "FS_FOUR_THREADS": "fragment_shader_4_way_threadable",
    "FS_FINAL_THREAD_SECTION": "fragment_shader_start_in_final_thread_section",
    "FS_PROPAGATE_NANS": "fragment_shader_propagate_nans",
    "VS_FOUR_THREADS": "vertex_shader_4_way_threadable",
    "VS_FINAL_THREAD_SECTION": "vertex_shader_start_in_final_thread_section",
    "VS_PROPAGATE_NANS": "vertex_shader_propagate_nans",
    "CS_FOUR_THREADS": "coordinate_shader_4_way_threadable",
    "CS_FINAL_THREAD_SECTION": "coordinate_shader_start_in_final_thread_section",
    "CS_PROPAGATE_NANS": "coordinate_shader_propagate_nans",
    "VCM_CACHE_BATCHES": "vpm_Vc", "REC_FLAGS": "flags24",
}
# tri-shaders71's REC_FLAGS is the 24-bit flag word of the first dword; its
# byte 3 is the varyings count (v3d_packet.xml:1343, start="3b"), REC_VARYINGS
rec["flags24"] = rec["word0"] & 0xffffff
for h, r in names.items():
    if h not in hdr:
        bad.append("tri-shaders71 has no MESA71_TRI_%s" % h)
    elif int(hdr[h], 0) != rec[r]:
        bad.append("%s: record71 %#x, the board's %s" % (h, rec[r], hdr[h]))
for b in bad:
    print("FAIL: control: " + b)
if bad:
    sys.exit(1)
print("control passthrough_interp_color: words, uniforms, prog_data, attributes and %d record "
      "values equal tri-shaders71 (the board's Mesa)" % len(names))
PY
  echo "$name: $(cat "$dir/control.log")"
else
  cat "$dir/control.log"
  fail=$((fail + 1))
fi

echo "$name: $programs programs compiled by Mesa at 7.1.10.2 and the control, $fail failed"
[ "$fail" -eq 0 ]
