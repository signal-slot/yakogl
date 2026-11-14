#!/usr/bin/env bash
# mp2s_comeback.sh: read-only "OpenSTLinux is back" verdict for the
# STM32MP257F-DK after a standalone run (docs/notes/stm32mp257f-dk-standalone.md).
#
#   tools/mp2s_comeback.sh [--wait SECONDS] [--before-boot-id ID]
#
# The ssh target comes only from the environment: BOARD_HOST (the variable
# tools/board-stm32mp2.sh uses), else BOARD_SSH. It is never written to a file.
# Host key checking is on and never recorded. Nothing is changed on the
# board: one ssh session runs reads only (cat, uname, findmnt, systemctl
# is-active). No retry beyond the --wait poll, no reset, no restart.
#
# Output (redacted: addresses, the ssh target, the board's host name):
#   comeback: ok new boot, OpenSTLinux <uname -r>, /boot on SD, sshd active, tailscaled active
#   comeback: FAIL <what>
#   comeback: NEEDS A PERSON: no ssh within <s> s; check the USB-C supply first (...)
# Exit: 0 ok, 1 FAIL, 2 usage or no target, 4 needs a person.
set -u

WAIT=0
BEFORE=
while [ $# -gt 0 ]; do
    case $1 in
    --wait) WAIT=${2:-}; shift 2 || true ;;
    --before-boot-id) BEFORE=${2:-}; shift 2 || true ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "comeback: usage: $0 [--wait SECONDS] [--before-boot-id ID]" >&2; exit 2 ;;
    esac
done
case $WAIT in ''|*[!0-9]*) echo "comeback: --wait takes whole seconds" >&2; exit 2 ;; esac
TARGET=${BOARD_HOST:-${BOARD_SSH:-}}
[ -n "$TARGET" ] || { echo "comeback: set BOARD_HOST (or BOARD_SSH) to the board's ssh target; it comes from the lease" >&2; exit 2; }

# the reads, run by the board's sh; key=value lines
REMOTE='
echo "boot_id=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
echo "uptime=$(cut -d. -f1 /proc/uptime 2>/dev/null)"
echo "kernel=$(uname -r)"
echo "cmdline_root=$(tr " " "\n" < /proc/cmdline 2>/dev/null | sed -n "s/^root=//p" | head -n1)"
src=$(findmnt -n -o SOURCE /boot 2>/dev/null)
echo "boot_source=$src"
b=${src##*/}
case $b in mmcblk*p[0-9]*|nvme*p[0-9]*) d=${b%p[0-9]*} ;; *) d=${b%%[0-9]*} ;; esac
echo "boot_disk=$d"
echo "boot_type=$(cat /sys/block/$d/device/type 2>/dev/null)"
echo "findmnt_boot=$(findmnt -n -o SOURCE,FSTYPE /boot 2>/dev/null)"
echo "findmnt_root=$(findmnt -n -o SOURCE,FSTYPE / 2>/dev/null)"
echo "sshd=$(systemctl is-active sshd 2>/dev/null || true)"
echo "ssh=$(systemctl is-active ssh 2>/dev/null || true)"
echo "tailscaled=$(systemctl is-active tailscaled 2>/dev/null || true)"
echo "end=1"
'

TMP=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_comeback.XXXXXX") || exit 2
trap 'rm -rf "$TMP"' EXIT

# strip addresses, the ssh target and its parts (>= 4 chars)
redact() {
    local e=() s esc
    local host=${TARGET#*@} user=${TARGET%@*}
    for s in "$TARGET" "$host" "$user"; do
        [ ${#s} -ge 4 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        e+=(-e "s/$esc/<board>/g")
    done
    LC_ALL=C sed -E "${e[@]}" \
        -e 's/([0-9]{1,3}\.){3}[0-9]{1,3}/<address>/g' \
        -e 's/([0-9A-Fa-f]{0,4}:){3,7}[0-9A-Fa-f]{0,4}/<address>/g'
}
say() { printf '%s\n' "$*" | redact; }

try_once() {
    timeout 20 ssh -o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=8 \
        -o BatchMode=yes "$TARGET" "$REMOTE" > "$TMP/out" 2>/dev/null </dev/null &&
        grep -q '^end=1$' "$TMP/out"
}

deadline=$((SECONDS + WAIT))
until try_once; do
    if [ $SECONDS -ge $deadline ]; then
        say "comeback: NEEDS A PERSON: no ssh within $WAIT s; check the USB-C supply first (docs/notes/stm32mp257f-dk-bringup.md)"
        exit 4
    fi
    sleep 5 &
    wait $!
done

g() { sed -n "s/^$1=//p" "$TMP/out" | head -n1; }
boot_id=$(g boot_id)
kernel=$(g kernel)
croot=$(g cmdline_root)
btype=$(g boot_type)
sshd=$(g sshd)
sshs=$(g ssh)
ts=$(g tailscaled)

fails=()
if [ -z "$boot_id" ]; then
    fails+=("no boot id")
elif [ -n "$BEFORE" ] && [ "$boot_id" = "$BEFORE" ]; then
    fails+=("same boot id: no reboot happened")
fi
case $btype in
SD) where=SD ;;
MMC) where=eMMC; fails+=("/boot is on MMC (the eMMC), not the SD") ;;
*) where=unknown; fails+=("/boot device kind '$btype' is not SD") ;;
esac
[ -n "$croot" ] || fails+=("no root= on /proc/cmdline")
[ -n "$(g findmnt_root)" ] || fails+=("/ not mounted?")
[ -n "$(g findmnt_boot)" ] || fails+=("/boot not mounted")
if [ "$sshd" != active ] && [ "$sshs" != active ]; then
    fails+=("sshd not active (${sshd:-${sshs:-unknown}})")
fi
[ "$ts" = active ] || fails+=("tailscaled not active (${ts:-unknown})")

if [ ${#fails[@]} -gt 0 ]; then
    IFS=';'
    say "comeback: FAIL ${fails[*]}"
    exit 1
fi
if [ -n "$BEFORE" ]; then nb="new boot"; else nb="boot id not compared"; fi
say "comeback: ok $nb, OpenSTLinux $kernel, /boot on $where, sshd active, tailscaled active"
say "comeback: uptime $(g uptime) s, root=$croot, /boot $(g findmnt_boot), / $(g findmnt_root)"
exit 0
