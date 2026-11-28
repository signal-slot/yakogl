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
#   tools/cts/t6-deqp-images.sh --run [--unattended] [--dry-run] <table> <img-dir> <out-dir>
#   tools/cts/t6-deqp-images.sh --help
#
# --run runs the built images on the board (the console from the farm's
# lease or T6_CONSOLE_CMD, T6_HAND or --unattended): its section below.
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
# ---- --run: the whole tree on the board -------------------------------------
#
#   tools/cts/t6-deqp-images.sh --run [--unattended] [--dry-run] <table> <img-dir> <out-dir>
#
# --run runs the images of <img-dir> (built by this script from <table>) on
# the NanoPC-T6, one jump per row, through `tools/board.sh nanopc-t6 tree`
# (TREE_IMAGE_DIR=<img-dir>; each row's jump is driver/tools/kexec-zephyr/
# nanopc-t6.sh --go). <out-dir> is the tree's record directory, outside any
# checkout (the runner's rule). It is resumable: a row whose run directory
# has an INDEX.txt is skipped, so the same command after a stop goes on from
# the first row not yet run. --dry-run passes board.sh's --dry-run (nothing
# touches the board). Everything comes from the environment, by name; no
# value of a variable below is printed or written:
#
#   The board: BOARD_HOST (user@host, root), else RESOURCE_NANOPC_T6_SSH, the
#   lease of target:nanopc-t6.
#
#   The console (the debug UART, 1500000 8N1, read only), the first set of:
#     T6_CONSOLE_CMD  a shell command whose stdout is the console's bytes
#                     (whatever the farm provides; write it with the lease's
#                     variable names, the shell expands them). It runs once
#                     for the whole tree under tools/cts/t6-console-pty.py,
#                     which gives the runner a pty as its T6_UART, never
#                     writes to the console, drops the bytes nobody reads
#                     and starts the command again when it ends.
#     T6_UART, BOARD_UART  a local tty, or rfc2217://<host>:<port> of the
#                     network serial adapter (nanopc-t6.sh opens it per row
#                     with t6-uart-rfc2217.py and sets 1500000 8N1).
#     the lease of target:nanopc-t6-console (resource serial-nanopc-t6):
#                     <P>_HOST and <P>_PORT for P = $T6_CONSOLE_LEASE (a
#                     prefix, if set), RESOURCE_NANOPC_T6_CONSOLE,
#                     RESOURCE_SERIAL_NANOPC_T6, taken as
#                     rfc2217://<host>:<port>. The adapter takes one client:
#                     while another one (the farm's own serial console server
#                     of the lease, a second run) holds it, each row is
#                     refused with "busy" before anything is copied; the stop
#                     says so (below).
#   Without any of them, BOARD_SCREEN or T6_SCREEN (with T6_CRASH=1) is the
#   only path out of the board; with neither, exit 2.
#
#   Who is at the power switch (the board has no remote power control):
#     T6_HAND=<name>  an attended run, as before: the person who cuts the
#                     power when a row hangs.
#     --unattended or T6_UNATTENDED=1  an agent's run with nobody there: no
#                     T6_HAND needed (T6_HAND set wins). nanopc-t6.sh records
#                     a board that does not come back as incomplete (hang or
#                     reset), board.sh stops the tree at that row (rc 5,
#                     never keep booting), and this script stops cleanly.
#   Neither: exit 2 before anything runs.
#
# When the tree stops before its end (board.sh's exit not 0: a row that hung
# or reset, rc 5; a refusal, rc 3, e.g. a busy console or a board that does
# not answer; anything else), <out-dir>/INCOMPLETE.txt says at which row and
# why, lists the rows incomplete (run, not back: their run directories), the
# rows left (no finished run) and the command that resumes; the same lines go
# to stdout, and the exit code is board.sh's. A row recorded incomplete is
# skipped by the resume (it has its INDEX.txt): to run it again, move its run
# directory out of <out-dir> first. A tree that ends with no incomplete row
# removes INCOMPLETE.txt. Every board.sh output line is also kept in
# <out-dir>/run-<date>.log. T6_BOARD_SH replaces tools/board.sh (the self-test,
# tools/cts/t6-deqp-run-selftest.sh, uses that seam with a fake console).
t6_run() {
	local unatt=${T6_UNATTENDED:-} dry=() a pos=() table img out repo src p h q log rc
	for a in "$@"; do
		case "$a" in
		--unattended) unatt=1 ;;
		--dry-run) dry=(--dry-run) ;;
		-*) echo "t6-deqp-images: --run: unknown option $a" >&2; return 2 ;;
		*) pos+=("$a") ;;
		esac
	done
	rdie() { echo "t6-deqp-images: --run: $*" >&2; exit 2; }
	[ ${#pos[@]} = 3 ] || rdie "usage: $0 --run [--unattended] [--dry-run] <table> <img-dir> <out-dir>"
	table=${pos[0]} img=${pos[1]} out=${pos[2]}
	[ -f "$table" ] || rdie "no table $table"
	[ -d "$img" ] || rdie "no image directory $img (build it first: $0 $table $img)"
	case "$unatt" in '' | 0) unatt=0 ;; 1) ;; *) rdie "T6_UNATTENDED='$unatt': 1 or 0/unset" ;; esac
	# the record directory: outside any checkout (nanopc-t6.sh's rule)
	p=$out
	while [ ! -d "$p" ]; do p=$(dirname "$p"); done
	[ "$(git -C "$p" rev-parse --is-inside-work-tree 2>/dev/null)" != true ] ||
		rdie "$out is inside a git checkout: the record holds the board's dmesg and console bytes, which are never committed; use a directory outside it"
	# the hand
	if [ -n "${T6_HAND:-}" ]; then
		echo "t6-deqp-images: --run: attended: T6_HAND is set (a hang waits for that person)"
		unset T6_UNATTENDED
	elif [ "$unatt" = 1 ]; then
		export T6_UNATTENDED=1
		echo "t6-deqp-images: --run: unattended (T6_UNATTENDED=1): nobody at the power switch; a row that hangs or resets is recorded incomplete and the tree stops there"
	else
		rdie "who is at the power switch? set T6_HAND=<name> (attended), or --unattended / T6_UNATTENDED=1 (an agent's run: a hang stops the tree, recorded as incomplete)"
	fi
	# the board
	if [ -z "${BOARD_HOST:-}" ] && [ -n "${RESOURCE_NANOPC_T6_SSH:-}" ]; then
		export BOARD_HOST=$RESOURCE_NANOPC_T6_SSH
		echo "t6-deqp-images: --run: board: RESOURCE_NANOPC_T6_SSH (the lease; value not shown)"
	fi
	[ -n "${BOARD_HOST:-}" ] || rdie "no board: set BOARD_HOST, or lease target:nanopc-t6 (RESOURCE_NANOPC_T6_SSH)"
	# the console
	local bridge=()
	if [ -n "${T6_CONSOLE_CMD:-}" ]; then
		[ -z "${T6_UART:-}${BOARD_UART:-}" ] || rdie "T6_CONSOLE_CMD and T6_UART/BOARD_UART are both set: set one console"
		bridge=(python3 -I "$HERE/t6-console-pty.py" run --)
		src="T6_CONSOLE_CMD (one session for the tree, through t6-console-pty.py)"
	elif [ -n "${T6_UART:-}${BOARD_UART:-}" ]; then
		export BOARD_UART=${BOARD_UART:-$T6_UART}
		case "$BOARD_UART" in
		rfc2217://*) src="$([ -n "${T6_UART:-}" ] && echo T6_UART || echo BOARD_UART): an RFC 2217 adapter (opened per row by t6-uart-rfc2217.py)" ;;
		*) src="$([ -n "${T6_UART:-}" ] && echo T6_UART || echo BOARD_UART): a local tty" ;;
		esac
	else
		for p in ${T6_CONSOLE_LEASE:-} RESOURCE_NANOPC_T6_CONSOLE RESOURCE_SERIAL_NANOPC_T6; do
			h=${p}_HOST q=${p}_PORT
			if [ -n "${!h:-}" ] && [ -n "${!q:-}" ]; then
				export BOARD_UART="rfc2217://${!h}:${!q}"
				src="the lease, ${p}_HOST and ${p}_PORT: an RFC 2217 adapter (opened per row by t6-uart-rfc2217.py)"
				break
			fi
		done
	fi
	if [ -n "${src:-}" ]; then
		echo "t6-deqp-images: --run: console: $src; values not shown"
	elif [ -n "${BOARD_SCREEN:-}${T6_SCREEN:-}" ]; then
		echo "t6-deqp-images: --run: console: none; the screen only (BOARD_SCREEN/T6_SCREEN, needs T6_CRASH=1)"
	else
		rdie "no console: set T6_CONSOLE_CMD, T6_UART or BOARD_UART, or lease target:nanopc-t6-console (RESOURCE_NANOPC_T6_CONSOLE_HOST/_PORT); or BOARD_SCREEN for the screen only"
	fi
	repo=$(cd "$HERE/../.." && pwd)
	local bsh=${T6_BOARD_SH:-$repo/tools/board.sh}
	local resume="$0 --run$([ "$unatt" = 1 ] && [ -z "${T6_HAND:-}" ] && echo " --unattended") $table $img $out"
	if [ ${#dry[@]} = 1 ]; then
		echo "t6-deqp-images: --run: dry run:${bridge[*]:+ ${bridge[*]}} TREE_IMAGE_DIR=$img $bsh --dry-run nanopc-t6 tree $table $out"
		TREE_IMAGE_DIR=$img "${bridge[@]}" bash "$bsh" --dry-run nanopc-t6 tree "$table" "$out"
		return $?
	fi
	mkdir -p "$out" || rdie "cannot create $out"
	log=$out/run-$(date +%Y%m%d-%H%M%S).log
	echo "t6-deqp-images: --run: tree of $table, images $img, record $out (board.sh's lines also in $log)"
	TREE_IMAGE_DIR=$img "${bridge[@]}" bash "$bsh" nanopc-t6 tree "$table" "$out" < /dev/null 2>&1 | tee "$log"
	rc=${PIPESTATUS[0]}
	t6_stop_report "$rc" "$out" "$log" "$resume"
	return "$rc"
}

# t6_row_state OUT LABEL IMAGE: done | incomplete <dir> | left
t6_row_state() {
	local out=$1 lab=$2 im=$3 sha8='[0-9a-f]{8}' d st=left
	[ -f "$im" ] && sha8=$(sha256sum "$im" | cut -c1-8)
	for d in "$out"/*/; do
		d=${d%/}
		[[ ${d##*/} =~ ^[0-9]{8}-[0-9]{4}-(.+)-($sha8)$ ]] && [ "${BASH_REMATCH[1]}" = "$lab" ] || continue
		[ -f "$d/INDEX.txt" ] || continue
		if [ "$(sed -n 's/^rc_cycle=//p' "$d/meta.txt" 2>/dev/null | tail -1)" = 5 ]; then
			st="incomplete $d"
		else
			echo done
			return
		fi
	done
	echo "$st"
}

# t6_stop_report RC OUT LOG RESUME: INCOMPLETE.txt and the stop's lines
t6_stop_report() {
	local rc=$1 out=$2 log=$3 resume=$4 lab im _s _f st left=() inc=() last why d busy
	[ -f "$out/tree.txt" ] || { echo "t6-deqp-images: --run: board.sh stopped (rc $rc) before it wrote $out/tree.txt: nothing ran (its lines: $log)"; return; }
	while IFS=$'\t' read -r lab im _s _f; do
		st=$(t6_row_state "$out" "$lab" "$im")
		case "$st" in
		done) ;;
		incomplete*) inc+=("$lab ${st#incomplete }") ;;
		*) left+=("$lab") ;;
		esac
	done < "$out/tree.txt"
	if [ "$rc" = 0 ] && [ ${#inc[@]} = 0 ]; then
		rm -f "$out/INCOMPLETE.txt"
		echo "t6-deqp-images: --run: the tree is complete: every row of $out/tree.txt has its record"
		return
	fi
	last=$(grep -aoE '^=== [A-Za-z0-9_-]+ rc=[0-9]+ ' "$log" | tail -1 | awk '{print $2, $3}')
	case "$rc" in
	0) why="the tree ended (tally run), with rows recorded incomplete" ;;
	5) why="hang or reset: the board did not come back into Armbian after the jump; $([ "${T6_UNATTENDED:-}" = 1 ] && echo "unattended: nobody at the power switch, so the board may stay where the image left it until its power is cut; check it answers (ssh) before resuming" || echo "T6_HAND is to power-cycle it")" ;;
	3)
		why="a row was refused before its jump (nothing copied or changed on the board for it)"
		d=$(ls -1dt "$out"/*/ 2>/dev/null | head -1)
		busy=$(grep -ahiE 'busy|another client' "${d%/}/kexec.log" "$log" 2>/dev/null | head -1)
		if [ -n "$busy" ]; then
			why+="; the console is busy: the adapter's one client slot is held (by the farm's serial console server of the lease, or another run); free it, or give the console another way (T6_CONSOLE_CMD), then resume"
		elif grep -aqiE 'does not answer|ssh:|Connection (timed out|refused)|No route to host' "${d%/}/kexec.log" "$log" 2>/dev/null; then
			why+="; the board does not answer (ssh): check it is up before resuming"
		else
			why+="; see ${d%/}/kexec.log and ${d%/}/meta.txt"
		fi
		;;
	*) why="board.sh ended with rc $rc (see $log)" ;;
	esac
	{
		echo "t6-deqp-images --run: INCOMPLETE $(date '+%F %T')"
		echo "stopped: rc $rc${last:+ at ${last% *} (${last#* })}: $why"
		echo "incomplete (run, the board not back: recorded, skipped by a resume; move the directory out of $out to run the row again): ${#inc[@]}"
		for st in ${inc[@]+"${inc[@]}"}; do echo "  ${st%% *} ${st#* }"; done
		echo "left (no finished run): ${#left[@]}${left[*]:+: ${left[*]}}"
		echo "resume: $resume"
		echo "  (the same environment by name: BOARD_HOST or the lease, the console, $([ -n "${T6_HAND:-}" ] && echo T6_HAND || echo "T6_UNATTENDED=1"))"
	} > "$out/INCOMPLETE.txt"
	sed 's/^/t6-deqp-images: --run: /' "$out/INCOMPLETE.txt"
}

if [ "${1:-}" = --help ] || [ "${1:-}" = -h ]; then
	sed -n '/^# Stage 2 of dEQP/,/^HERE=/p' "$0" | sed '$d; s/^# \{0,1\}//'
	sed -n '/^# ---- --run:/,/^t6_run()/p' "$0" | sed '1d; $d; s/^# \{0,1\}//'
	exit 0
fi
if [ "${1:-}" = --run ]; then
	shift
	t6_run "$@"
	exit $?
fi
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
