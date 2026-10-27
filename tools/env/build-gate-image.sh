#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build-gate-image.sh -- make a machine into the gate's image, from this
# repository's own tools only. Run it inside a fresh VM (an image build step,
# or by hand). Who builds the image, and where this script is called from, is
# outside this file.
#
#   tools/env/build-gate-image.sh --pvr-fw-blob <path to pvr_fw_blob.h>
#
# Steps (each is idempotent):
#   1. tools/setup-gate-host.sh --install     packages, rustup components, targets
#   2. copy pvr_fw_blob.h into the layout      never committed: the one hand step
#   3. tools/cts/deps-env.sh --build-missing   every tree tools/cts/deps.toml pins
#   4. tools/cts/deps-env.sh --check           the verdict the gate also uses
#
# Step 2 runs before step 3 because the blob entry has no build, so
# --build-missing would fail on it. The packages need passwordless sudo
# (setup-gate-host.sh prints the owner commands otherwise and exits non-zero,
# which stops this script).
set -euo pipefail

if [ "${1-}" != --pvr-fw-blob ] || [ -z "${2-}" ]; then
  echo "usage: ${0##*/} --pvr-fw-blob <path to pvr_fw_blob.h>" >&2
  exit 2
fi
blob=$2
[ -f "$blob" ] || { echo "${0##*/}: no such file: $blob" >&2; exit 2; }

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

"$root/tools/setup-gate-host.sh" --install

# deps-env.sh without arguments stops when the layout directory is missing (a
# fresh VM); only --build* create it. Create it where deps-env.sh puts it.
mkdir -p "$("$root/tools/cts/deps-env.sh" --layout)"

# deps-env.sh knows the layout; the firmware entry's path is its PVR_FW_BLOB line.
fw=$("$root/tools/cts/deps-env.sh" | sed -n 's/^PVR_FW_BLOB=//p')
[ -n "$fw" ] || { echo "${0##*/}: deps-env.sh prints no PVR_FW_BLOB line" >&2; exit 1; }
mkdir -p "$(dirname "$fw")"
cp "$blob" "$fw"

"$root/tools/cts/deps-env.sh" --build-missing
"$root/tools/cts/deps-env.sh" --check
