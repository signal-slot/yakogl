#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The replay glue without a sample: the Slint replay program linked against
# fake_sample.c over the real library, per GPU. A clean run must pass; a run
# with FAKE_BAD=1 must fail and the report must name the GL error, the
# unresolved name and the draws that sampled an incomplete texture.
#
#   host-replay/selftest/selftest.sh <out-dir> [gpu...]
set -euo pipefail

out=${1:?usage: selftest.sh <out-dir> [gpu...]}
shift
here=$(cd "$(dirname "$0")" && pwd)
hr=$(cd "$here/.." && pwd)
repo=$(cd "$hr/../.." && pwd)
# shellcheck source=../common.sh
. "$hr/common.sh"
hr_init "$repo"
mkdir -p "$out"
out=$(cd "$out" && pwd)
gpus=("$@")
[ ${#gpus[@]} -gt 0 ] || read -r -a gpus <<< "$HR_GPUS_DEFAULT"
hr_check_gpus "${gpus[@]}"

cc -O1 -g -Wall -Wextra -I"$repo/include" -c "$here/fake_sample.c" -o "$out/fake_sample.o"
ar rcs "$out/libfake.a" "$out/fake_sample.o"

fail() { echo "selftest: FAIL: $*" >&2; exit 1; }
for sample in slint; do
	for g in "${gpus[@]}"; do
		hr_lib "$out" "$g"
		o="$out/$sample"
		mkdir -p "$o"
		ln -sfn "$out/lib-$g" "$o/lib-$g"
		FAKE_BAD= hr_run "$o" "$g" "$repo/tools/$sample/replay/replay.c" "$out/libfake.a"
		grep -q "summary frames=[1-9][0-9]* failed=0 glerrors=0 refused=0 incomplete=0 unresolved=0" "$o/$g.log" ||
			fail "$sample $g: the clean run did not pass: $(tail -3 "$o/$g.log")"
		FAKE_BAD=1 hr_run "$o" "$g" "$repo/tools/$sample/replay/replay.c" "$out/libfake.a"
		python3 -I "$hr/report.py" "$o" "$g" > "$o/bad-$g.md" && fail "$sample $g: the bad run was not reported"
		grep -q 'GL error 0x500' "$o/bad-$g.md" || fail "$sample $g: no GL error in the report"
		grep -q 'unresolved glNoSuchEntryPoint' "$o/bad-$g.md" || fail "$sample $g: no unresolved name in the report"
		# one draw per frame samples a one-level texture under a mipmap filter:
		# the library logs the first (one line), the summary counts every one
		grep -q 'draws sampling an incomplete texture [1-9]' "$o/bad-$g.md" ||
			fail "$sample $g: no incomplete-texture draws in the report"
		grep -q 'is incomplete and samples (0,0,0,1)' "$o/bad-$g.md" ||
			fail "$sample $g: the library's incomplete-texture line is not in the report"
		echo "selftest: $sample $g ok"
	done
done
