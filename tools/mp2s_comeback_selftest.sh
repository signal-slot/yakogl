#!/usr/bin/env bash
# Selftest of tools/mp2s_comeback.sh with a fake ssh on PATH. No board.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_cb_st.XXXXXX")
trap 'rm -rf "$T"' EXIT
mkdir "$T/bin"
cat > "$T/bin/ssh" <<'F'
#!/bin/sh
[ -f "$FAKE_OUT" ] || exit 255
cat "$FAKE_OUT"
F
chmod +x "$T/bin/ssh"
export PATH="$T/bin:$PATH" BOARD_HOST=root@dk-fake-box.example.net FAKE_OUT="$T/canned"
fail=0
mk() { # boot_id type sshd [kernel suffix]
    cat > "$T/canned" <<E
boot_id=$1
uptime=42
kernel=6.6.78-fake${4:-}
cmdline_root=PARTUUID=aaaa
boot_source=/dev/mmcblk1p4
boot_disk=mmcblk1
boot_type=$2
findmnt_boot=/dev/mmcblk1p4 ext4
findmnt_root=/dev/mmcblk1p5 ext4
sshd=$3
ssh=inactive
tailscaled=active
end=1
E
}
chk() { # name want_rc want_pattern [args...]
    local n=$1 rc=$2 pat=$3 out got
    shift 3
    out=$(bash "$HERE/mp2s_comeback.sh" "$@" 2>&1)
    got=$?
    if [ "$got" = "$rc" ] && printf '%s' "$out" | grep -q -- "$pat"; then
        echo "ok   $n"
    else
        echo "FAIL $n (rc $got): $out"
        fail=1
    fi
    if printf '%s' "$out" | grep -Eq 'dk-fake-box|example\.net|192\.168|10\.9\.8\.7|fd7a:'; then
        echo "FAIL $n: address or host name in output"
        fail=1
    fi
}
mk new SD active
chk "new boot id" 0 '^comeback: ok new boot, OpenSTLinux 6.6.78-fake, /boot on SD, sshd active, tailscaled active' --before-boot-id old
mk old SD active
chk "same boot id" 1 'FAIL same boot id' --before-boot-id old
mk new MMC active
chk "/boot on MMC" 1 'FAIL .*MMC' --before-boot-id old
mk new SD inactive
chk "sshd inactive" 1 'FAIL sshd not active' --before-boot-id old
mk new SD active "-dk-fake-box.example.net-192.168.7.9-10.9.8.7-fd7a:115c:a1e0::1"
chk "redaction" 0 'comeback: ok' --before-boot-id old
rm -f "$T/canned"
chk "never answers" 4 'NEEDS A PERSON: no ssh within 0 s; check the USB-C supply first' --wait 0
exit $fail
