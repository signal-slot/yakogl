#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_session_selftest.sh - tools/board-session-lib.sh without a board.
#
#   tools/board_session_selftest.sh
#
# board_session_others against `who` texts written here: no remote session,
# only this host's address, only the address the board sees for our ssh (the
# connected task machine's case), a foreign address beside ours (printed),
# several foreign ones (sorted, unique), an empty seen address (the old rule),
# IPv6 and host names (exact strings), X display and tmux entries (local).
# board_session_seen_cmd: its text, that it does not expand here, and what a
# shell with an SSH_CONNECTION prints from it. Sourcing the lib under set -u
# defines its two functions and nothing else. Only documentation addresses
# (192.0.2.0/24, 198.51.100.0/24, 2001:db8::/32) and made-up names appear.
#
# Prints "board_session_selftest: N checks, F failed"; exit 0 only when F is
# 0 and every check written in this file ran (as tools/board_selftest.sh: each
# "check" call notes its line, and every line that starts with one must be
# among them). An end before the last line fails too.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)

at_end=
trap 'rc=$?; [ -n "$at_end" ] || { echo "board_session_selftest: ended before its last line (rc $rc)"; [ "$rc" != 0 ] || exit 1; }' EXIT
n=0
fails=0
declare -A ran=() # the lines of this file a check was called from
check() { # check DESCRIPTION COMMAND...
    local what=$1
    shift
    n=$((n + 1))
    ran[${BASH_LINENO[0]}]=1
    if "$@"; then echo "  PASS  $what"; else echo "  FAIL  $what"; fails=$((fails + 1)); fi
}
# is EXPECTED WHO MINE SEEN: board_session_others prints EXPECTED exactly
# (shown when not) and returns 0
is() {
    local got rc=0
    got=$(board_session_others "$2" "$3" "$4") || rc=$?
    [ "$rc" = 0 ] && [ "$got" = "$1" ] && return 0
    printf '        rc %s, expected:\n%s\n        got:\n%s\n' "$rc" "$1" "$got" | sed 's/^/        /'
    return 1
}

# ---- sourcing: under set -u, nothing but the two functions -------------------
# vars_added: the variable names a source of the lib adds, in a fresh bash -u
# (v0 and v1 are the probe's own; BASH_*, PIPESTATUS and _ move by themselves)
vars_added() {
    # shellcheck disable=SC2016  # expanded by the inner bash
    bash -uc 'v0=$(compgen -v); . "$1"; v1=$(compgen -v)
              comm -13 <(grep -vxE "v0|v1|_|BASH_.*|PIPESTATUS" <<<"$v0" | sort) <(grep -vxE "v0|v1|_|BASH_.*|PIPESTATUS" <<<"$v1" | sort)' \
        _ "$HERE/board-session-lib.sh"
}
fn_before=$(declare -F)
# shellcheck source=tools/board-session-lib.sh
. "$HERE/board-session-lib.sh"
check "sourcing under set -u adds no variable" \
    eval '[ -z "$(vars_added 2>&1)" ]'
check "sourcing defines exactly board_session_others and board_session_seen_cmd" \
    eval '[ "$(comm -13 <(sort <<<"$fn_before") <(declare -F | sort))" = "declare -f board_session_others
declare -f board_session_seen_cmd" ]'
check "sourcing in a fresh bash -u prints nothing and exits 0" \
    eval '[ -z "$(bash -uc ". \"$HERE/board-session-lib.sh\"" 2>&1)" ]'

# ---- board_session_others -----------------------------------------------------
MINE='127.0.0.1
192.0.2.10
::1
2001:db8::10'
SEEN=198.51.100.7
CONSOLE='root     ttyS0        2026-11-16 09:00
root     tty1         2026-11-16 09:01'

check "no session at all: nothing" is '' '' "$MINE" "$SEEN"
check "console logins only (no parentheses): nothing" is '' "$CONSOLE" "$MINE" "$SEEN"
check "only this host's address: nothing" \
    is '' "$CONSOLE
pi       pts/0        2026-11-16 10:00 (192.0.2.10)" "$MINE" "$SEEN"
check "only the address the board sees for our ssh (a connected task machine): nothing" \
    is '' "pi       pts/0        2026-11-16 10:00 (198.51.100.7)
pi       pts/1        2026-11-16 10:02 (198.51.100.7)" "$MINE" "$SEEN"
check "the seen address is ours even when this host has no address list" \
    is '' "pi       pts/0        2026-11-16 10:00 (198.51.100.7)" '' "$SEEN"
check "one foreign address beside ours and the seen one: the foreign one" \
    is '192.0.2.99' "pi       pts/0        2026-11-16 10:00 (198.51.100.7)
pi       pts/1        2026-11-16 10:01 (192.0.2.10)
alice    pts/2        2026-11-16 10:05 (192.0.2.99)" "$MINE" "$SEEN"
check "several foreign addresses: sorted, unique, one per line" \
    is '192.0.2.50
192.0.2.99
198.51.100.200' "bob      pts/3        2026-11-16 10:06 (198.51.100.200)
alice    pts/2        2026-11-16 10:05 (192.0.2.99)
alice    pts/4        2026-11-16 10:07 (192.0.2.99)
carol    pts/5        2026-11-16 10:08 (192.0.2.50)
pi       pts/0        2026-11-16 10:00 (198.51.100.7)" "$MINE" "$SEEN"
check "empty seen address keeps the old rule: the forwarded address is foreign" \
    is '198.51.100.7' "pi       pts/0        2026-11-16 10:00 (198.51.100.7)
pi       pts/1        2026-11-16 10:01 (192.0.2.10)" "$MINE" ''
check "seen address left out (two arguments) is the empty one" \
    eval '[ "$(board_session_others "pi pts/0 2026-11-16 10:00 (198.51.100.7)" "$MINE")" = 198.51.100.7 ]'
check "the seen address is an exact match, not a prefix (198.51.100.70 stays foreign)" \
    is '198.51.100.70' "pi       pts/0        2026-11-16 10:00 (198.51.100.70)" "$MINE" "$SEEN"
check "IPv6 in parentheses: ours by exact string, a foreign one printed" \
    is '2001:db8::99' "pi       pts/0        2026-11-16 10:00 (2001:db8::10)
alice    pts/1        2026-11-16 10:01 (2001:db8::99)" "$MINE" "$SEEN"
check "IPv6 seen address (the board sees our ssh over IPv6): ours" \
    is '' "pi       pts/0        2026-11-16 10:00 (2001:db8:0:1::7)" "$MINE" '2001:db8:0:1::7'
check "a host name is compared as a string: foreign unless it is listed" \
    is 'devbox.example' "pi       pts/0        2026-11-16 10:00 (devbox.example)
pi       pts/1        2026-11-16 10:01 (gate.example)" "$MINE
gate.example" "$SEEN"
check "X display (:0, :1.0) and tmux pane entries are local: nothing" \
    is '' "pi       :0           2026-11-16 08:00 (:0)
pi       pts/6        2026-11-16 08:01 (:1.0)
pi       pts/7        2026-11-16 08:02 (tmux(1234).%0)" "$MINE" "$SEEN"
check "a tmux entry beside a foreign session: only the foreign one" \
    is '192.0.2.99' "pi       pts/7        2026-11-16 08:02 (tmux(1234).%0)
alice    pts/2        2026-11-16 10:05 (192.0.2.99)" "$MINE" "$SEEN"
check "this host's addresses separated by blanks, as an unquoted list: same answer" \
    is '192.0.2.99' "pi       pts/1        2026-11-16 10:01 (192.0.2.10)
alice    pts/2        2026-11-16 10:05 (192.0.2.99)" "127.0.0.1 192.0.2.10 ::1" "$SEEN"
check "who -a style lines (idle and pid before the address): the last field is read" \
    is '192.0.2.99' "alice    + pts/2        2026-11-16 10:05 00:03        4242 (192.0.2.99)
pi       + pts/0        2026-11-16 10:00   .          4100 (198.51.100.7)" "$MINE" "$SEEN"
check "returns 0 when a foreign session is found (the caller decides)" \
    eval 'board_session_others "alice pts/2 2026-11-16 10:05 (192.0.2.99)" "$MINE" "$SEEN" >/dev/null'
check "runs under set -euo pipefail with nothing to print and returns 0" \
    eval '[ -z "$(bash -euo pipefail -c ". \"$HERE/board-session-lib.sh\"; board_session_others \"\" \"\" \"\"; echo -n" 2>&1)" ]'

# ---- board_session_seen_cmd ---------------------------------------------------
check "the seen command is the literal text, not expanded here" \
    eval '[ "$(SSH_CONNECTION="192.0.2.1 1 192.0.2.2 22" board_session_seen_cmd)" = "echo \"\${SSH_CONNECTION%% *}\"" ]'
check "the seen command, run by a shell with an SSH_CONNECTION, prints its first field" \
    eval '[ "$(SSH_CONNECTION="198.51.100.7 50122 192.0.2.20 22" bash -c "$(board_session_seen_cmd)")" = 198.51.100.7 ]'
check "the seen command with an IPv6 SSH_CONNECTION prints the whole client address" \
    eval '[ "$(SSH_CONNECTION="2001:db8::7 50122 2001:db8::20 22" bash -c "$(board_session_seen_cmd)")" = 2001:db8::7 ]'
check "the seen command outside ssh prints an empty line (the old rule follows)" \
    eval '[ "$(env -u SSH_CONNECTION bash -c "$(board_session_seen_cmd)")" = "" ]'
check "the seen command runs under a POSIX sh too (the board's login shell may be one)" \
    eval '[ "$(SSH_CONNECTION="198.51.100.7 50122 192.0.2.20 22" sh -c "$(board_session_seen_cmd)")" = 198.51.100.7 ]'
check "end to end: the seen command's answer makes the forwarded session ours" \
    eval 'seen=$(SSH_CONNECTION="198.51.100.7 50122 192.0.2.20 22" bash -c "$(board_session_seen_cmd)");
          [ -z "$(board_session_others "pi pts/0 2026-11-16 10:00 (198.51.100.7)" "$MINE" "$seen")" ]'

# every check written above ran: the call lines of this file against the
# lines noted by check
at_end=1
notrun=0
while IFS=: read -r line _; do
    [ -n "${ran[$line]:-}" ] && continue
    echo "  FAIL  the check written at line $line did not run"
    notrun=$((notrun + 1))
done < <(grep -nE '^[[:space:]]*check ' "$0")
echo "board_session_selftest: $n checks, $fails failed, $notrun written but not run"
[ "$fails" = 0 ] && [ "$notrun" = 0 ]
