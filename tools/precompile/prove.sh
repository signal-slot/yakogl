#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# prove.sh FEATURES - what tools/check.sh requires of yakogl-precompile for one
# build (FEATURES: gpu-powervr or gpu-vivante, optionally ,report-es2):
#
#   1. the whole list's table, made twice by two processes, is the same bytes;
#   2. `--verify`: each of its rows is what the compiler makes for that row
#      alone, in a fresh cache (and the table is the regeneration);
#   3. the row and code-byte counts of the whole list and of two applications'
#      sublists (femtovg, as Slint and the cluster link it; gl_test's ES 1.00
#      build) are tests/fixtures/precompile_counts.txt's;
#   4. the table compiles against the library it was made for (rustc, the
#      library's own rlib, -D warnings; metadata only, which const-evaluates
#      every static).
#
# Everything is written under build/precompiled/prove/. Prints the counts.
#
# prove.sh --artefacts FEATURES builds the tool and prints the paths of the
# tool and of the rlib it links, and stops: tools/check.sh keys the row by
# their bytes.
set -euo pipefail

ARTEFACTS=0
if [ "${1-}" = --artefacts ]; then
  ARTEFACTS=1
  shift
fi
FEATURES="${1:?usage: prove.sh [--artefacts] gpu-powervr|gpu-vivante[,report-es2]}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
TAG="${FEATURES//,/-}"
OUT="build/precompiled/prove"
mkdir -p "$OUT"
COUNTS="tests/fixtures/precompile_counts.txt"

# The tool and the library's rlib it links, from cargo's own account.
json=$(cargo build -q -p yakogl-precompile --features "$FEATURES" --message-format=json)
read -r TOOL RLIB < <(printf '%s\n' "$json" | python3 -c '
import json, sys
tool = rlib = None
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") != "compiler-artifact":
        continue
    if m["target"]["name"] == "yakogl-precompile" and m.get("executable"):
        tool = m["executable"]
    if m["target"]["name"] == "yakogl":
        rlib = [f for f in m["filenames"] if f.endswith(".rlib")][0]
if not tool or not rlib:
    sys.exit("prove.sh: cargo named no yakogl-precompile executable or yakogl rlib")
print(tool, rlib)')
DEPS="$(dirname "$RLIB")"
if [ "$ARTEFACTS" = 1 ]; then
  echo "$TOOL $RLIB"
  exit 0
fi

# 1. twice, two processes, the same bytes
"$TOOL" -o "$OUT/$TAG.rs" --pin "$COUNTS"
"$TOOL" -o "$OUT/$TAG.again.rs" >/dev/null
cmp "$OUT/$TAG.rs" "$OUT/$TAG.again.rs"
echo "prove.sh: $TAG: made twice, the same $(wc -c <"$OUT/$TAG.rs") bytes"

# 2. each row is what the compiler makes for it alone
"$TOOL" --verify "$OUT/$TAG.rs" | grep 'verify:'

# 3. the two applications' sublists, pinned
for app in femtovg gl_test_es100; do
  "$TOOL" --app "$app" -o "$OUT/$TAG.$app.rs" --pin "$COUNTS" | grep -v '^yakogl-precompile: wrote'
done

# 4. the table compiles against the library: the generated file is a module
# under `shaderc` in the library (`use super::...`, `crate::util::cname`),
# which this crate root stands in for.
cat >"$OUT/$TAG.check.rs" <<EOF
#![no_std]
#![allow(dead_code)]
pub use yakogl::shaderc::*;
mod util {
    pub use yakogl::util::cname;
}
#[path = "$ROOT/$OUT/$TAG.rs"]
mod table;
EOF
# The library's metadata for rustc: the .rmeta beside the rlib. A nightly
# since 2026-10 no longer embeds the full metadata in the rlib (cargo's
# no-embed-metadata), and rustc then refuses the rlib alone ("only metadata
# stub found for rlib dependency"); the .rmeta is what --emit=metadata reads.
RMETA="${RLIB%.rlib}.rmeta"
[ -f "$RMETA" ] || RMETA="$RLIB"
rustc --edition 2021 --crate-type lib --crate-name precompiled_check --emit=metadata -C panic=abort -D warnings \
  -L "dependency=$DEPS" --extern "yakogl=$RMETA" -o "$OUT/$TAG.check.rmeta" "$OUT/$TAG.check.rs"
echo "prove.sh: $TAG: the table compiles against the library (rustc -D warnings)"
