#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# sched71_oracle.sh - Mesa 26.2.1's QPU scheduler at V3D 7.1, as a table.
#
# Builds tools/v3d/sched71_oracle.c against a Mesa 26.2.1 source tree and
# writes its output to tools/v3d/sched71_oracle.tsv: for a fixed set of
# instruction pairs (A, then B in program order), what Mesa's scheduler
# (src/broadcom/compiler/qpu_schedule.c, devinfo->ver = 71) makes of them.
# It is the reference sched71.rs (the 7.1 back end's packer,
# docs/notes/v3d71-backend.md) is checked against; the counterpart of
# hazards71 (qpu_validate.c's verdicts) for the scheduler.
#
# Usage:
#   tools/v3d/sched71_oracle.sh [MESA_TREE]           regenerate the .tsv
#   tools/v3d/sched71_oracle.sh --check [MESA_TREE]   compare, do not write
#
# MESA_TREE is the root of the Mesa 26.2.1 source tree (the release tarball
# of third_party/mesa-26.2.1/UPSTREAM, sha256 c47e81bd...4442, unpacked);
# without the argument, MESA_V3D_TREE (tools/v3d/README.md: a 26.2.1 tree,
# build-v3d is not needed here). Only sources are used, nothing built in the
# tree: the NIR headers Mesa generates are made with Mesa's own generators
# into build/v3d71/sched71_oracle/, the harness and Mesa's qpu/ and the part
# of util/ it needs are compiled there. The script refuses a tree whose files
# differ from 26.2.1's (the sha256 list below) and exits 77 with
# "gap: MESA_V3D_TREE" when no tree is given (a gap is not a pass). --check
# exits 1 when the committed table differs from the regenerated one.
# Regeneration is byte for byte: the harness prints nothing that depends on
# the host, the time or memory addresses.
#
# To regenerate (after a change to sched71_oracle.c, or to check the table):
#   1. get the tarball named in third_party/mesa-26.2.1/UPSTREAM, check its
#      sha256 and unpack it outside the repository (for example under the
#      dependency cache, as tools/v3d/README.md does for build-v3d);
#   2. tools/v3d/sched71_oracle.sh <that tree>   (needs cc and python3 with
#      Mesa's generator modules: mako, PyYAML);
#   3. commit tools/v3d/sched71_oracle.tsv in the same commit as the change.
#   tools/v3d/sched71_oracle.sh --check <that tree> must then print
#   "table identical".
#
# How the harness gets its verdicts. It #includes qpu_schedule.c, whose
# functions are static (Mesa is MIT; nothing of it is copied into this
# repository), and runs, per pair, what qpu_schedule_instructions_block
# (qpu_schedule.c:2873) runs on a block of the two instructions:
# calculate_forward_deps and calculate_reverse_deps (:546, :561; the rules of
# calculate_deps, :311-543, with the 7.1 raddr reads of process_raddr_deps,
# :179, and the implicit rf0 write, :479), compute_delay (:1894), then
# schedule_instructions (:2733), whose choose_instruction_to_schedule
# (:1400) applies every pairing and spacing rule: reads_too_soon (:623, rf0
# one tick after ldvary), writes_too_soon_after_write (:699, no rf0 write
# right after ldvary at 7.1), ldunif after ldvary (:1468), the thrsw delay
# slots (qpu_inst_after_thrsw_valid_in_delay_slot :2132,
# qpu_inst_before_thrsw_valid_in_delay_slot :2070, including the 7.1 rf0
# flops rule :2111 and ldvary allowed in slots 0-1, :2089), the ldvary
# pairing rule (:1548-1559, 7.1: not in the last delay slot), the TMU fifo
# rule (:1577), read_stalls (:779: reading an SFU result in the very next
# word stalls, so such a word is never paired and only chosen last, :1594), and
# qpu_merge_inst (:1236): qpu_compatible_peripheral_access (:896, the 7.1
# restricted set :937-959), the add<->mul conversions (:1109-1233, mov/fmov
# move to the add ALU only at 7.1, :1180, and the small immediate moves
# between slots a/b and c/d, :1146-1170 and :1210-1232) and qpu_merge_raddrs
# (:1008, 7.1: at most one small immediate per word, :1013-1032). A thrsw is
# placed by emit_thrsw (:2250), an ldvary paired into a word is moved up by
# fixup_pipelined_ldvary (:2613).
#
# The context of every pair (sched71_oracle.c run_pair): a fragment shader,
# 4 threads, num_inputs 0 (no ldvary pipelining, :1415), the scoreboard as
# v3d_qpu_schedule_instructions initialises it (:3033-3044) but after the
# program's last thrsw (TLB writes allowed at once, :732-742) and with one
# TMU lookup whose two results are unread (an ldtmu has a result, :1685).
# One NOP is scheduled ahead of the pair (word 0), so that
# fixup_pipelined_ldvary has a previous word (:2651); not when the pair has a
# thrsw, which emit_thrsw would move into it (:2283-2337).
#
# The instruction forms (sched71_oracle.c, enum form): add ALU fadd, add,
# sub, mov.a, fmov.a (the add-ALU mov/fmov of 7.1); the SFU as add ops
# recip, rsqrt, exp, log, sin; fadd.si_a, fadd.si_b, add.si_b (a small
# immediate in add operand a/b: 1.0, 1.0, 1); mul ALU fmul, mov.m, fmov.m,
# fmul.si_c, fmul.si_d, mov.m.si_c (small immediate in mul operand a/b);
# the signals ldunif (implicit rf0), ldunifrf, ldvary (rf and implicit rf0
# C coefficient), ldtmu, thrsw, wrtmuc, each on a NOP; the magic writes
# (a mul-ALU mov, as vir_MOV_dest emits them) tmud, tmut, tmus, tmua (tmus
# and tmua end a lookup: ldtmu_count 1), tlb, tlbu.
#
# The pair kinds (id = kind.formA.formB):
#   ind   every form, then every form (not thrsw twice); no register shared
#         (A writes rf11, reads rf1/rf2; B writes rf12, reads rf3/rf4)
#   raw   every form with a register-file destination writes rf10, then
#         every form with a register-file source reads rf10 (first operand)
#   raw0  ldunif and ldvary (implicit rf0), then every reader of rf0
#   war   every reader of rf10, then every writer of rf10
#   waw   every writer of rf10, then every writer of rf10
#
# The columns of the .tsv (tab separated, '#' lines are comments):
#   id          kind.formA.formB
#   kind        ind, raw, raw0, war, waw
#   a, b        A and B in Mesa's disassembly (v3d_qpu_disasm at ver 71 of
#               the packed word; its column padding printed as one space)
#   dep         the DAG edge A->B: order (a read-after-write, write-after-
#               write or other ordering edge), war (only a write-after-read
#               edge, removed when A is scheduled, :1915, so B may share
#               A's word), none
#   latency     instruction_latency(A, B) (:1859): 2 after an SFU op, 100
#               from a TMU write to an ldtmu, 3 after a magic SFU write, else
#               1. Mesa uses it for priorities (compute_delay) and cycle
#               estimates only, never to insert NOPs: the hard distance is
#               dist.
#   merge_inst  qpu_merge_inst(A, B) on its own: the encoding allows the
#               two in one word (peripherals, ALU slots, small immediates,
#               signals, the pack); no scheduling rule applied
#   merge       Mesa's scheduler put A and B in one word
#   dist        word of B minus word of A in Mesa's schedule: 0 merged, 1
#               the next word, n > 1 with n-1 words between (NOPs Mesa had
#               to insert), negative when B ended up ahead of A (an ldvary
#               paired with A and then moved up into the leading NOP by
#               fixup_pipelined_ldvary: the ind.*.ldvary rows at -1)
#   schedule    every word of Mesa's schedule, ' | ' between words (the
#               leading NOP included)
# The last line is "# N pairs".
set -u
cd "$(dirname "$0")/../.." || exit 1
export PYTHONDONTWRITEBYTECODE=1

check=0
if [ "${1:-}" = --check ]; then
  check=1
  shift
fi
M=${1:-${MESA_V3D_TREE:-}}
if [ -z "$M" ]; then
  echo "sched71_oracle: no Mesa 26.2.1 source tree (argument or MESA_V3D_TREE)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi
if [ "$(cat "$M/VERSION" 2>/dev/null)" != 26.2.1 ]; then
  echo "sched71_oracle: $M is not a Mesa 26.2.1 tree (VERSION)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

fail() {
  echo "sched71_oracle: $*"
  exit 1
}

# The Mesa files the table depends on, as the 26.2.1 tarball has them.
while read -r sum f; do
  [ "$(sha256sum <"$M/$f" | cut -d' ' -f1)" = "$sum" ] || fail "$M/$f is not Mesa 26.2.1's"
done <<'SUMS'
10cfaa401ad64586a3a60cc18c238f09c923ddde51a5f646c2851863cb6754f7 src/broadcom/compiler/qpu_schedule.c
ae335250bef2843f561afb61795a1e043f25372945d123e8c6a6c620f0880caf src/broadcom/compiler/v3d_compiler.h
2c631bd8a936fba5a5d607e7d526604e1691c99295d08ed0948f981e41ee85f6 src/broadcom/qpu/qpu_instr.c
125761b56819085278c6685fc8b664b944e07cf563834e18de1ccbdbedf0cd9b src/broadcom/qpu/qpu_instr.h
323295a44f055594303440afb763d72dee33c1bde177d2787ab8bc5ab3cec216 src/broadcom/qpu/qpu_pack.c
4bd788cef88b69dea6befb9fa9bad8a218e93adf486eaab7e98121f855f586ae src/broadcom/qpu/qpu_disasm.c
2ae079ee7f77ce6b178be82d57195ee2dd9fcb13add3e2b6f54cf2b7ebd4f143 src/util/dag.c
SUMS

O=build/v3d71/sched71_oracle
G=$O/gen
mkdir -p "$G/compiler/nir" "$G/util/format" "$O/obj" || exit 1

# Mesa's generated headers, by Mesa's generators (their meson.build commands).
python3 "$M/src/compiler/nir/nir_opcodes_h.py" >"$G/compiler/nir/nir_opcodes.h" ||
  fail "nir_opcodes_h.py failed"
python3 "$M/src/compiler/nir/nir_intrinsics_h.py" --out "$G/compiler/nir/nir_intrinsics.h" ||
  fail "nir_intrinsics_h.py failed"
python3 "$M/src/compiler/nir/nir_intrinsics_indices_h.py" --out "$G/compiler/nir/nir_intrinsics_indices.h" ||
  fail "nir_intrinsics_indices_h.py failed"
python3 "$M/src/util/format/u_format_table.py" "$M/src/util/format/u_format.yaml" --enums \
  >"$G/util/format/u_format_gen.h" || fail "u_format_table.py failed"
python3 "$M/src/compiler/builtin_types_h.py" "$G/compiler/builtin_types.h" ||
  fail "builtin_types_h.py failed"

# Mesa's flags (its meson files); Mesa's warnings are Mesa's business.
FLAGS=(-std=gnu11 -O1 -g0 -w -D_GNU_SOURCE -DHAVE_ENDIAN_H -DHAVE_PTHREAD
  -DHAVE_STRUCT_TIMESPEC -DHAVE_SYSCONF -DHAVE_TIMESPEC_GET
  -I"$G" -I"$G/compiler" -I"$G/compiler/nir" -I"$M/include" -I"$M/src"
  -I"$M/src/broadcom" -I"$M/src/broadcom/compiler" -I"$M/src/compiler"
  -I"$M/src/compiler/nir")

objs=()
build_one() { # build_one <source> <object name>
  local obj=$O/obj/$2.o
  cc -c "${FLAGS[@]}" "$1" -o "$obj" >"$obj.log" 2>&1 || {
    cat "$obj.log"
    fail "$1 did not compile"
  }
  objs+=("$obj")
}

build_one tools/v3d/sched71_oracle.c sched71_oracle
for f in src/broadcom/qpu/qpu_instr.c src/broadcom/qpu/qpu_pack.c \
  src/broadcom/qpu/qpu_disasm.c src/util/ralloc.c src/util/dag.c \
  src/util/set.c src/util/hash_table.c src/util/u_dynarray.c src/util/u_printf.c \
  src/util/log.c src/util/os_misc.c src/util/u_debug.c src/util/u_process.c \
  src/util/simple_mtx.c src/util/u_call_once.c src/util/os_file.c src/util/blob.c \
  src/util/half_float.c src/util/u_cpu_detect.c src/util/softfloat.c src/util/u_thread.c; do
  build_one "$M/$f" "$(basename "$f" .c)"
done
cc "${objs[@]}" -lm -lpthread -o "$O/sched71_oracle" >"$O/link.log" 2>&1 || {
  cat "$O/link.log"
  fail "the harness did not link"
}

"$O/sched71_oracle" >"$O/sched71_oracle.tsv" || fail "the harness failed"
rows=$(grep -vc '^#' "$O/sched71_oracle.tsv")
if [ "$check" = 1 ]; then
  cmp -s "$O/sched71_oracle.tsv" tools/v3d/sched71_oracle.tsv ||
    fail "tools/v3d/sched71_oracle.tsv differs from Mesa's output ($O/sched71_oracle.tsv)"
  echo "sched71_oracle: $((rows - 1)) pairs, table identical, 0 failed"
else
  cp "$O/sched71_oracle.tsv" tools/v3d/sched71_oracle.tsv || exit 1
  echo "sched71_oracle: $((rows - 1)) pairs written to tools/v3d/sched71_oracle.tsv"
fi
