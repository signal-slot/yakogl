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
# --dry-run is the builder's: "<LABEL> <filter>" per selected row, no
# environment, nothing built (images-selftest.sh 7f compares these lines byte
# for byte, so it stays exactly that and checks no environment of its own).
#
#   tools/cts/t6-deqp-images.sh --plan <table> <img-dir> [LABEL...]
#
# --plan is the check before a rebuild on a machine of one's own (the images
# of a board session do not travel with the checkout). It builds nothing:
# first it checks that the environment names the build needs are set (names
# only, never values: ZEPHYR_WS, ZEPHYR_SDK_INSTALL_DIR and DEQP_WORK — the
# builder reads DEQP_WORK, not DEQP_STAGE1), that DEQP_MODULE answers gles2
# or gles3, and that every LABEL is a row of the table (no LABEL: all rows);
# then it prints, per row, the cmake and ninja commands and the output paths
# as the builder will run them, the rows resolved through the builder's own
# --dry-run. An unknown row, a missing variable, a bad DEQP_MODULE or an
# unreadable table: exit 2 with a message naming it. What a name cannot be
# checked against (a real stage 1 in DEQP_WORK, the west workspace in
# ZEPHYR_WS, driver/src/fw/mali_csffw_blob.h) is refused at build time, as
# before. Self-test without toolchain or board: tools/cts/t6-deqp-images-selftest.sh.
if [ "${1:-}" != --plan ]; then
	exec "$(dirname "$0")/deqp-tree-images.sh" nanopc_t6 "$@"
fi
shift
HERE=$(cd "$(dirname "$0")" && pwd)
die() { echo "t6-deqp-images: --plan: $*" >&2; exit 2; }
[ $# -ge 2 ] || die "usage: $0 --plan <table> <img-dir> [LABEL...]"
TABLE=$1 OUT=$2
shift 2
for v in ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK; do
	if [ -z "${!v:-}" ]; then
		hint=
		if [ "$v" = DEQP_WORK ] && [ -n "${DEQP_STAGE1:-}" ]; then
			hint=" (DEQP_STAGE1 is set: the builder reads DEQP_WORK, not DEQP_STAGE1)"
		fi
		die "environment variable $v is not set$hint"
	fi
done
DEQP_MODULE=${DEQP_MODULE:-gles2}
case "$DEQP_MODULE" in
	gles2) ;;
	gles3) ;;
	*) die "DEQP_MODULE must be gles2 or gles3, not '$DEQP_MODULE'" ;;
esac
# the rows through the builder's own --dry-run: one resolver for both modes.
ROWS=$("$HERE/deqp-tree-images.sh" nanopc_t6 --dry-run "$TABLE" "$OUT" "$@") ||
	die "the rows of $TABLE did not resolve (the builder's message is above)"
for l in "$@"; do
	printf '%s\n' "$ROWS" | cut -d' ' -f1 | grep -qxF -- "$l" ||
		die "unknown row $l in $TABLE"
done
[ -n "$ROWS" ] || die "no rows selected in $TABLE"
REPO=$(cd "$HERE/../.." && pwd)
echo "t6-deqp-images: --plan: nothing is built. Rows of $TABLE, images under $OUT."
echo "t6-deqp-images: --plan: environment set: ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK${WEST_VENV:+ WEST_VENV} (values not shown), DEQP_MODULE=$DEQP_MODULE"
printf '%s\n' "$ROWS" | while IFS= read -r line; do
	l=${line%% *}
	rest=${line#"$l" }
	extra=
	if [[ $rest == *" ["*"]"* ]]; then
		extra=${rest##* [}
		extra=${extra%]}
		rest=${rest% [*}
	fi
	echo "$l: filter $rest"
	echo "  cmake -GNinja -S $REPO/driver/samples/zephyr/deqp -B $OUT/$l -DBOARD=nanopc_t6 -DYAKOGL_WERROR=ON -DZEPHYR_MODULES=\"\$($HERE/zephyr-modules.sh \"\$ZEPHYR_WS\")\" -DDEQP_SRC=\$DEQP_WORK/cts -DDEQP_BUILD=\$DEQP_WORK/build${extra:+ $extra} \"-DCONFIG_DEQP_CASE=\\\"$rest\\\"\""
	echo "  ninja -C $OUT/$l"
	echo "  image: $OUT/$l/zephyr/zephyr.bin  .built: $OUT/$l.built  log: $OUT/$l.log"
done
echo "t6-deqp-images: --plan: the builder also exports ZEPHYR_BASE=\$ZEPHYR_WS/zephyr, unsets DEQP_GL_CONFIG_NAME and refuses an existing $OUT/<LABEL>/ (never reused)."
[ "$DEQP_MODULE" = gles3 ] && echo "t6-deqp-images: --plan: after each image check-deqp-images.py runs with --gles3 (the context reports ES 3.0)."
exit 0
