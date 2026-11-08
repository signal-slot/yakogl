#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gen71: driver/src/v3d/generated/v3d_packet_v71_pack.h is, byte for byte,
# what Mesa's own generator makes for V3D 7.1 from the vendored packet
# description, with the command of Mesa's src/broadcom/cle/meson.build:
#
#   python3 third_party/mesa-26.2.1/src/broadcom/cle/gen_pack_header.py \
#       third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml 71
#
# The 4.2 header has the same check in tools/check.sh's "gen
# driver/src/v3d/generated" row. The file is never edited by hand; an update
# of Mesa regenerates it. No Mesa build is needed (MESA_V3D_TREE is not read):
# the generator and the XML are vendored. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=gen71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

m=third_party/mesa-26.2.1/src/broadcom/cle
g=driver/src/v3d/generated/v3d_packet_v71_pack.h
command -v python3 >/dev/null 2>&1 || {
  echo "gen71: python3 is not installed; Mesa's generator is a Python script"
  exit 1
}
# Mesa's generator imports nothing of its own, but no .pyc into the vendored tree.
PYTHONDONTWRITEBYTECODE=1 python3 "$m/gen_pack_header.py" "$m/v3d_packet.xml" 71 \
  >"$dir/v3d_packet_v71_pack.h" 2>"$dir/gen.log" || {
  cat "$dir/gen.log"
  echo "gen71: gen_pack_header.py failed"
  exit 1
}
if cmp -s "$dir/v3d_packet_v71_pack.h" "$g"; then
  echo "gen71: $g equals gen_pack_header.py's output for 71 ($(wc -l <"$g") lines), 0 failed"
  exit 0
fi
diff -u "$g" "$dir/v3d_packet_v71_pack.h" | head -40
echo "gen71: $g differs from gen_pack_header.py's output for 71 (regenerate it, never edit it), 1 failed"
exit 1
