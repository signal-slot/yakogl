#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build, from the Mesa 26.2.1 sources vendored unmodified in
# third_party/mesa-26.2.1, what tools/v3d/{tex_check,cl_golden,job_check}
# compare driver/src/v3d against, so that no Mesa tree has to exist:
#
#   build/v3d-check/mesa/gen/cle/v3d_packet_v{21,42,71}_pack.h
#       Mesa's packers (src/broadcom/cle/meson.build's command)
#   build/v3d-check/mesa/gen/v3d_xml.h
#       the zipped packet XMLs v3d_decoder.c reads (same file's command)
#   build/v3d-check/mesa/libmesa_ref.a
#       what Mesa builds as libbroadcom_cle, libbroadcom_v3d (the CLIF
#       decoder, the device info, the QPU disassembler) and the part of
#       libmesa_util those need (ralloc, hash table, log, ...), each source
#       compiled as it is
#   build/v3d-check/mesa/fmt.o, fmt_blocks.o
#       src/gallium/drivers/v3d/v3dx_format_table.c (V3D_VERSION=42) and the
#       block sizes of Mesa's format table (gen_fmt_blocks.py); the one
#       header they take from the gallium driver, v3d_context.h (the whole
#       driver: NIR, the pipe context), is tools/v3d/mesa-stub/v3d_context.h
#
# tools/check.sh and tex_check.sh run this; so can a person, from anywhere.
# Exit status 0 when everything built; 1 otherwise (the log is printed).
set -u
# Mesa's generators import their siblings: no .pyc into the vendored tree.
export PYTHONDONTWRITEBYTECODE=1
cd "$(dirname "$0")/../.." || exit 1

T=third_party/mesa-26.2.1
O=build/v3d-check/mesa
G=$O/gen
mkdir -p "$G/cle" "$O/obj" || exit 1

fail() {
  echo "mesa_ref.sh: $*"
  exit 1
}

# The generated headers, with Mesa's own generators.
for v in 42 71; do
  python3 "$T/src/broadcom/cle/gen_pack_header.py" "$T/src/broadcom/cle/v3d_packet.xml" "$v" \
    >"$G/cle/v3d_packet_v${v}_pack.h" || fail "gen_pack_header.py $v failed"
done
python3 "$T/src/broadcom/cle/gen_pack_header.py" "$T/src/broadcom/cle/vc4_packet.xml" 21 \
  >"$G/cle/v3d_packet_v21_pack.h" || fail "gen_pack_header.py 21 failed"
python3 "$T/src/util/gen_zipped_xml_file.py" "$T/src/broadcom/cle/vc4_packet.xml" \
  "$T/src/broadcom/cle/v3d_packet.xml" >"$G/v3d_xml.h" || fail "gen_zipped_xml_file.py failed"
python3 tools/v3d/gen_fmt_blocks.py "$T" >"$G/fmt_blocks.c" || fail "gen_fmt_blocks.py failed"

# Mesa's flags (its meson files); warnings are Mesa's own business here.
FLAGS=(-std=gnu11 -O1 -w -D_GNU_SOURCE -DHAVE_ENDIAN_H -DHAVE_PTHREAD -DHAVE_STRUCT_TIMESPEC
  -DHAVE_SYSCONF -DHAVE_TIMESPEC_GET -DWITH_LIBEXPAT=1
  -Itools/v3d/mesa-stub -I"$G" -Idriver/src/v3d/generated
  -I"$T/include" -I"$T/src" -I"$T/src/broadcom" -I"$T/src/broadcom/cle" -I"$T/src/broadcom/clif"
  -I"$T/src/gallium/include" -I"$T/src/gallium/drivers/v3d")

objs=()
build_one() { # build_one <source> <object name> [extra cc flags]
  local src=$1 obj=$O/obj/$2.o
  shift 2
  cc -c "${FLAGS[@]}" "$@" "$src" -o "$obj" >"$obj.log" 2>&1 || {
    cat "$obj.log"
    fail "$src did not compile"
  }
  objs+=("$obj")
}

for f in src/broadcom/clif/clif_dump.c src/broadcom/cle/v3d_decoder.c \
  src/broadcom/common/v3d_device_info.c src/broadcom/common/v3d_util.c \
  src/broadcom/common/v3d_submit_util.c src/broadcom/qpu/qpu_disasm.c \
  src/broadcom/qpu/qpu_pack.c src/broadcom/qpu/qpu_instr.c \
  src/util/ralloc.c src/util/log.c src/util/os_misc.c src/util/u_debug.c \
  src/util/u_process.c src/util/hash_table.c src/util/simple_mtx.c \
  src/util/u_call_once.c src/util/os_file.c src/util/u_printf.c src/util/blob.c \
  src/util/half_float.c src/util/u_cpu_detect.c src/util/softfloat.c src/util/u_thread.c; do
  build_one "$T/$f" "$(basename "$f" .c)"
done
for v in 42 71; do
  build_one "$T/src/broadcom/clif/v3dx_dump.c" "v3dx_dump_$v" -DV3D_VERSION=$v
done

rm -f "$O/libmesa_ref.a"
ar rcs "$O/libmesa_ref.a" "${objs[@]}" || fail "ar failed"

# The format table and the block sizes, kept out of the archive: only
# tex_check wants them.
cc -c "${FLAGS[@]}" -DV3D_VERSION=42 "$T/src/gallium/drivers/v3d/v3dx_format_table.c" \
  -o "$O/fmt.o" >"$O/fmt.log" 2>&1 || {
  cat "$O/fmt.log"
  fail "v3dx_format_table.c did not compile"
}
cc -c "${FLAGS[@]}" "$G/fmt_blocks.c" -o "$O/fmt_blocks.o" >"$O/fmt_blocks.log" 2>&1 || {
  cat "$O/fmt_blocks.log"
  fail "fmt_blocks.c did not compile"
}
