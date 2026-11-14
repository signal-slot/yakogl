#!/usr/bin/env bash
# Self-test of record.sh: every subcommand on a temporary directory, with
# crafted logs holding each kind of secret. Needs no board and no ssh.
set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rec() { bash "$here/record.sh" "$@"; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/record_selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
export RECORDS_ROOT=$tmp/records/rpi5

n=0 failed=0
ok() { n=$((n + 1)); if ! "$@"; then failed=$((failed + 1)); echo "FAIL: $*" >&2; fi; }
has() { grep -qF -- "$2" "$1"; }
hasnt() { ! grep -qiF -- "$2" "$1"; }
fails() { ! "$@" >/dev/null 2>&1; }
matches() { [[ $1 =~ $2 ]]; }

# new
dir=$(rec new u3-probe)
sha=$(git -C "$here" rev-parse --short=8 HEAD)
ok test -d "$dir"
ok [ "$(dirname "$dir")" = "$RECORDS_ROOT" ]
ok [ "${dir##*-}" = "$sha" ]
ok matches "$(basename "$dir")" '^[0-9]{8}-[0-9]{4}-u3-probe-[0-9a-f]{8}$'
ok fails rec new 'bad name'
ok fails rec new ../x

# throttled
rec throttled "$dir" linux-before 0x0
echo "throttled=0x50005" | rec throttled "$dir" zephyr-after
echo "0x50000" | rec throttled "$dir" end
ok has "$dir/throttled.txt" 'linux-before get_throttled=0x0'
ok has "$dir/throttled.txt" 'zephyr-after get_throttled=0x50005'
ok has "$dir/throttled.txt" 'end get_throttled=0x50000'
ok [ "$(wc -l <"$dir/throttled.txt")" -eq 3 ]
ok fails rec throttled "$dir" x notahex
ok fails rec throttled "$dir" 'bad label' 0x0
ok fails rec throttled "$tmp/nodir" x 0x0

# crafted log with every kind of secret and things that must survive
cat >"$dir/console.log" <<'LOG'
ssh to 192.168.10.42 failed, retry 10.0.0.7
peer fe80::dead:beef%eth0 and 2001:db8:0:0:0:0:0:1 and ::1 and 2001:db8::
eth0 link/ether dc:a6:32:12:34:56 brd ff:ff:ff:ff:ff:ff
uart /dev/ttyUSB3 and /dev/ttyACM0 via /dev/serial/by-id/usb-FTDI_FT232R_USB_UART_A50285BI-if00-port0
iSerial 3 A50285BI
ID_SERIAL_SHORT=A50285BI
Serial Number: 0123456789AB
host boardhost.example.net user ownername
scanout fb=0x3e000000 size=0x7e9000 hvs=0x107c580000 time 12:34:56 v6.12.34
LOG
printf '\000\001binary 192.168.1.1\n' >"$dir/blob.bin"
cp "$dir/blob.bin" "$tmp/blob.orig"
mkdir "$dir/sub"
cp "$dir/console.log" "$dir/sub/nested.txt"

export REDACT_NAMES='boardhost.example.net,ownername'
ok fails rec check "$dir"
rec redact "$dir"
ok rec check "$dir"
for f in console.log sub/nested.txt; do
    for s in 192.168.10.42 10.0.0.7 fe80::dead 2001:db8 dc:a6:32 ff:ff:ff ttyUSB3 ttyACM0 \
        A50285BI 0123456789AB boardhost ownername FT232R; do
        ok hasnt "$dir/$f" "$s"
    done
    ok has "$dir/$f" 'fb=0x3e000000'
    ok has "$dir/$f" 'hvs=0x107c580000'
    ok has "$dir/$f" 'size=0x7e9000'
    ok has "$dir/$f" '12:34:56'
    ok has "$dir/$f" 'v6.12.34'
done
ok has "$dir/console.log" '<ip4>'
ok has "$dir/console.log" '<ip6>'
ok has "$dir/console.log" '<mac>'
ok has "$dir/console.log" '<usb-serial>'
ok has "$dir/console.log" '<redacted-name>'
ok cmp -s "$dir/blob.bin" "$tmp/blob.orig"
ok has "$dir/throttled.txt" 'zephyr-after get_throttled=0x50005'

# idempotent
cp "$dir/console.log" "$tmp/once.log"
rec redact "$dir"
ok cmp -s "$dir/console.log" "$tmp/once.log"

# check catches a late leak; names only count while REDACT_NAMES is set
echo 'late 172.16.0.9' >>"$dir/console.log"
ok fails rec check "$dir"
rec redact "$dir"
ok rec check "$dir"
echo 'ownername again' >>"$dir/console.log"
ok fails rec check "$dir"
unset REDACT_NAMES
ok rec check "$dir"

ok fails rec bogus
echo "record_selftest: $((n - failed)) of $n checks, $failed failed"
[ "$failed" -eq 0 ]
