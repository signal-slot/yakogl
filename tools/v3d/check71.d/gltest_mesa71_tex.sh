#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gltest_mesa71_tex: tools/v3d/gltest-mesa71-tex/ (Mesa's offline V3D 7.1 programs for
# the gl_test texture, mipmap and cube cases: 2 and 41; README.md there).
#
# Always, against the vendored Mesa reference build (tools/v3d/mesa_ref.sh) or
# MESA_V3D_TREE's build-v3d, as the other 7.1 checks:
#  - every committed header compiles as C11 with -Wall -Wextra -Werror;
#  - check71.c packs each header's record fields with Mesa's own 7.1 packer
#    (GL Shader State Record Draw Index) and gets the header's 32 bytes and
#    flag word, and disassembles the header's code words with Mesa's own 7.1
#    QPU disassembler: the lines equal <name>.disasm (statistics lines aside);
#  - gen.py --recheck writes each header again from its committed JSON (the
#    record71 values read back from the header): it equals the committed
#    header byte for byte, and the record values that copy prog_data equal it;
#  - every committed .json, .disasm and .h belongs to a program of gen.py.
# With Mesa's compiler (v3d_shc at 7.1.10.2: V3D_SHC naming the binary, or
# MESA_V3D_TREE with build-v3d/src/broadcom/v3d_shc built; tools/v3d/README.md,
# "Set up once"), also the regeneration: gen.py compiles every program again,
# every file equals the committed one. record71.c is the one of
# tools/v3d/gltest-mesa71 (called, not copied), as is its gen.py code.
# The vendored reference has no compiler (third_party/mesa-26.2.1 has no
# src/compiler), so without one a line says the regeneration was not run. That
# is not a gap: the committed files are still checked against Mesa's packer
# and disassembler (as tfu71.sh without its Mesa tree). MESA_V3D_TREE without
# build-v3d, or a V3D_SHC that is not an executable inside a Mesa build, is a
# gap naming it.
#
#   tools/v3d/check71.d/gltest_mesa71.sh            check
#   tools/v3d/check71.d/gltest_mesa71.sh --write    regenerate the committed files (needs v3d_shc)
# Convention: tools/v3d/check71.d/README.md.
# Run as `sh <this>` too: a POSIX shell hands it to bash.
[ -n "${BASH_VERSION-}" ] || exec bash "$0" "$@"
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=gltest_mesa71_tex
dir=build/v3d71/$name
committed=tools/v3d/gltest-mesa71-tex
shared=tools/v3d/gltest-mesa71
write=0
[ "${1-}" = --write ] && write=1
rm -rf "$dir" || exit 1
mkdir -p "$dir/out" "$dir/work" "$dir/recheck" "$dir/code" || exit 1
fail=0

cc=${CC:-cc}
cxx=${CXX:-c++}
tree=${MESA_V3D_TREE:-}
shc=${V3D_SHC:-}

# Mesa's compiler when there is one: V3D_SHC, else MESA_V3D_TREE's build.
if [ -n "$tree" ] && [ ! -d "$tree/build-v3d" ]; then
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi
if [ -z "$shc" ] && [ -n "$tree" ] && [ -x "$tree/build-v3d/src/broadcom/v3d_shc" ]; then
  shc=$tree/build-v3d/src/broadcom/v3d_shc
fi
if [ -n "$shc" ] && [ ! -x "$shc" ]; then
  echo "$name: V3D_SHC='$shc' is not an executable v3d_shc (ninja -C <build> src/broadcom/v3d_shc)"
  echo "gap: V3D_SHC"
  exit 77
fi
if [ "$write" -eq 1 ] && [ -z "$shc" ]; then
  echo "$name: --write needs Mesa's v3d_shc: set V3D_SHC or MESA_V3D_TREE (tools/v3d/README.md, 'Set up once')"
  exit 2
fi

programs=0
if [ "$write" -eq 0 ]; then
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

  # Mesa's 7.1 packer and QPU disassembler on the committed files (check71.c).
  mt=third_party/mesa-26.2.1
  inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H -I"$committed")
  if [ -z "$tree" ]; then
    tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
      tail -20 "$dir/mesa_ref.log"
      echo "$name: tools/v3d/mesa_ref.sh failed"
      exit 1
    }
    ref=build/v3d-check/mesa
    inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
    libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
    against="the vendored reference build"
  else
    tb=$tree/build-v3d
    inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$tb/src/broadcom" -I"$tb/src")
    libs=("$tb/src/broadcom/libbroadcom_v3d.a" "$tb/src/broadcom/cle/libbroadcom_cle.a"
      "$tb/src/broadcom/libv3d_neon.a" "$tb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
    against="MESA_V3D_TREE's build-v3d"
  fi
  # -Wall only: Mesa's generated packers leave `data` unused (as mesa71_clear.sh).
  if ! "$cc" -O1 -std=gnu11 -Wall -Werror "${inc[@]}" "$committed/check71.c" "${libs[@]}" \
    -o "$dir/check71" >"$dir/check71.log" 2>&1; then
    tail -30 "$dir/check71.log"
    echo "$name: check71.c did not build against $against"
    exit 1
  fi
  if "$dir/check71" "$dir/code" >"$dir/check71.out" 2>&1; then
    sed "s/^/$name: /" "$dir/check71.out"
  else
    cat "$dir/check71.out"
    fail=$((fail + 1))
  fi
  for j in "$committed"/*.json; do
    [ -e "$j" ] || continue
    b=$(basename "$j" .json)
    programs=$((programs + 1))
    if [ ! -e "$dir/code/$b.code" ]; then
      echo "$name: $j is committed but check71.c does not check it (name it in check71.c and gen.py)"
      fail=$((fail + 1))
    elif grep -v '^v3d: ' "$committed/$b.disasm" | cmp -s - "$dir/code/$b.code"; then
      echo "$name: $b.disasm equals Mesa's 7.1 disassembly of $b.h's words"
    else
      grep -v '^v3d: ' "$committed/$b.disasm" | diff -u - "$dir/code/$b.code" | head -20
      echo "$name: $b.disasm differs from Mesa's 7.1 disassembly of the committed words"
      fail=$((fail + 1))
    fi
  done
  for c in "$dir"/code/*.code; do
    [ -e "$c" ] || continue
    b=$(basename "$c" .code)
    [ -e "$committed/$b.json" ] || {
      echo "$name: check71.c checks $b, which has no $committed/$b.json"
      fail=$((fail + 1))
    }
  done

  # The headers again from their JSON (gen.py --recheck), byte for byte.
  if python3 -I "$committed/gen.py" --recheck "$committed" --out-dir "$dir/recheck" >"$dir/recheck.log" 2>&1; then
    sed 's/^/  /' "$dir/recheck.log"
  else
    cat "$dir/recheck.log"
    echo "$name: gen.py --recheck failed"
    fail=$((fail + 1))
  fi
  for f in "$committed"/*.json "$committed"/*.disasm "$committed"/*.h; do
    [ -e "$f" ] || continue
    b=$(basename "$f")
    stem=${b%.*}
    if [ ! -e "$dir/recheck/$stem.h" ]; then
      echo "$name: $f is committed but no program of gen.py makes it"
      fail=$((fail + 1))
    elif [ "${b##*.}" = h ]; then
      if cmp -s "$f" "$dir/recheck/$b"; then
        echo "$name: $f equals the header gen.py writes from $stem.json"
      else
        diff -u "$f" "$dir/recheck/$b" | head -20
        echo "$name: $f differs from the header gen.py writes from $stem.json"
        fail=$((fail + 1))
      fi
    fi
  done

  if [ -z "$shc" ]; then
    echo "$name: no v3d_shc (V3D_SHC, or MESA_V3D_TREE with v3d_shc built): the regeneration was not run"
    echo "$name: $programs programs checked against Mesa's 7.1 packer and disassembler ($against), regeneration not run, $fail failed"
    [ "$fail" -eq 0 ]
    exit
  fi
fi

# The regeneration, with Mesa's compiler.
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
if ! python3 -I - "$mb/compile_commands.json" "$PWD/$shared/record71.c" "$PWD/$dir/record71.o" \
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
echo "$name: $programs programs compiled by Mesa at 7.1.10.2 and checked against its 7.1 packer and disassembler, $fail failed"
[ "$fail" -eq 0 ]
