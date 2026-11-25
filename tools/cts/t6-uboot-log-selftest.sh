#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Usage: t6-uboot-log-selftest.sh
# Selftest of t6-uboot-log.sh and t6-uboot-redact.py, host only, on synthetic
# records in the shape driver/tools/uboot-t6/README.md describes. A record that
# carries every secret kind is copied redacted: the staged files show the
# placeholders, none of the fixture values survives, the verdict, marker and
# typed-command lines are byte-for-byte what the record said, the binary is
# skipped, and a second copy of the same record reaches the same bytes (the
# placeholders re-redact to themselves). A UART mention in the form the redactor
# deliberately does not rewrite (the device word upper case) refuses the run:
# exit 1, the hit named by kind, nothing left behind. An existing destination is
# never overwritten; a record with no text file is refused; usage errors exit 2;
# a copied record passes t6-uboot-redact.py --check and, as it stands in the
# tree, holds no rfc2217 URL, tty path, MAC, tailnet-range address, serial
# number or UUID. Fixture values are built by concatenation, so this file's own
# text stays clean under tools/release/redaction-scan.py. Scratch under
# ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/t6-uboot-log.sh
redact=$here/t6-uboot-redact.py
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/t6-uboot-log-selftest.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }
export T6_UBOOT_LOG_ROOT=$tmp/log

# The fixture secrets, each assembled from parts on its line (the scan reads
# this file line by line; no whole look-alike stands here).
f_mac='aa:bb:cc:'"dd:ee:ff"
f_uuid='123e4567-e89b'"-12d3-a456-426614174000"
f_uuid_up='0FC63DAF-8483'"-4772-8E79-3D69D8477DE4"
f_quad_glued='host_100.'"64.1.2"
f_quad_tail='100.'"64.0.9"
f_quad_pub='192.0.'"2.7"
f_uart='/dev/'"ttyUSB0"
f_unit='serial-getty@'"ttyACM1.service"
f_url='rfc22'"17://host.example:7000"
f_tsnet='box1.example.'"ts.net"
f_v6='fe80'"::1ff:fe23:4567:890a"
f_port='3593'"4"
f_home='/ho'"me/labuser/x"
f_login='user@host.'"example"
f_login_at='user@t6.'"example.com"
f_root='root@'"t6host"
f_root_at='root'@"t6.example.com"
f_secretvalue='pass'"word=hunter2x"
f_serial='Serial Number'": 0123456789"
f_serialnum='SerialNumber'": 000000001234"
f_serialhash='serial#='"abcdef0123456789"
f_serialbare='Serial'": 1234567890"
f_serpath='/dev/serial/by-'"id/usb-Ftdi-x"
f_upper='/DEV/'"TTYUSB0"   # the surviving form: a device word upper case
f_version='vulkan-cts-3.6.1.0'"-555-g1234567890ab"

mkrec() { # $1 dir: the six record files plus a binary and a nested file
    rm -rf "$1"; mkdir -p "$1/sub"
    {
        printf 'U-Boot 2026.07_armbian\n'
        printf 'bootconsole %s at 1500000\n' "$f_uart"
        printf 'unit %s\n' "$f_unit"
        printf 'adapter port %s open as %s\n' "$f_port" "$f_url"
        printf 'peer %s inet6 %s\n' "$f_tsnet" "$f_v6"
        printf 'link %s up, tailnet probe %s\n' "$f_quad_glued" "$f_quad_tail"
        printf 'ipaddr=%s\n' "$f_quad_pub"
        printf 'stage %s done\n' "$f_home"
        printf 'ssh %s and %s refused; also %s and %s\n' "$f_login" "$f_login_at" "$f_root" "$f_root_at"
        printf 'dEQP Core %s starting: 1 case\n' "$f_version"
        printf 'gpu_clock_set: move CPLL / 5\n'
        printf 'CLKSEL_CON(158) 0x1f80 -> 0x40ff0024 -> 0x1f24\n'
    } > "$1/console.txt"
    printf '%s\n%s\n' '2026-11-25 14:31:02 handover by T6_HAND' "$f_secretvalue" > "$1/record.txt"
    {
        printf '## printenv\nethaddr=%s\nethact=ethernet@fe2d0000\n%s\n' "$f_mac" "$f_serialhash"
        printf '## mmc part\nPartition GUID code: %s\nPartition UUID: %s\n' "$f_uuid" "$f_uuid_up"
        printf '## mmc info\n%s\n## lsusb\n%s\n' "$f_serialbare" "$f_serialnum"
        printf '## kernel\nby-id %s\n' "$f_serpath"
    } > "$1/answers.txt"
    printf 'T6:A SCTLR_EL2 0x30400800\nT6:G SCTLR_EL2 read back 0x30400800\nt6_cold: PASS\nboard: t6_cold done: PSCI SYSTEM_RESET at 4226 ms (30 feeds)\n' > "$1/image.log"
    printf 'VERDICT t6_cold PASS\n' > "$1/verdict.txt"
    printf 'ext4load mmc 0:1 0x10000000 /boot/yakogl/t6_cold.bin\ngo 0x10000000\nreset\n' > "$1/typed.log"
    printf 'U-Boot 2026.07_armbian\n%s\n' "$f_mac" > "$1/sub/extra.log"
    printf '\0\1\2binary\n' > "$1/blob.bin"
}

run() { bash "$tool" "$@" >"$tmp/out" 2>&1; }
has() { grep -qF -- "$2" "$1"; }
nohit() { ! grep -qF -- "$2" "$1"; }

# 1. A record carrying every kind is copied redacted.
mkrec "$tmp/r1"
d=$tmp/log/2026-11-25-t6-uboot-t6_cold-0123abcd
if run "$tmp/r1" 2026-11-25 t6_cold 0123abcd; then ok "copy a record carrying every kind"; else bad "copy refused: $(cat "$tmp/out")"; fi
for f in console.txt record.txt answers.txt image.log verdict.txt typed.log sub/extra.log; do
    [ -f "$d/$f" ] || bad "layout: $d/$f missing"
done
[ ! -e "$d/blob.bin" ] && ok "binary skipped" || bad "binary copied"

# 2. The placeholders stand and no fixture value survives anywhere.
if has "$d/console.txt" '<uart>' && has "$d/console.txt" '<uart-net>' && has "$d/console.txt" '<port>' \
   && has "$d/console.txt" '<host>' && has "$d/console.txt" '<ipv6>' && has "$d/console.txt" '<addr>' \
   && has "$d/console.txt" '<home>' && has "$d/console.txt" '<login>' \
   && has "$d/record.txt" '<secret>' && has "$d/answers.txt" '<mac>' && has "$d/answers.txt" '<uuid>' \
   && has "$d/answers.txt" '<serial>' && has "$d/answers.txt" '<serial-path>' && has "$d/sub/extra.log" '<mac>'; then
    ok "placeholders stand in the copy"
else
    bad "placeholders missing: $(cat "$d/console.txt" "$d/record.txt" "$d/answers.txt")"
fi
clean=1
for v in "$f_mac" "$f_uuid" "$f_uuid_up" "$f_quad_glued" "$f_quad_tail" "$f_quad_pub" "$f_uart" \
         "$f_unit" "$f_url" "$f_tsnet" "$f_v6" "$f_port" "$f_home" "$f_login_at" "$f_root_at" "$f_root" \
         "$f_secretvalue" "$f_serial" "$f_serialnum" "$f_serialhash" "$f_serialbare" "$f_serpath"; do
    grep -qrF -- "$v" "$d" && { bad "a fixture value survived: $v"; clean=0; }
done
[ $clean -eq 1 ] && ok "no fixture value survived the copy"

# 3. What is not a secret stays byte for byte.
if has "$d/verdict.txt" 'VERDICT t6_cold PASS' && has "$d/image.log" 'T6:G SCTLR_EL2 read back 0x30400800' \
   && has "$d/image.log" 'board: t6_cold done: PSCI SYSTEM_RESET at 4226 ms (30 feeds)' \
   && has "$d/console.txt" 'gpu_clock_set: move CPLL / 5' \
   && has "$d/console.txt" 'CLKSEL_CON(158) 0x1f80 -> 0x40ff0024 -> 0x1f24' \
   && has "$d/console.txt" 'vulkan-cts-<3.6.1-0>-555-g1234567890ab' \
   && has "$d/answers.txt" 'ethact=ethernet@fe2d0000' \
   && has "$d/typed.log" 'ext4load mmc 0:1 0x10000000 /boot/yakogl/t6_cold.bin'; then
    ok "verdict, handover and command lines kept byte for byte"
else
    bad "a line that should be kept was rewritten: $(cat "$d/console.txt" "$d/answers.txt")"
fi

# 4. The copy is clean under the check, and a second copy reaches the same bytes.
if python3 -I "$redact" --check "$d" > "$tmp/chk" 2>&1; then ok "the copy passes --check"; else bad "the copy failed --check: $(cat "$tmp/chk")"; fi
d2=$tmp/log/2026-11-25-t6-uboot-cold2-0123abcd
if run "$tmp/r1" 2026-11-25 cold2 0123abcd && diff -r -- "$d" "$d2" > "$tmp/diff" 2>&1; then
    ok "redaction is stable: the same record copied twice is byte-identical"
else
    bad "a second copy differs: $(cat "$tmp/diff" "$tmp/out")"
fi

# 5. A surviving kind refuses the run and leaves nothing behind.
mkrec "$tmp/s1"
printf 'console %s lost\n' "$f_upper" >> "$tmp/s1/answers.txt"
if run "$tmp/s1" 2026-11-25 bad 0123abcd; then
    bad "a surviving kind was not refused"
else
    grep -q 'refusing' "$tmp/out" && grep -q 'uart' "$tmp/out" || bad "the refusal does not name the kind: $(cat "$tmp/out")"
    [ ! -e "$tmp/log/2026-11-25-t6-uboot-bad-0123abcd" ] || bad "a refusal left a destination behind"
    ls -d "$tmp"/log/.t6-uboot-log.* > /dev/null 2>&1 && bad "a refusal left the stage behind" || ok "a surviving kind is refused, nothing left behind"
fi

# 6. The redactor is stable on its own output.
if bash "$tool" "$tmp/r1" 2026-11-25 cold3 0123abcd > /dev/null 2>&1 \
   && bash "$tool" "$d" 2026-11-25 cold4 0123abcd > "$tmp/rerun" 2>&1 \
   && diff -r -- "$tmp/log/2026-11-25-t6-uboot-cold3-0123abcd" "$tmp/log/2026-11-25-t6-uboot-cold4-0123abcd" > "$tmp/diff2" 2>&1; then
    ok "a redacted tree redacts to itself"
else
    bad "a redacted tree changed on a second redaction: $(cat "$tmp/diff2" "$tmp/rerun")"
fi

# 7. An existing destination is never overwritten; an empty record is refused.
if run "$tmp/r1" 2026-11-25 t6_cold 0123abcd; then
    bad "an existing destination was overwritten"
else
    grep -q 'not overwriting' "$tmp/out" && ok "an existing destination is refused" || bad "overwrite refusal unclear: $(cat "$tmp/out")"
fi
mkdir -p "$tmp/empty"
if run "$tmp/empty" 2026-11-25 emptyrec 0123abcd; then bad "an empty record was copied"; else ok "an empty record is refused"; fi

# 8. Usage errors.
bash "$tool" > /dev/null 2>&1; [ $? -eq 2 ] && ok "no args exits 2" || bad "no args"
run "$tmp/r1" 2026-1-2 t6_cold 0123abcd; [ $? -eq 2 ] && ok "bad date exits 2" || bad "bad date"
run "$tmp/r1" 2026-11-25 'bad/name' 0123abcd; [ $? -eq 2 ] && ok "bad name exits 2" || bad "bad name"
run "$tmp/r1" 2026-11-25 t6_cold DEADBEEF; [ $? -eq 2 ] && ok "bad sha8 exits 2" || bad "bad sha8"
run "$tmp/nonexist" 2026-11-25 t6_cold 0123abcd; [ $? -eq 2 ] && ok "missing record exits 2" || bad "missing record"

echo "t6-uboot-log selftest: $nok ok, $nfail failed"
[ $nfail -eq 0 ]
