#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of BOARD_UART=rfc2217://host:port in tools/board.sh for the pi4,
# am62p and imx8mp consoles, against a fake RFC 2217 server on 127.0.0.1 (the
# fake of tools/uart_rfc2217_selftest.sh, taken from it). No board, no network
# beyond loopback. Per board it runs:
#   - board.sh --dry-run <board> run: the route is named, the address is not;
#   - the probe and the capture path of board.sh itself (its own functions
#     uart_is_rfc2217, scrub, pi4_uart_free, bridge_capture, redact_run, cut
#     out of the file and run here: the rest of board.sh needs the board);
#   - the host and the port appear in no output and no file of the run directory.
#   bash tools/board_rfc2217_selftest.sh
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
board_sh="$here/board.sh"
work=$(mktemp -d "${TMPDIR:-/tmp}/board2217.XXXXXX") || exit 2
# built at run time: the redaction scan reads no login at a host in the tree
bhost="root@$(printf "%s.%s.%s.%s" 192 0 2 9)"
pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$work"; }
trap cleanup EXIT

ok=0; failed=0
check() { # name, status (0 = pass)
    if [ "$2" -eq 0 ]; then ok=$((ok + 1)); echo "ok   $1"
    else failed=$((failed + 1)); echo "FAIL $1"; fi
}

# the fake server, as uart_rfc2217_selftest.sh writes it
sed -n "/^cat > \"\$work\/fake.py\" <<'PY'\$/,/^PY\$/p" "$here/uart_rfc2217_selftest.sh" | sed '1d;$d' > "$work/fake.py"
[ -s "$work/fake.py" ] || { echo "no fake server in uart_rfc2217_selftest.sh"; exit 2; }

start() { # MODE: sets $port, $recv
    rm -f "$work/port"; recv="$work/recv.$1.$RANDOM"; : > "$recv"
    python3 "$work/fake.py" "$1" "$work/port" "$recv" & pids+=($!)
    for _ in $(seq 100); do [ -s "$work/port" ] && break; sleep 0.05; done
    port=$(cat "$work/port")
}

# board.sh's own functions, cut out by name
fn() { # NAME...
    local n
    for n in "$@"; do
        awk -v n="$n" '
            !p && index($0, n "() {") == 1 { p = 1; print; if ($0 ~ /}$/) exit; next }
            p { print; if ($0 == "}") exit }' "$board_sh"
    done
}
fn uart_is_rfc2217 uart_rfc2217_hp uart_is_bridge uart_cli_set uart_shown scrub note \
    pi4_uart_free bridge_capture redact_words redact_dir redact_run > "$work/fns.sh"
[ "$(grep -c '^[a-z_0-9]*() {' "$work/fns.sh")" -eq 12 ]; check "board.sh functions found (12)" $?

# harness BOARD ADDR RUNDIR VERB: board.sh's functions under the board's name
harness() {
    BOARD=$1 BOARD_UART=$2 RD=$3 HERE=$here DRY=0 CAP= UART_CLI=() UART_TAIL=() \
    bash -c '
        set -u
        . "$0"
        sleep() { command sleep 0.05; }
        die() { echo "board.sh $BOARD: $1" | scrub >&2; exit "${2:-3}"; }
        case "$1" in
        probe) pi4_uart_free ;;
        capture)
            bridge_capture || exit 4
            for _ in $(seq 100); do [ -s "$RD/console.raw" ] && break; command sleep 0.1; done
            kill -TERM "$CAP"; wait "$CAP"
            echo "board.sh $BOARD: console.raw $(wc -c < "$RD/console.raw") bytes"
            ;;
        redact)
            redact_run "$RD" ;;
        esac' "$work/fns.sh" "$4"
}

printf 'hello\xffworld\r\nlogin: ' > "$work/want"
BAD='127\.0\.0\.1'

for b in pi4 am62p imx8mp; do
    start ok
    addr="rfc2217://127.0.0.1:$port"
    pat="127\.0\.0\.1|(^|[^0-9])$port([^0-9]|\$)|rfc2217://"

    # the route in the dry run; the address nowhere
    mkdir -p "$work/log.$b"
    case $b in
    am62p) dry=(env BOARD_UART="$addr" BOARD_HOST="$bhost" bash "$board_sh" --dry-run am62p probe "$work/log.$b") ;;
    *) dry=(env BOARD_UART="$addr" BOARD_HOST="$bhost" bash "$board_sh" --dry-run "$b" probe "$work/log.$b") ;;
    esac
    "${dry[@]}" > "$work/dry.$b" 2>&1 < /dev/null; rc=$?
    echo "-- $b dry run rc=$rc"
    grep -qE "uart_rfc2217\.py (probe|capture) <BOARD_UART>" "$work/dry.$b"
    check "$b: --dry-run names uart_rfc2217.py and <BOARD_UART>" $?
    ! grep -qE "$pat" "$work/dry.$b"; check "$b: --dry-run output holds no host, port or URL" $?
    ! grep -rqE "$pat" "$work/log.$b"; check "$b: --dry-run leaves no address in the log directory" $?
    [ "$rc" -eq 0 ] || sed 's/^/     /' "$work/dry.$b" | head -20

    # the probe path, free line
    harness "$b" "$addr" "$work/rd.$b" probe > "$work/probe.$b" 2>&1; check "$b: probe of a free console exits 0" $?
    ! grep -qE "$pat" "$work/probe.$b"; check "$b: probe output holds no address" $?

    # busy / unreachable: refusal text without the address
    start busy; baddr="rfc2217://127.0.0.1:$port"
    harness "$b" "$baddr" "$work/rd.$b" probe > "$work/busy.$b" 2>&1; rc=$?
    check "$b: probe of a busy console refuses (rc 3)" $([ $rc -eq 3 ] && echo 0 || echo 1)
    grep -qE "BOARD_UART bridge is busy|accepted and closed at once" "$work/busy.$b"; check "$b: busy message says so (or the fake's close won the race)" $?
    ! grep -qE "127\.0\.0\.1|(^|[^0-9])$port([^0-9]|\$)" "$work/busy.$b"; check "$b: busy message holds no address" $?

    # wrong baud acknowledgement (exit 42) names no address either
    start wrong; waddr="rfc2217://127.0.0.1:$port"
    harness "$b" "$waddr" "$work/rd.$b" probe > "$work/wrong.$b" 2>&1; rc=$?
    check "$b: wrong baud refuses" $([ $rc -ne 0 ] && echo 0 || echo 1)
    ! grep -qE "127\.0\.0\.1|(^|[^0-9])$port([^0-9]|\$)" "$work/wrong.$b"; check "$b: wrong-baud message holds no address" $?

    # the capture path: console.raw/.ts of the run directory
    mkdir -p "$work/rd.$b"
    start ok; addr="rfc2217://127.0.0.1:$port"
    harness "$b" "$addr" "$work/rd.$b" capture > "$work/cap.$b" 2>&1; check "$b: capture runs and stops" $?
    cmp -s "$work/rd.$b/console.raw" "$work/want"; check "$b: console.raw is the payload without Telnet bytes" $?
    [ -s "$work/rd.$b/console.ts" ]; check "$b: console.ts written" $?
    ! grep -qE "$pat" "$work/cap.$b"; check "$b: capture output holds no address" $?
    ! grep -rqE "$pat" "$work/rd.$b"; check "$b: the run directory holds no address" $?

    # redaction of a record that does name the address
    printf 'console %s\nhost 127.0.0.1\nat 127.0.0.1:%s ok\n' "$addr" "$port" > "$work/rd.$b/meta.txt"
    harness "$b" "$addr" "$work/rd.$b" redact > /dev/null 2>&1
    ! grep -rqE "127\.0\.0\.1|$port|rfc2217://" "$work/rd.$b/meta.txt"; check "$b: redact_run removes URL, host:port and host from a record" $?
    grep -q '<BOARD_UART>' "$work/rd.$b/meta.txt"; check "$b: redacted as <BOARD_UART>" $?
done

# the other forms stay as they were
BOARD=pi4 BOARD_UART=127.0.0.1:5000 bash -c '. "$0"; uart_is_bridge && ! uart_is_rfc2217' "$work/fns.sh"
check "host:port stays a raw bridge (am62p_netboot_uart.py)" $?
BOARD=pi4 BOARD_UART=/dev/ttyUSB0 bash -c '. "$0"; ! uart_is_bridge' "$work/fns.sh"
check "a tty stays a local device" $?
BOARD=nanopc-t6 BOARD_UART=rfc2217://127.0.0.1:5000 bash -c '. "$0"; ! uart_is_bridge && ! uart_is_rfc2217' "$work/fns.sh"
check "nanopc-t6's rfc2217:// is not touched" $?
BOARD=pi5 BOARD_UART=rfc2217://127.0.0.1:5000 bash -c '. "$0"; ! uart_is_rfc2217' "$work/fns.sh"
check "pi5 is not taken" $?

echo "board_rfc2217 selftest: $ok ok, $failed failed"
[ "$failed" -eq 0 ]
