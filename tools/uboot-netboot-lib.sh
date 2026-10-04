# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# uboot-netboot-lib.sh - the U-Boot side of the one-shot boot, sourced by
# tools/board.sh (its uboot-tftp and uboot-mmc backends, and the primitives
# every backend uses). SOURCE it, do not run it.
#
# The design (the hook in front of bootcmd, the one-shot variable U-Boot
# deletes and saves before it loads anything, the eMMC source, the default
# and its boot count) is docs/standalone-boot.md; the TFTP source is
# driver/docs/am62p-netboot-plan.md and imx8mp-netboot-plan.md. The hook
# strings are the same on both boards; only the load address and the eMMC
# partition differ. An installed hook is compared with these strings byte for
# byte; "install" brings an installed hook of another version to these
# strings in one env write, bootcmd untouched.
#
# The runner sets, before sourcing:
#   NB_TAG        its name, the prefix of every message ("board.sh am62p")
#   NB_RECORDS    where "install" saves the env, relative to the main
#                 checkout ("records/am62p/netboot"); NETBOOT_BACKUP_DIR wins
#   NB_WATCHDOG   what the image must arm, for the refusal text ("the RTI")
#   NB_MMC        U-Boot's <dev>:<part> of the eMMC partition that holds the
#                 staged image ("0:1"; "-" on a board without U-Boot)
#   YAKO_ADDR     the load and entry address (the image's link address)
#   DRY           1 for --dry-run: every board, TFTP-root and host action is
#                 printed and none is run (board reads included)
#   REPO          the repository root
# and gets: YAKO_HOOK YAKO_DFLT YAKO_NET YAKO_MMC HOOK_PREFIX SSH_OPTS TMP and the
# functions die b act need_board env_get check_install setenv_script nb_status
# nb_install nb_uninstall nb_disarm nb_check_image nb_tftp_canary.

: "${NB_TAG:?}" "${NB_RECORDS:?}" "${NB_WATCHDOG:?}" "${NB_MMC:?}" "${YAKO_ADDR:?}" "${DRY:?}" "${REPO:?}"

die() { echo "$NB_TAG: $1" >&2; exit "${2:-3}"; }

# The U-Boot side, exactly as installed. fw_setenv stores ${...} literally;
# U-Boot's shell expands it when the variable runs.
#  - yako_hook: the one-shot first. When yako_once is set, U-Boot clears it and
#    saves the env BEFORE it loads anything, then runs yako_<yako_src> (net:
#    TFTP, mmc: the eMMC; the same "run bootcmd_${target}" form as the distro
#    boot). Then, whatever the one-shot did, the permanent default: when
#    yako_default names a file, yako_mmc loads and starts it. A load that fails
#    falls through to the bootcmd as it was (Linux).
#  - yako_dflt: the default. With yako_default_left unset it boots the default
#    every time (the product form). With a count it boots it only while the
#    count is above 0, saving the count less one BEFORE the load; at 0 it
#    deletes yako_default and the count and boots Linux. So an image that ends
#    in a watchdog reset returns the board to Linux by itself after that many
#    boots.
#  - yako_net / yako_mmc: load the image to yako_addr and "go" there, or print
#    why not and return.
YAKO_HOOK='if test -n "${yako_once}"; then echo yakogl-netboot: once=${yako_once}; setenv yako_once; if saveenv; then echo yakogl-netboot: cleared; run yako_${yako_src}; else echo yakogl-netboot: saveenv failed, booting Linux; fi; fi; if test -n "${yako_default}"; then run yako_dflt; fi'
YAKO_DFLT='if test -n "${yako_default_left}"; then if test ${yako_default_left} -gt 0; then setexpr yako_default_left ${yako_default_left} - 1; if saveenv; then echo yakogl-netboot: default ${yako_default}, boots left ${yako_default_left}; setenv yako_file ${yako_default}; run yako_mmc; else echo yakogl-netboot: saveenv failed, booting Linux; fi; else echo yakogl-netboot: default used up, booting Linux; setenv yako_default; setenv yako_default_left; saveenv; fi; else echo yakogl-netboot: default ${yako_default}; setenv yako_file ${yako_default}; run yako_mmc; fi'
YAKO_NET='setenv autoload no; setenv netretry no; setenv tftptimeout 1000; setenv tftptimeoutcountmax 5; setenv tftpblocksize 1468; setenv tftpwindowsize 8; if dhcp && tftpboot ${yako_addr} ${yako_server}:${yako_file}; then echo yakogl-netboot: go ${yako_addr}; dcache off; icache off; go ${yako_addr}; fi; echo yakogl-netboot: fetch failed, booting Linux'
YAKO_MMC="if ext4load mmc $NB_MMC \${yako_addr} \${yako_file}; then echo yakogl-netboot: go \${yako_addr}; dcache off; icache off; go \${yako_addr}; fi; echo yakogl-netboot: mmc load failed, booting Linux"
HOOK_PREFIX='run yako_hook; '

SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=10 -o BatchMode=yes)
# b <command>: run on the board, bounded; in --dry-run print it instead.
b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    timeout 60 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@"
}
# act <command...>: a host-side action with an effect; printed in --dry-run.
act() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ $*"; return 0; fi
    "$@"
}
need_board() {
    if [ "$DRY" = 1 ]; then BOARD_HOST=${BOARD_HOST:-user@BOARD}; return; fi
    : "${BOARD_HOST:?set BOARD_HOST=user@host of the board}"
}
# env_get NAME < fw_printenv output: the value of NAME, empty when absent.
env_get() { sed -n "s/^$1=//p"; }

# The installed state, read from the board: prints "ok", "absent",
# "stale: <reason>" (bootcmd is ours, around the saved bootcmd, but a hook
# string or the address is another version's: "install" upgrades it,
# "uninstall" still restores bootcmd), or another reason (refused).
check_install() {
    # --dry-run reads nothing: callers skip the verdict ("not read" is not "ok")
    if [ "$DRY" = 1 ]; then echo "not read (dry run)"; return; fi
    local env hook dflt net mmc addr bc prev
    env=$(b 'sudo -n fw_printenv' 2>/dev/null) || { echo "fw_printenv failed"; return; }
    hook=$(env_get yako_hook <<<"$env"); net=$(env_get yako_net <<<"$env")
    mmc=$(env_get yako_mmc <<<"$env"); dflt=$(env_get yako_dflt <<<"$env")
    addr=$(env_get yako_addr <<<"$env"); bc=$(env_get bootcmd <<<"$env")
    prev=$(env_get yako_bootcmd_prev <<<"$env")
    if [ -z "$hook$dflt$net$mmc$addr$prev" ] && [ "${bc#"$HOOK_PREFIX"}" = "$bc" ]; then echo absent; return; fi
    [ -n "$prev" ] || { echo "yako_bootcmd_prev is empty"; return; }
    [ "$bc" = "$HOOK_PREFIX$prev" ] || { echo "bootcmd is not '$HOOK_PREFIX' + yako_bootcmd_prev"; return; }
    [ "$hook" = "$YAKO_HOOK" ] || { echo "stale: yako_hook differs from this script's"; return; }
    [ "$dflt" = "$YAKO_DFLT" ] || { echo "stale: yako_dflt differs from this script's"; return; }
    [ "$net" = "$YAKO_NET" ] || { echo "stale: yako_net differs from this script's"; return; }
    [ "$mmc" = "$YAKO_MMC" ] || { echo "stale: yako_mmc differs from this script's"; return; }
    [ "$addr" = "$YAKO_ADDR" ] || { echo "stale: yako_addr is '$addr', not $YAKO_ADDR"; return; }
    echo ok
}
# setenv_script FILE: one fw_setenv call = one env write (all or nothing).
setenv_script() {
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ sudo -n fw_setenv -s /dev/stdin <<'EOF'"; sed 's/^/    /' "$1"; echo "    EOF"
        return 0
    fi
    b 'sudo -n fw_setenv -s /dev/stdin' < "$1"
}
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT

nb_status() {
    need_board
    b 'sudo -n fw_printenv bootcmd bootdelay yako_hook yako_dflt yako_net yako_mmc yako_addr yako_src yako_server yako_file yako_once yako_default yako_default_left yako_bootcmd_prev 2>&1'
    echo "install: $(check_install)"
}

# the hook's strings and the address, as install writes them
nb_hook_vars() {
    printf 'yako_addr=%s\n' "$YAKO_ADDR"
    printf 'yako_hook=%s\n' "$YAKO_HOOK"
    printf 'yako_dflt=%s\n' "$YAKO_DFLT"
    printf 'yako_net=%s\n' "$YAKO_NET"
    printf 'yako_mmc=%s\n' "$YAKO_MMC"
}

nb_install() {
    local st main bdir bak prev
    need_board
    st=$(check_install)
    if [ "$DRY" = 0 ]; then
        [ "$st" = ok ] && { echo "$NB_TAG: already installed"; exit 0; }
        if [ "${st#stale: }" != "$st" ]; then
            # bootcmd and the saved bootcmd stay as they are: only the strings change
            nb_hook_vars > "$TMP/upgrade"
            setenv_script "$TMP/upgrade" || die "fw_setenv failed; the env is as it was (one write)" 5
            prev=$(check_install)
            [ "$prev" = ok ] || die "read-back after the upgrade: $prev" 5
            echo "$NB_TAG: upgraded in place (was: ${st#stale: }); bootcmd unchanged"
            exit 0
        fi
        [ "$st" = absent ] || die "the board has a partial or foreign install ($st); inspect with 'status' and fix by hand"
    fi
    # The whole env is saved first: the bootcmd this replaces is the one
    # thing this script could not rebuild.
    main=$(cd "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)/.." && pwd)
    bdir=${NETBOOT_BACKUP_DIR:-$main/$NB_RECORDS}
    bak=$bdir/env-before-install-$(date +%Y%m%d-%H%M%S).txt
    if [ "$DRY" = 1 ]; then
        b 'sudo -n fw_printenv'; echo "DRY host\$ (the output goes to $bak)"
        prev='<the bootcmd read from the board>'
    else
        mkdir -p "$bdir"
        # The copy is unredacted (MACs, serial numbers): created mode 600
        # before the first byte goes in, never widened afterwards.
        install -m 600 /dev/null "$bak" || die "cannot create $bak"
        b 'sudo -n fw_printenv' > "$bak" || die "cannot read the env"
        prev=$(env_get bootcmd < "$bak")
        [ -n "$prev" ] || die "the board's env has no bootcmd; refusing to build on nothing"
        echo "$NB_TAG: env saved to $bak ($(wc -l < "$bak") lines)"
    fi
    {
        printf 'yako_bootcmd_prev=%s\n' "$prev"
        nb_hook_vars
        printf 'bootcmd=%s%s\n' "$HOOK_PREFIX" "$prev"
    } > "$TMP/install"
    setenv_script "$TMP/install" || die "fw_setenv failed; the env is as it was (one write)" 5
    [ "$DRY" = 1 ] && exit 0
    st=$(check_install)
    [ "$st" = ok ] || die "read-back after install: $st" 5
    echo "$NB_TAG: installed; bootcmd = '$HOOK_PREFIX' + the saved bootcmd"
}

nb_uninstall() {
    local st prev v
    need_board
    st=$(check_install)
    if [ "$DRY" = 0 ]; then
        [ "$st" = absent ] && { echo "$NB_TAG: not installed"; exit 0; }
        # a stale hook is still ours: bootcmd is the saved one behind our prefix
        [ "$st" = ok ] || [ "${st#stale: }" != "$st" ] ||
            die "the install is not this script's ($st); restore bootcmd by hand from the saved env"
        prev=$(b 'sudo -n fw_printenv -n yako_bootcmd_prev') || die "cannot read yako_bootcmd_prev"
    else
        prev='<yako_bootcmd_prev read from the board>'
    fi
    # libubootenv's script mode: "name=" with an empty value removes the variable.
    {
        printf 'bootcmd=%s\n' "$prev"
        for v in yako_bootcmd_prev yako_hook yako_dflt yako_net yako_mmc yako_addr yako_src yako_server yako_file yako_once yako_default yako_default_left; do
            printf '%s=\n' "$v"
        done
    } > "$TMP/uninstall"
    setenv_script "$TMP/uninstall" || die "fw_setenv failed; the env is as it was (one write)" 5
    [ "$DRY" = 1 ] && exit 0
    st=$(check_install)
    [ "$st" = absent ] || die "read-back after uninstall: $st" 5
    echo "$NB_TAG: uninstalled; bootcmd restored"
}

nb_disarm() {
    local v
    need_board
    b 'sudo -n fw_setenv yako_once' || die "fw_setenv failed" 5
    if [ "$DRY" = 0 ]; then
        v=$(b 'sudo -n fw_printenv -n yako_once 2>/dev/null')
        [ -z "$v" ] || die "yako_once is still '$v'" 5
    fi
    if [ "$DRY" = 1 ]; then echo "$NB_TAG: (dry run) nothing was written"; else echo "$NB_TAG: disarmed"; fi
}

# nb_check_image BIN END MARKER: refuse an image without the AArch64 Image
# header, one whose image_size runs past END from YAKO_ADDR, or one without
# MARKER (the watchdog-arming literal) unless NETBOOT_NO_WATCHDOG=1. Sets
# `footprint` and `sha` (the first 16 hex digits of the sha256).
nb_check_image() {
    local bin=$1 end=$2 marker=$3 magic max
    max=$(( end - YAKO_ADDR ))
    [ -f "$bin" ] || die "no image $bin"
    magic=$(od -A n -t x1 -j 56 -N 4 "$bin" | tr -d ' \n')
    [ "$magic" = 41524d64 ] || die "$bin has no AArch64 Image header ('$magic' at byte 56); build with CONFIG_AARCH64_IMAGE_HEADER=y"
    footprint=$(od -A n -t u8 -j 16 -N 8 "$bin" | tr -d ' ')
    { [ "$footprint" -gt 0 ] && [ "$footprint" -le "$max" ]; } || \
        die "image_size $footprint does not fit $YAKO_ADDR..$end ($max bytes)"
    if ! grep -aqF "$marker" "$bin"; then
        [ "${NETBOOT_NO_WATCHDOG:-0}" = 1 ] || \
            die "REFUSED: $bin does not contain \"$marker\": it would not arm $NB_WATCHDOG, and a hang would hold the board until the power is cut (NETBOOT_NO_WATCHDOG=1 overrides, deliberately)"
        echo "$NB_TAG: WARNING: NETBOOT_NO_WATCHDOG=1: this image has no way back if it hangs" >&2
    fi
    sha=$(sha256sum "$bin" | cut -c1-16)
}

# nb_tftp_canary SERVER ROOT TOKEN: the TFTP server serves ROOT on SERVER:69
# (a canary file fetched with curl, an independent client).
nb_tftp_canary() {
    local server=$1 root=$2 canary="yakogl/canary-$3" got
    act mkdir -p "$root/yakogl"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ echo canary > $root/$canary"
        echo "DRY host\$ curl -s --max-time 5 tftp://$server/$canary   # must print 'canary'"
        return 0
    fi
    echo canary > "$root/$canary"
    got=$(curl -s --max-time 5 "tftp://$server/$canary")
    rm -f "$root/$canary" "$root/$canary.served"
    [ "$got" = canary ] || die "no TFTP server serves $root on $server:69 (curl got '$got'); start tools/tftp_oneshot.py --root $root --bind $server"
}
