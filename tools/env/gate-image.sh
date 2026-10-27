#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gate-image.sh -- build the gate image from this checkout, as the pool's
# next generation. Needs an orion session (`orion login`; ORION_URL names
# the server that keeps the lab's pools) and the firmware header, found
# through the dependency layout (tools/cts/deps-env.sh, PVR_FW_BLOB) or given
# as --pvr-fw-blob <path>.
#
#   tools/env/gate-image.sh [--pvr-fw-blob <path>]
#
# It stages what the recipe copies (git archive of HEAD, the header) under
# build/gate-image/ and runs `orion image build tools/env/yakogl-gate.image.yaml`.
# The proof that the image is good is the pool's next CI run on it.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
blob=
while [ $# -gt 0 ]; do
    case "$1" in
        --pvr-fw-blob) blob=$2; shift 2 ;;
        *) echo "usage: $0 [--pvr-fw-blob <path>]" >&2; exit 2 ;;
    esac
done
if [ -z "$blob" ]; then
    blob=$("$REPO/tools/cts/deps-env.sh" | sed -n 's/^PVR_FW_BLOB=//p')
fi
[ -f "$blob" ] || { echo "gate-image: no firmware header at '$blob' (--pvr-fw-blob)" >&2; exit 1; }
command -v orion >/dev/null || { echo "gate-image: no orion CLI on PATH" >&2; exit 1; }
stage=$REPO/build/gate-image
mkdir -p "$stage"
git -C "$REPO" archive --format=tar -o "$stage/yakogl.tar" HEAD
cp "$blob" "$stage/pvr_fw_blob.h"
echo "gate-image: tree $(git -C "$REPO" rev-parse --short HEAD), $(du -h "$stage/yakogl.tar" | cut -f1) staged in $stage"
cd "$REPO"
ORION_RECIPES="$REPO/tools/env" orion image build tools/env/yakogl-gate.image.yaml
rm -f "$stage/pvr_fw_blob.h"
