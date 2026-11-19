#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host test of tri_check.h under ASan/UBSan. Prints "tri_check: N checks, 0 failed".
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d "${TMPDIR:-/tmp}/tri_check_test.XXXXXX")
trap 'rm -rf "$out"' EXIT
${CC:-cc} -std=c99 -Wall -Wextra -Werror -g -O1 \
	-fsanitize=address,undefined -fno-sanitize-recover=undefined \
	-I"$here/.." -o "$out/tri_check_test" "$here/tri_check_test.c" -lm
"$out/tri_check_test"
