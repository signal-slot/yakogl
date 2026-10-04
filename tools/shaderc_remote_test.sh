#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# shaderc_remote_test.sh - start a compile server and check the remote shader
# back end against the offline table. No board and no GPU: this exercises the
# shader back end and its reflection only.
#
# The check itself is tests/shaderc_remote.rs, which asks the server for the
# shaders of src/shaderc/blobs.rs and compares what comes back with what the
# table holds -- USC dwords and every ShcShader field, blend variants
# included. This script is the harness around it: build the tool, start the
# server, run the test, stop the server.
#
# Config:
#   PCO_SHC   the pco_shc binary (required; the server refuses without it)
#   GLSLANG   glslangValidator (default: PATH)
#   ROWS      how much of the table to sweep: a count, or `all` (default 64).
#             The table has ~2000 rows and each is a glslang + pco_shc run, so
#             `all` takes a while the first time and seconds afterwards, from
#             the caches.
#   PORT      the port to run the server on (default 7877, not the protocol's
#             default 7777, so a server you already have keeps its port)
#   OUT       where the log and the cache go (default build/shaderc-remote-test)
#
# A row that differs is reported by the test with the first differing dword.
# It means one of two things, and the test's message says how to tell them
# apart: the remote path is wrong, or blobs.rs was generated with a different
# pco_shc than this host has.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
OUT="${OUT:-$ROOT/build/shaderc-remote-test}"
PORT="${PORT:-7877}"
ROWS="${ROWS:-64}"

mkdir -p "$OUT"

if [ -z "${PCO_SHC:-}" ]; then
	echo "shaderc_remote_test: set PCO_SHC to the pco_shc binary" >&2
	exit 1
fi

echo "=== building yakogl-tools"
cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p yakogl-tools
TOOL=""
for d in .target target; do
	for p in release debug; do
		[ -x "$ROOT/$d/$p/yakogl-tools" ] && TOOL="$ROOT/$d/$p/yakogl-tools"
	done
done
if [ -z "$TOOL" ]; then
	echo "shaderc_remote_test: cargo built yakogl-tools but it is not where it was looked for" >&2
	exit 1
fi
export YAKOGL_TOOLS="$TOOL"

echo "=== starting the compile server on port $PORT"
"$HERE/shaderc_server.py" --port "$PORT" --cache "$OUT/cache" > "$OUT/server.log" 2>&1 &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT

# Wait for the port rather than sleeping a guess.
for _ in $(seq 60); do
	if python3 -c "import socket, sys
try:
    socket.create_connection(('127.0.0.1', $PORT), timeout=1).close()
except OSError:
    sys.exit(1)
" 2>/dev/null; then
		break
	fi
	sleep 1
done

echo "=== comparing the remote back end with the offline table (ROWS=$ROWS)"
rc=0
YAKOGL_SHADERC_SERVER="127.0.0.1:$PORT" YAKOGL_SHADERC_TEST_ROWS="$ROWS" \
	cargo test --quiet --manifest-path "$ROOT/Cargo.toml" \
	--no-default-features --features host-test,gpu-powervr,shaderc-remote \
	--test shaderc_remote -- --nocapture || rc=$?

echo
echo "=== the server's own account of it (tail of $OUT/server.log)"
tail -n 20 "$OUT/server.log"

echo
if [ "$rc" = 0 ]; then
	echo "shaderc_remote_test: PASS"
else
	echo "shaderc_remote_test: FAIL (the test exited $rc)"
fi
exit "$rc"
