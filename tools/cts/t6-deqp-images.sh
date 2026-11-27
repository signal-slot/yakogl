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
#
# A build whose builder exited 0 (a refused build keeps its code and leaves
# nothing) additionally leaves two files at the top of <img-dir>, for
# driver/tools/uboot-t6/preflight.sh:
#
#   SHA256SUMS  sha256sum format, "<sum>  <LABEL>/zephyr/zephyr.bin" per label
#               of this call whose <LABEL>.built says rc=0 and whose image is
#               not empty. A selected row that failed or came out empty is
#               named on stderr and left out (preflight names it too). The
#               file is merged: a later selective build into the same
#               directory replaces only the lines of the labels it
#               selected and keeps the others.
#   COMMIT      the full hash of this checkout's HEAD, the commit built from.
#               A directory whose COMMIT holds another commit than this
#               checkout's HEAD is refused before building, exit 2 ("use a
#               fresh directory"): images of two commits never mix.
#               No git HEAD (a scratch copy of the scripts, as
#               images-selftest.sh runs them): a warning on stderr, no file,
#               the build's own exit code stands; preflight names it.
#
# --plan also says what a build would write (the SHA256SUMS lines it would
# keep and replace, the COMMIT check). --dry-run stays the builder's bytes.
# --plan and --dry-run write nothing: the dry run stays the builder's output
# byte for byte (images-selftest.sh 7f compares it), so it prints no news of
# what a build "would" write. T6_DEQP_BUILDER replaces the builder script;
# the self-test uses that seam.
HERE=$(cd "$(dirname "$0")" && pwd)
BUILDER=${T6_DEQP_BUILDER:-$HERE/deqp-tree-images.sh}
if [ "${1:-}" = --dry-run ]; then
	exec "$BUILDER" nanopc_t6 "$@"
fi
if [ "${1:-}" != --plan ]; then
	# images of two commits never mix: an <img-dir> whose COMMIT names
	# another commit than this checkout's HEAD is refused before building.
	HEADSHA=$(git -C "$HERE/../.." rev-parse HEAD 2>/dev/null)
	if [ $# -ge 2 ] && [ -n "$HEADSHA" ] && [ -s "$2/COMMIT" ] &&
		[ "$(head -n1 "$2/COMMIT")" != "$HEADSHA" ]; then
		echo "t6-deqp-images: $2/COMMIT is $(head -n1 "$2/COMMIT" | cut -c1-8), this checkout is ${HEADSHA:0:8}: use a fresh directory, images of two commits must not mix" >&2
		exit 2
	fi
	"$BUILDER" nanopc_t6 "$@" || exit $?
	# the builder exited 0 (a row may still have failed: its .built says
	# so): resolve this call's labels through the builder's own --dry-run,
	# one resolver as for --plan, and write the two files.
	[ $# -ge 2 ] || exit 0
	TABLE=$1 OUT=$2
	shift 2
	if [ ! -d "$OUT" ]; then
		echo "t6-deqp-images: no directory $OUT: the builder wrote no image, no SHA256SUMS and no COMMIT" >&2
		exit 0
	fi
	if ! SEL=$("$BUILDER" nanopc_t6 --dry-run "$TABLE" "$OUT" "$@"); then
		echo "t6-deqp-images: the rows of $TABLE did not resolve after the build" >&2
		exit 2
	fi
	SEL=$(printf '%s\n' "$SEL" | cut -d' ' -f1)
	# keep the lines of the labels this call did not select; the lines of
	# the selected ones are replaced by this call's result.
	: > "$OUT/SHA256SUMS.new"
	if [ -f "$OUT/SHA256SUMS" ]; then
		while IFS= read -r line; do
			[ -n "$line" ] || continue
			k=${line#*  }
			k=${k%%/*}
			case " $(echo $SEL) " in *" $k "*) continue ;; esac
			printf '%s\n' "$line" >> "$OUT/SHA256SUMS.new"
		done < "$OUT/SHA256SUMS"
	fi
	for l in $SEL; do
		img=$OUT/$l/zephyr/zephyr.bin
		if ! grep -q "^$l rc=0 " "$OUT/$l.built" 2>/dev/null || [ ! -s "$img" ]; then
			echo "t6-deqp-images: $l is not listed in SHA256SUMS (no rc=0 build of this call with a non-empty image)" >&2
			continue
		fi
		(cd "$OUT" && sha256sum "$l/zephyr/zephyr.bin") >> "$OUT/SHA256SUMS.new"
	done
	mv "$OUT/SHA256SUMS.new" "$OUT/SHA256SUMS"
	COMMIT=$(git -C "$HERE/../.." rev-parse HEAD 2>/dev/null)
	if [ -n "$COMMIT" ]; then
		echo "$COMMIT" > "$OUT/COMMIT"
		echo "t6-deqp-images: wrote $OUT/SHA256SUMS ($(wc -l < "$OUT/SHA256SUMS") image(s)) and $OUT/COMMIT (${COMMIT:0:8})"
	else
		echo "t6-deqp-images: wrote $OUT/SHA256SUMS ($(wc -l < "$OUT/SHA256SUMS") image(s)); no COMMIT: $(cd "$HERE/../.." && pwd) has no git HEAD (preflight names the missing file)" >&2
	fi
	exit 0
fi
shift
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
HEADSHA=$(git -C "$HERE/../.." rev-parse HEAD 2>/dev/null)
if [ -s "$OUT/COMMIT" ] && [ -n "$HEADSHA" ] && [ "$(head -n1 "$OUT/COMMIT")" != "$HEADSHA" ]; then
	echo "t6-deqp-images: --plan: a build would be refused (exit 2): $OUT/COMMIT is $(head -n1 "$OUT/COMMIT" | cut -c1-8), HEAD is ${HEADSHA:0:8}; use a fresh directory"
else
	echo "t6-deqp-images: --plan: a build would write $OUT/SHA256SUMS: replace the lines of $(printf '%s\n' "$ROWS" | cut -d' ' -f1 | tr '\n' ' ')and keep every other line; COMMIT ${HEADSHA:+${HEADSHA:0:8}}${HEADSHA:-none (no git HEAD)}"
fi
echo "t6-deqp-images: --plan: the builder also exports ZEPHYR_BASE=\$ZEPHYR_WS/zephyr, unsets DEQP_GL_CONFIG_NAME and refuses an existing $OUT/<LABEL>/ (never reused)."
[ "$DEQP_MODULE" = gles3 ] && echo "t6-deqp-images: --plan: after each image check-deqp-images.py runs with --gles3 (the context reports ES 3.0)."
exit 0
