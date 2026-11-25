#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-log-check.sh <logdir>
#
# Checks a glmark2 board-session log directory (tools/cts/log/<date>-t6-
# glmark2-<sha8>/) before it is committed:
#
#   c1 layout     the directory holds tally-table.txt, tally-results-row.txt
#                 and state-compare.txt (regular files).
#   c2 redaction  no file in it matches the redaction expression below, the
#                 same one the glmark2 plan's acceptance check c2 runs with
#                 `grep -rEn`: an IPv4 address, a MAC address, a home path, a
#                 USB/ACM serial device path or a login at a host.
#
# A c2 hit is printed as <file>:<line> and the classes it matched, with the
# file relative to <logdir>. The matched text is never printed (nor the rest of
# the line), so the checker cannot echo an address it found.
#
# Output: one verdict line per check, then `glmark2-log-check: ok` (exit 0) or
# `glmark2-log-check: FAIL <n>`, n the number of failed checks (exit 1).
# A usage error (no argument, not a directory) exits 2.
#
# Self-test: tools/cts/glmark2-log-check-selftest.sh (run by
# tools/cts/cts-selftest.sh, check 62).
set -u

PROG=glmark2-log-check
REQUIRED="tally-table.txt tally-results-row.txt state-compare.txt"

# The plan's c2 expression, alternative by alternative (one class each).
RX_IPV4='([0-9]{1,3}\.){3}[0-9]{1,3}'
RX_MAC='([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}'
RX_HOME='/home/'
RX_TTY='/dev/tty(USB|ACM)'
RX_LOGIN='@[a-z0-9-]+\.'
RX="$RX_IPV4|$RX_MAC|$RX_HOME|$RX_TTY|$RX_LOGIN"
CLASSES="ipv4 mac home tty login"

usage() {
    echo "usage: $0 <logdir>" >&2
    exit 2
}

[ $# -eq 1 ] || usage
logdir=$1
if [ ! -d "$logdir" ]; then
    echo "$PROG: not a directory (path not shown)" >&2
    exit 2
fi
# grep and find below run inside the directory, so no output names its path.
cd -- "$logdir" || exit 2

fail=0

# mask <text>: the text with every c2 match replaced, for file names.
mask() { printf '%s' "$1" | sed -E "s#$RX#[masked]#g"; }

# ---- c1: layout -------------------------------------------------------------
missing=""
for f in $REQUIRED; do
    [ -f "$f" ] || missing="$missing $f"
done
if [ -z "$missing" ]; then
    echo "c1 layout: ok ($(echo $REQUIRED | sed 's/ /, /g'))"
else
    echo "c1 layout: FAIL missing$missing"
    fail=$((fail + 1))
fi

# ---- c2: redaction ----------------------------------------------------------
# Files that match, NUL-separated (any byte in a name is safe). -a reads a
# binary file as text, so its hits get line numbers like any other.
hits=0
files=0
while IFS= read -r -d '' f; do
    files=$((files + 1))
    # line number -> classes, from one grep per alternative
    declare -A cls=()
    for c in $CLASSES; do
        case $c in
            ipv4) rx=$RX_IPV4 ;; mac) rx=$RX_MAC ;; home) rx=$RX_HOME ;;
            tty) rx=$RX_TTY ;; login) rx=$RX_LOGIN ;;
        esac
        while IFS= read -r n; do
            cls[$n]="${cls[$n]:+${cls[$n]},}$c"
        done < <(grep -naE -- "$rx" "$f" | cut -d: -f1)
    done
    name=$(mask "${f#./}")
    for n in $(printf '%s\n' "${!cls[@]}" | sort -n); do
        echo "  hit $name:$n (${cls[$n]}; matched text masked)"
        hits=$((hits + 1))
    done
    unset cls
done < <(grep -rlaEZ -- "$RX" . 2>/dev/null)
nfiles=$(find . -type f | wc -l)
# A file grep cannot read would pass unseen: name it and fail c2.
unread=0
while IFS= read -r -d '' f; do
    echo "  unreadable $(mask "${f#./}")"
    unread=$((unread + 1))
done < <(find . -type f ! -readable -print0)
if [ "$unread" -ne 0 ]; then
    echo "c2 redaction: FAIL $unread file(s) unreadable, $hits line(s) match"
    fail=$((fail + 1))
elif [ "$hits" -eq 0 ]; then
    echo "c2 redaction: ok (no match in $nfiles file(s))"
else
    echo "c2 redaction: FAIL $hits line(s) in $files file(s) match"
    fail=$((fail + 1))
fi

if [ "$fail" -eq 0 ]; then
    echo "$PROG: ok"
    exit 0
fi
echo "$PROG: FAIL $fail"
exit 1
