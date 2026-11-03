#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh -- the partial-render Kconfig fragments (tools/cts/pr-configs/)
# against the Kconfig files that declare their symbols and against
# tools/cts/pr-session.sh, the script whose -D options they mirror.
#
# For each fragment:
#   1. an SPDX header on its first lines (within the first three);
#   2. every CONFIG_<SYM>=<value> line names a <SYM> declared as `config <SYM>`
#      in driver/zephyr/Kconfig, driver/samples/zephyr/glmark2/Kconfig or
#      driver/samples/zephyr/gl_test/Kconfig;
#   3. the values equal the -DCONFIG_... options of the image's west line in
#      `pr-session.sh <name> --knob off|on [--glmark2 --one-in-flight] --dry-run`
#      (the gltest line for gl_test-*.conf, the glmark2 line for glmark2-*.conf),
#      and each -DCONFIG_ option of that line is in the fragment, so neither
#      side can gain a setting the other lacks. The dry run reads no board or
#      toolchain environment (GLMARK2_SRC only names a path in the printed
#      line); --allow-local lets it print on a topic branch.
# Also: README.md here has its SPDX header and names every fragment.
#
# Exit 0 when every check passes, 1 otherwise (each failure is one line).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
SESSION=$REPO/tools/cts/pr-session.sh
KCONFIGS=(driver/zephyr/Kconfig driver/samples/zephyr/glmark2/Kconfig driver/samples/zephyr/gl_test/Kconfig)
EXPECTED=(gl_test-175-off.conf gl_test-175-on.conf glmark2-d320c8a5-off.conf glmark2-d320c8a5-on.conf)

fails=0
passes=0
ok() { echo "ok   $*"; passes=$((passes + 1)); }
bad() { echo "FAIL $*"; fails=$((fails + 1)); }

cd "$REPO" || { echo "selftest.sh: no repository at $REPO"; exit 1; }
[ -x "$SESSION" ] || { bad "tools/cts/pr-session.sh is missing or not executable"; echo "$passes ok, $fails failed"; exit 1; }

spdx_ok() { # spdx_ok FILE: the two SPDX lines at the top (any comment syntax),
    # within the first three lines so a shebang may come first
    head -3 "$1" | grep -q 'SPDX-License-Identifier: Apache-2.0' &&
        head -3 "$1" | grep -q 'SPDX-FileCopyrightText: 2026 Signal Slot, Inc.'
}

# declared SYM: `config SYM` in one of the Kconfig files
declared() { grep -qE "^(menu)?config[[:space:]]+$1[[:space:]]*$" "${KCONFIGS[@]}"; }

# dry_opts IMAGE OPTIONS...: the -DCONFIG_X=V tokens of IMAGE's west line in
# the dry run, one `CONFIG_X=V` per output line. The west line is the one
# with `-d .../IMAGE ` after `west build`.
dry_opts() {
    local image=$1
    shift
    local out
    out=$("$SESSION" selftest "$@" --allow-local --dry-run 2>&1) ||
        { echo "pr-session.sh --dry-run failed: $out" >&2; return 1; }
    grep -E "west build .* -d [^ ]*/$image " <<< "$out" |
        grep -oE -- '-DCONFIG_[A-Za-z0-9_]+=("[^"]*"|[^ )]*)' | sed 's/^-D//'
}

# the directory holds exactly the expected fragments
mapfile -t present < <(cd "$HERE" && ls -- *.conf 2> /dev/null | LC_ALL=C sort)
if [ "${present[*]}" = "${EXPECTED[*]}" ]; then
    ok "fragments: ${EXPECTED[*]}"
else
    bad "fragments present (${present[*]:-none}) are not the expected set (${EXPECTED[*]})"
fi

for f in "${EXPECTED[@]}"; do
    path=$HERE/$f
    [ -f "$path" ] || { bad "$f: missing"; continue; }
    if spdx_ok "$path"; then ok "$f: SPDX header"; else bad "$f: no SPDX header on lines 1-3"; fi

    # the settings: CONFIG_X=V lines, nothing else but comments and blanks
    mapfile -t lines < <(grep -vE '^[[:space:]]*(#|$)' "$path")
    declare -A frag=()
    nbad=0
    for l in "${lines[@]}"; do
        if [[ $l =~ ^CONFIG_([A-Za-z0-9_]+)=(\"[^\"]*\"|[yn]|[0-9]+)$ ]]; then
            sym=${BASH_REMATCH[1]}
            frag[$sym]=${BASH_REMATCH[2]}
            if declared "$sym"; then
                ok "$f: CONFIG_$sym declared"
            else
                bad "$f: CONFIG_$sym is declared in none of ${KCONFIGS[*]}"
            fi
        else
            bad "$f: line is not CONFIG_<SYM>=<\"string\"|y|n|number>: $l"
            nbad=$((nbad + 1))
        fi
    done
    [ "${#frag[@]}" -gt 0 ] || bad "$f: sets nothing"

    # the matching pr-session.sh options
    case "$f" in
    gl_test-175-off.conf) image=gltest opts=(--knob off) ;;
    gl_test-175-on.conf) image=gltest opts=(--knob on) ;;
    glmark2-d320c8a5-off.conf) image=glmark2 opts=(--knob off --glmark2 --one-in-flight) ;;
    glmark2-d320c8a5-on.conf) image=glmark2 opts=(--knob on --glmark2 --one-in-flight) ;;
    esac
    mapfile -t dry < <(dry_opts "$image" "${opts[@]}")
    if [ "${#dry[@]}" -eq 0 ]; then
        bad "$f: pr-session.sh ${opts[*]} --dry-run printed no -DCONFIG_ option for the $image image"
        unset frag
        continue
    fi
    declare -A want=()
    for kv in "${dry[@]}"; do
        sym=${kv%%=*}
        sym=${sym#CONFIG_}
        want[$sym]=${kv#*=}
    done
    # pr-session.sh's board arguments (common-images.sh's watchdog arm) are the
    # board's, not the session's: they are documented in README.md, not in a
    # fragment, and are not compared here.
    board_arg() { case "$1" in YAKOGL_AM62P_RTI_SELF_ARM | SOC_AM62PX_A53_FEED_INHERITED_WDT) return 0 ;; esac; return 1; }
    for sym in "${!frag[@]}"; do
        if [ -z "${want[$sym]+x}" ]; then
            bad "$f: CONFIG_$sym=${frag[$sym]} is not among pr-session.sh ${opts[*]}'s -D options"
        elif [ "${want[$sym]}" = "${frag[$sym]}" ]; then
            ok "$f: CONFIG_$sym=${frag[$sym]} as pr-session.sh ${opts[*]}"
        else
            bad "$f: CONFIG_$sym=${frag[$sym]}, but pr-session.sh ${opts[*]} passes ${want[$sym]}"
        fi
    done
    for sym in "${!want[@]}"; do
        board_arg "$sym" && continue
        [ -n "${frag[$sym]+x}" ] || bad "$f: pr-session.sh ${opts[*]} passes CONFIG_$sym=${want[$sym]}, which the fragment lacks"
    done
    unset frag want
done

# README.md: SPDX header, every fragment named
readme=$HERE/README.md
if [ -f "$readme" ]; then
    if spdx_ok "$readme"; then ok "README.md: SPDX header"; else bad "README.md: no SPDX header on lines 1-3"; fi
    for f in "${EXPECTED[@]}"; do
        grep -q -- "$f" "$readme" && ok "README.md names $f" || bad "README.md does not name $f"
    done
else
    bad "README.md: missing"
fi
spdx_ok "$HERE/selftest.sh" && ok "selftest.sh: SPDX header" || bad "selftest.sh: no SPDX header on lines 1-3"

echo "$passes ok, $fails failed"
[ "$fails" -eq 0 ]
