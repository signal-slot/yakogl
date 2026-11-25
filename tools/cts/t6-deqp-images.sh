#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP on Zephyr, NanoPC-T6 (RK3588, Mali-G610): one image per
# "# image <LABEL>:" block of a table. The builder is
# tools/cts/deqp-tree-images.sh (its header says the arguments and the
# environment); this name gives it the board nanopc_t6.
#
#   tools/cts/t6-deqp-images.sh [--dry-run] <table> <img-dir> [LABEL...]
#
# --dry-run (first argument) resolves the rows and builds nothing: it checks
# that ZEPHYR_WS, ZEPHYR_SDK_INSTALL_DIR and DEQP_WORK are set (names only,
# never values), that every LABEL is a row of the table, then prints per row
# the cmake and ninja commands and the output paths. Exit 2 with a message on
# an unknown row, a missing variable or a bad table. DEQP_STAGE1 is not read
# by the builder: DEQP_WORK is the stage 1 directory.
HERE=$(cd "$(dirname "$0")" && pwd)
if [ "${1:-}" != --dry-run ]; then
	exec "$HERE/deqp-tree-images.sh" nanopc_t6 "$@"
fi
shift
die() { echo "t6-deqp-images: --dry-run: $*" >&2; exit 2; }
[ $# -ge 2 ] || die "usage: $0 --dry-run <table> <img-dir> [LABEL...]"
TABLE=$1 OUT=$2
shift 2
for v in ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK; do
	if [ -z "${!v:-}" ]; then
		hint=
		if [ "$v" = DEQP_WORK ] && [ -n "${DEQP_STAGE1:-}" ]; then
			hint=" (DEQP_STAGE1 is set, but the builder reads DEQP_WORK)"
		fi
		die "environment variable $v is not set$hint"
	fi
done
ROWS=$("$HERE/deqp-tree-images.sh" nanopc_t6 --dry-run "$TABLE" "$OUT" "$@") || die "cannot resolve rows of $TABLE"
for l in "$@"; do
	printf '%s\n' "$ROWS" | grep -q "^$l " || die "unknown row $l in $TABLE"
done
[ -n "$ROWS" ] || die "no rows selected in $TABLE"
REPO=$(cd "$HERE/../.." && pwd)
echo "t6-deqp-images: dry run, nothing is built; rows from $TABLE, images under $OUT"
echo "environment set: ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK"
printf '%s\n' "$ROWS" | while IFS= read -r line; do
	l=${line%% *}
	rest=${line#* }
	extra=
	if [[ $rest == *" ["*"]" ]]; then
		extra=${rest##* [}
		extra=${extra%]}
		rest=${rest% [*}
	fi
	echo "$l: filter $rest"
	echo "  cmake -GNinja -S $REPO/driver/samples/zephyr/deqp -B $OUT/$l -DBOARD=nanopc_t6 -DYAKOGL_WERROR=ON -DDEQP_SRC=\$DEQP_WORK/cts -DDEQP_BUILD=\$DEQP_WORK/build${extra:+ $extra} '-DCONFIG_DEQP_CASE=\"$rest\"'"
	echo "  ninja -C $OUT/$l"
	echo "  image: $OUT/$l/zephyr/zephyr.bin  record: $OUT/$l.built  log: $OUT/$l.log"
done
exit 0
