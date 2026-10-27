#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tools/cts/gles3-mustpass.txt from a VK-GL-CTS checkout: the union of its
# android/cts/main/gles3-main-*.txt (the yearly lists are increments), sorted
# (LC_ALL=C) and without duplicates -- the list host-deqp-run.sh MODULE=gles3
# runs, as a file the Zephyr tally (deqp-uart-tally.py --mustpass) can read.
#
#   CTS=<VK-GL-CTS checkout> tools/cts/gen-gles3-mustpass.sh
#
# At VK-GL-CTS 93bca01 it holds 42,810 cases; tools/check.sh pins that count,
# so a regenerated list of another size fails there until the pin is
# re-derived on purpose.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
CTS=${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}
ls "$CTS"/android/cts/main/gles3-main-*.txt >/dev/null
cat "$CTS"/android/cts/main/gles3-main-*.txt | tr -d '\r' | grep -v '^$' | LC_ALL=C sort -u > "$HERE/gles3-mustpass.txt"
echo "gen-gles3-mustpass: $(wc -l < "$HERE/gles3-mustpass.txt") cases from $(git -C "$CTS" rev-parse --short HEAD)"
