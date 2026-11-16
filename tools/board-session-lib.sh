# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# board-session-lib.sh - which of a board's `who` sessions are this run's own,
# for tools/board.sh common_prechecks. SOURCE it, do not run it: it defines
# two functions and nothing else (no variable, no command, no trap), so it is
# safe under set -u and set -e. tools/board_session_selftest.sh checks it
# without a board.
#
# A board is refused when someone else is logged in. A remote session is ours
# when its address is one of this host's addresses (`ip -o addr`), or when it
# is the address the board itself sees for our ssh: the first field of
# SSH_CONNECTION read ON the board. From a connected task machine the ssh
# reaches the board through a forwarding or a proxy, so the board sees an
# address that is none of this host's; without the second rule board.sh
# refused its own session and no Pi 4 / i.MX8MP run could start (plan #351,
# pi4-tree). An empty seen address (the read failed, a dry run) keeps the old
# rule: only this host's addresses are ours.
#
# The call common_prechecks makes, in place of its awk | sort | grep pipeline
# (the source line beside the others at the top of board.sh):
#
#   # shellcheck source=tools/board-session-lib.sh
#   . "$REPO/tools/board-session-lib.sh"
#   ...
#   bget who 'who' '' || die "cannot run who on the board"
#   bget seen "$(board_session_seen_cmd)" '' || die "cannot read the address the board sees for our ssh"
#   mine=$(ip -o addr 2>/dev/null | awk '{sub(/\/.*/, "", $4); print $4}')
#   others=$(board_session_others "$who" "$mine" "$seen")
#   [ -z "$others" ] || die "the board has sessions from other machines ($others); ask before taking it:
#   $who"
#
# (`seen` joins common_prechecks' `local` line; the seen command runs without
# SU, as `who` does: sudo would drop SSH_CONNECTION.)
#
# Caveat: a second machine behind the same forwarding address as ours looks
# like us to the board; the board lease (board.sh take_lock) is what keeps two
# runs apart, this check only finds people logged in by hand.

# board_session_others WHO_TEXT MINE_ADDRS SEEN_ADDR: the remote addresses in
# WHO_TEXT (the board's `who`) that are neither in MINE_ADDRS (this host's
# addresses, one per line or separated by blanks) nor equal to SEEN_ADDR (the
# board's first field of SSH_CONNECTION, may be empty), sorted and unique, one
# per line; nothing when every session is ours. The address of a session is
# its last field when that field is in parentheses, as board.sh read it
# before, with the outer parentheses taken off; lines without one (a console
# login) are local. So are the parenthesised forms that name no machine: an X
# display (":0", ":1.0") and a tmux pane ("tmux(1234).%0"). Every other
# address (IPv4, IPv6, a host name) is compared as an exact string.
# Always returns 0.
board_session_others() {
    local who_text=$1 mine=$2 seen=${3:-}
    awk '$NF ~ /^\(.*\)$/ {
            a = substr($NF, 2, length($NF) - 2)
            if (a == "" || a ~ /^:/ || a ~ /^tmux\(/) next
            print a
        }' <<<"$who_text" | sort -u |
        # shellcheck disable=SC2086  # one address per word
        grep -vxF -f <(printf '%s\n' $mine ${seen:+"$seen"} | grep .) || true
}

# board_session_seen_cmd: the board-side command that prints the client
# address the board sees for this ssh connection (empty outside ssh). Printed
# in single quotes so that ${SSH_CONNECTION} expands on the board, not here.
board_session_seen_cmd() {
    # shellcheck disable=SC2016  # expanded by the board's shell
    printf '%s\n' 'echo "${SSH_CONNECTION%% *}"'
}
