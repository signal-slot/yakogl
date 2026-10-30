#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of pvr-cmd3d-decode.py: the sample log (built from render.rs's
# print format, dword n = 0xA0000000|n except the shared block and the flags)
# decodes to the expected file byte for byte, from a file and from stdin.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
td="$here/testdata"
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
python3 -I "$here/pvr-cmd3d-decode.py" "$td/pvr-cmd3d-sample.log" >"$tmp"
diff -u "$td/pvr-cmd3d-sample.expected" "$tmp"
python3 -I "$here/pvr-cmd3d-decode.py" <"$td/pvr-cmd3d-sample.log" | diff -u "$td/pvr-cmd3d-sample.expected" -
python3 -I "$here/pvr-cmd3d-decode.py" --pr-bgnd-dw 80 "$td/pvr-cmd3d-sample.log" >/dev/null
if echo "no words here" | python3 -I "$here/pvr-cmd3d-decode.py" 2>/dev/null; then
	echo "pvr-cmd3d-decode-selftest: empty input must fail" >&2
	exit 1
fi
echo "pvr-cmd3d-decode-selftest: ok"
